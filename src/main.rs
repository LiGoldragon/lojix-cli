use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use horizon_lib::name::{ClusterName, NodeName};
use ractor::Actor;

use lojix_cli_v2::Error;
use lojix_cli_v2::build::BuildAction;
use lojix_cli_v2::cluster::{FlakeRef, ProposalSource};
use lojix_cli_v2::deploy::{DeployCoordinator, DeployMsg, DeployOutcome, DeployRequest};

#[derive(Parser)]
#[command(
    name = "lojix-cli-v2",
    about = "Project a goldragon-style cluster proposal nota into a content-addressed horizon flake and deploy CriomOS via an explicit nix build → nix copy → switch-to-configuration pipeline."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build the toplevel derivation, then activate via switch-to-configuration on the target.
    Deploy(RunArgs),
    /// nix build the toplevel derivation; no activation.
    Build(RunArgs),
    /// nix eval the toplevel derivation's drvPath. Useful for cache testing.
    Eval(RunArgs),
}

#[derive(clap::Args)]
struct RunArgs {
    #[arg(long)]
    cluster: String,
    #[arg(long)]
    node: String,
    /// Path to the cluster proposal nota.
    #[arg(long)]
    source: PathBuf,
    /// Flake reference for CriomOS.
    #[arg(long, default_value = "github:LiGoldragon/CriomOS")]
    criomos: String,
    /// Override the action (Deploy defaults to Switch, Build to Build, Eval to Eval).
    #[arg(long, value_enum)]
    action: Option<ActionArg>,
    /// Sibling node to run `nix build` on instead of the dispatcher.
    /// Must be `is_builder && online` in the projected horizon. The
    /// closure is then `nix copy`'d to the target.
    #[arg(long)]
    builder: Option<String>,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum ActionArg {
    Eval,
    Build,
    Boot,
    Switch,
    Test,
    /// Install the new gen's bootloader entry but keep the
    /// persistent default on the *current* gen; set the new gen
    /// as a one-shot. Reboot 1 lands the new gen; subsequent
    /// reboots return to the old gen automatically. Designed
    /// for headless boxes where a permanent-default boot of an
    /// unverified gen is unsafe.
    BootOnce,
}

impl From<ActionArg> for BuildAction {
    fn from(a: ActionArg) -> Self {
        match a {
            ActionArg::Eval => BuildAction::Eval,
            ActionArg::Build => BuildAction::Build,
            ActionArg::Boot => BuildAction::Boot,
            ActionArg::Switch => BuildAction::Switch,
            ActionArg::Test => BuildAction::Test,
            ActionArg::BootOnce => BuildAction::BootOnce,
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let (default_action, args) = match cli.command {
        Command::Deploy(a) => (BuildAction::Switch, a),
        Command::Build(a) => (BuildAction::Build, a),
        Command::Eval(a) => (BuildAction::Eval, a),
    };
    let action = args.action.map(BuildAction::from).unwrap_or(default_action);

    let request = match build_request(args, action) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    let (coordinator, handle) =
        match Actor::spawn(Some("deploy".into()), DeployCoordinator, ()).await {
            Ok(x) => x,
            Err(e) => {
                eprintln!("error: spawn coordinator: {e}");
                return ExitCode::from(1);
            }
        };

    let outcome = coordinator
        .call(
            |reply| DeployMsg::Run { request, reply },
            Some(Duration::from_secs(3600)),
        )
        .await;

    coordinator.stop(None);
    let _ = handle.await;

    match outcome {
        Ok(ractor::rpc::CallResult::Success(Ok(DeployOutcome { stdout }))) => {
            print!("{stdout}");
            if !stdout.ends_with('\n') {
                println!();
            }
            ExitCode::SUCCESS
        }
        Ok(ractor::rpc::CallResult::Success(Err(e))) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
        Ok(other) => {
            eprintln!("error: rpc did not succeed: {other:?}");
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("error: rpc: {e}");
            ExitCode::from(1)
        }
    }
}

fn build_request(args: RunArgs, action: BuildAction) -> Result<DeployRequest, Error> {
    let cluster = ClusterName::try_new(args.cluster)?;
    let node = NodeName::try_new(args.node)?;
    let builder = match args.builder {
        Some(s) => Some(NodeName::try_new(s)?),
        None => None,
    };
    Ok(DeployRequest {
        cluster,
        node,
        builder,
        action,
        source: ProposalSource::new(args.source),
        criomos: FlakeRef::new(args.criomos),
    })
}
