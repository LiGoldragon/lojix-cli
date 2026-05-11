use std::process::ExitCode;
use std::time::Duration;

use ractor::Actor;

use lojix_cli::check::CheckHostKeyMaterial;
use lojix_cli::deploy::{DeployCoordinator, DeployMsg};
use lojix_cli::request::{CommandLine, LojixRequest};

#[tokio::main]
async fn main() -> ExitCode {
    let request = match CommandLine::from_env().decode_request() {
        Ok(request) => request,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(2);
        }
    };

    match request {
        LojixRequest::FullOs(_) | LojixRequest::OsOnly(_) | LojixRequest::HomeOnly(_) => {
            run_deploy(request).await
        }
        LojixRequest::CheckHostKeyMaterial(check) => run_check_host_key_material(check).await,
    }
}

async fn run_deploy(request: LojixRequest) -> ExitCode {
    let deploy_request = match request {
        LojixRequest::FullOs(request) => request.into_deploy_request(),
        LojixRequest::OsOnly(request) => request.into_deploy_request(),
        LojixRequest::HomeOnly(request) => request.into_deploy_request(),
        LojixRequest::CheckHostKeyMaterial(_) => {
            // Unreachable — branched out in main.
            eprintln!("error: invariant: CheckHostKeyMaterial does not deploy");
            return ExitCode::from(1);
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
            |reply| DeployMsg::Run {
                request: deploy_request,
                reply,
            },
            Some(Duration::from_secs(3600)),
        )
        .await;

    coordinator.stop(None);
    let _ = handle.await;

    match outcome {
        Ok(ractor::rpc::CallResult::Success(Ok(outcome))) => {
            let stdout = outcome.stdout_text();
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

async fn run_check_host_key_material(check: CheckHostKeyMaterial) -> ExitCode {
    match check.run().await {
        Ok(report) => {
            print!("{}", report.render_text());
            // Exit non-zero when there are mismatches — gives the
            // operator a CI-friendly signal (`if lojix ... ; then`).
            if report.is_consistent() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(3)
            }
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}
