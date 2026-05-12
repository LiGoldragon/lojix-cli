use std::process::ExitCode;

use lojix_cli::check::CheckHostKeyMaterial;
use lojix_cli::deploy::deploy;
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

    match deploy(deploy_request).await {
        Ok(outcome) => {
            let stdout = outcome.stdout_text();
            print!("{stdout}");
            if !stdout.ends_with('\n') {
                println!();
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
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
