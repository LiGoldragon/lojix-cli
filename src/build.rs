use std::process::Stdio;

use process_wrap::tokio::*;
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::cluster::{FlakeRef, OverrideUri, StorePath};
use crate::error::{Error, Result};
use crate::host::SshTarget;

#[derive(Copy, Clone, Debug)]
pub enum BuildAction {
    Eval,
    Build,
    Boot,
    Switch,
    Test,
    /// Install the new generation's bootloader entry, but keep the
    /// persistent default pointing at the *current* gen and set
    /// the new gen as a one-shot. Reboot 1 lands the new gen;
    /// reboot 2 (and every subsequent boot) returns to the old
    /// gen automatically. Designed for headless boxes where a
    /// permanent-default boot of an unverified gen is unsafe.
    BootOnce,
}

impl BuildAction {
    pub fn produces_closure(self) -> bool {
        !matches!(self, BuildAction::Eval)
    }

    pub fn activates(self) -> bool {
        matches!(
            self,
            BuildAction::Boot
                | BuildAction::Switch
                | BuildAction::Test
                | BuildAction::BootOnce,
        )
    }
}

/// Where the build phase's closure landed. Drives the copy phase:
/// `Dispatcher` → `nix copy --to`; `Builder(t)` → `nix copy --from
/// <t> --to <target>` (or skip when builder == target).
#[derive(Debug, Clone)]
pub enum BuildLocation {
    Dispatcher,
    Builder(SshTarget),
}

#[derive(Debug)]
pub enum BuildPhaseOutcome {
    /// `Eval` action — drvPath only, no closure.
    EvalDone { drv_path: String },
    /// `Build`/`Boot`/`Switch`/`Test` — closure realised somewhere.
    BuildDone {
        store_path: StorePath,
        location: BuildLocation,
    },
}

pub struct NixBuild {
    pub flake: FlakeRef,
    pub horizon_uri: OverrideUri,
    pub system_uri: OverrideUri,
    pub action: BuildAction,
    pub builder: Option<SshTarget>,
}

impl NixBuild {
    /// (program, argv) for the nix invocation. Pure — the same
    /// values are run locally or wrapped into an ssh argv when a
    /// `builder` is set. Exposed so tests can assert wire shape
    /// without spawning nix.
    pub fn nix_argv(&self) -> (&'static str, Vec<String>) {
        let target_attr = format!(
            "{}#nixosConfigurations.target.config.system.build.toplevel",
            self.flake.as_str(),
        );
        let mut argv = match self.action {
            BuildAction::Eval => vec![
                "eval".to_string(),
                "--raw".to_string(),
                format!("{target_attr}.drvPath"),
            ],
            BuildAction::Build
            | BuildAction::Boot
            | BuildAction::Switch
            | BuildAction::Test
            | BuildAction::BootOnce => vec![
                "build".to_string(),
                "--no-link".to_string(),
                "--print-out-paths".to_string(),
                target_attr,
            ],
        };
        argv.push("--override-input".to_string());
        argv.push("horizon".to_string());
        argv.push(self.horizon_uri.as_str().to_string());
        argv.push("--override-input".to_string());
        argv.push("system".to_string());
        argv.push(self.system_uri.as_str().to_string());
        ("nix", argv)
    }

    pub async fn run(&self) -> Result<BuildPhaseOutcome> {
        let (program, argv) = self.nix_argv();
        // stderr inherits the dispatcher's terminal so nix's
        // progress (and ssh diagnostics, when running remote)
        // stream live. stdout is piped — drvPath / store path is
        // returned to the caller. ProcessGroup + KillOnDrop reap
        // the whole nix child tree (and any ssh tunnel) on
        // Ctrl-C / future-drop.
        let stdout = match &self.builder {
            None => run_local(program, &argv).await?,
            Some(target) => run_remote(target, program, &argv).await?,
        };

        match self.action {
            BuildAction::Eval => Ok(BuildPhaseOutcome::EvalDone {
                drv_path: stdout.trim().to_string(),
            }),
            _ => {
                let store_path = StorePath::try_new(stdout)?;
                let location = match &self.builder {
                    None => BuildLocation::Dispatcher,
                    Some(target) => BuildLocation::Builder(target.clone()),
                };
                Ok(BuildPhaseOutcome::BuildDone { store_path, location })
            }
        }
    }
}

async fn run_local(program: &str, argv: &[String]) -> Result<String> {
    let mut wrap = CommandWrap::with_new(program, |c: &mut Command| {
        c.args(argv)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
    });
    wrap.wrap(ProcessGroup::leader());
    wrap.wrap(KillOnDrop);
    let mut child = wrap.spawn()?;
    let mut stdout = String::new();
    if let Some(mut s) = child.stdout().take() {
        s.read_to_string(&mut stdout).await?;
    }
    let status = child.wait().await?;
    if !status.success() {
        return Err(Error::NixFailed {
            status: status.code().unwrap_or(-1),
            stderr: "(streamed to terminal — see above)".to_string(),
        });
    }
    Ok(stdout)
}

async fn run_remote(target: &SshTarget, program: &str, argv: &[String]) -> Result<String> {
    // OpenSSH joins the trailing argv with single spaces into one
    // command string the remote $SHELL re-parses. Quote each token
    // up-front so a value containing whitespace or a shell metachar
    // survives the round-trip intact.
    let mut remote_command = shell_quote(program);
    for arg in argv {
        remote_command.push(' ');
        remote_command.push_str(&shell_quote(arg));
    }
    run_local(
        "ssh",
        &[
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            target.as_ssh_arg().to_string(),
            remote_command,
        ],
    )
    .await
}

/// Single-quote `s` for safe inclusion in a POSIX shell command
/// line. Returns the input unchanged when every byte is from a
/// known-safe alphabet (saves visual noise on flag-like args).
fn shell_quote(s: &str) -> String {
    let safe = !s.is_empty()
        && s.bytes().all(|b| {
            matches!(b,
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9'
                | b'-' | b'_' | b'.' | b'/' | b'=' | b':' | b'#' | b'+' | b','
            )
        });
    if safe {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub struct NixBuilder;

pub enum BuildMsg {
    Run {
        build: NixBuild,
        reply: RpcReplyPort<Result<BuildPhaseOutcome>>,
    },
}

#[ractor::async_trait]
impl Actor for NixBuilder {
    type Msg = BuildMsg;
    type State = ();
    type Arguments = ();

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        _args: (),
    ) -> std::result::Result<Self::State, ActorProcessingErr> {
        Ok(())
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        msg: Self::Msg,
        _state: &mut Self::State,
    ) -> std::result::Result<(), ActorProcessingErr> {
        match msg {
            BuildMsg::Run { build, reply } => {
                let _ = reply.send(build.run().await);
            }
        }
        Ok(())
    }
}
