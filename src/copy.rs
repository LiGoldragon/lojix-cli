use std::process::Stdio;

use process_wrap::tokio::*;
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};
use tokio::process::Command;

use crate::build::BuildLocation;
use crate::cluster::StorePath;
use crate::error::{Error, Result};
use crate::host::SshTarget;

/// Move a closure from wherever the build phase landed it to the
/// activation target.
///
/// Three cases:
/// - source is `Dispatcher`: `nix copy --to ssh-ng://<target>`.
/// - source is `Builder(b)` and `b == target`: skip — the closure
///   is already where it needs to be.
/// - source is `Builder(b)` and `b != target`: `nix copy --from
///   ssh-ng://<b> --to ssh-ng://<target>`. The dispatcher's
///   nix-daemon orchestrates the NAR transfer through both ssh
///   tunnels.
pub struct ClosureCopy {
    pub store_path: StorePath,
    pub source: BuildLocation,
    pub target: SshTarget,
}

impl ClosureCopy {
    /// `Some((program, argv))` if a copy is needed, `None` if the
    /// closure already lives on the target. Pure.
    pub fn argv(&self) -> Option<(&'static str, Vec<String>)> {
        if self.source_matches_target() {
            return None;
        }
        let mut argv: Vec<String> = vec!["copy".to_string()];
        if let BuildLocation::Builder(builder) = &self.source {
            argv.push("--from".to_string());
            argv.push(builder.ssh_uri());
        }
        argv.push("--to".to_string());
        argv.push(self.target.ssh_uri());
        argv.push(self.store_path.as_str().to_string());
        Some(("nix", argv))
    }

    pub async fn run(&self) -> Result<()> {
        let Some((program, argv)) = self.argv() else { return Ok(()) };
        let mut wrap = CommandWrap::with_new(program, |c: &mut Command| {
            c.args(&argv)
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
        });
        wrap.wrap(ProcessGroup::leader());
        wrap.wrap(KillOnDrop);
        let mut child = wrap.spawn()?;
        let status = child.wait().await?;
        if !status.success() {
            return Err(Error::NixFailed {
                status: status.code().unwrap_or(-1),
                stderr: "(nix copy — see streamed output)".to_string(),
            });
        }
        Ok(())
    }

    fn source_matches_target(&self) -> bool {
        match &self.source {
            BuildLocation::Dispatcher => false,
            BuildLocation::Builder(b) => b.as_ssh_arg() == self.target.as_ssh_arg(),
        }
    }
}

pub struct ClosureCopier;

pub enum CopyMsg {
    Run {
        copy: ClosureCopy,
        reply: RpcReplyPort<Result<()>>,
    },
}

#[ractor::async_trait]
impl Actor for ClosureCopier {
    type Msg = CopyMsg;
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
            CopyMsg::Run { copy, reply } => {
                let _ = reply.send(copy.run().await);
            }
        }
        Ok(())
    }
}
