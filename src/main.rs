use std::process::ExitCode;
use std::time::Duration;

use ractor::Actor;

use lojix_cli::deploy::{DeployCoordinator, DeployMsg, DeployOutcome};
use lojix_cli::request::CommandLine;

#[tokio::main]
async fn main() -> ExitCode {
    let request = match CommandLine::from_env().decode_request() {
        Ok(request) => request.into_deploy_request(),
        Err(error) => {
            eprintln!("error: {error}");
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
