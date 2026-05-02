use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};

use crate::build::BuildLocation;
use crate::cluster::StorePath;
use crate::error::Result;
use crate::host::SshTarget;
use crate::process::{ProcessFailure, ProcessInvocation, ProcessRun};

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
    /// `Some(invocation)` if a copy is needed, `None` if the
    /// closure already lives on the target. Pure.
    pub fn invocation(&self) -> Option<ProcessInvocation> {
        if self.source_matches_target() {
            return None;
        }
        let mut arguments: Vec<String> = vec!["copy".to_string()];
        if let BuildLocation::Builder(builder) = &self.source {
            arguments.push("--from".to_string());
            arguments.push(builder.ssh_uri());
        }
        arguments.push("--to".to_string());
        arguments.push(self.target.ssh_uri());
        arguments.push(self.store_path.as_str().to_string());
        Some(ProcessInvocation::new("nix").with_arguments(arguments))
    }

    pub async fn run(&self) -> Result<()> {
        let Some(invocation) = self.invocation() else {
            return Ok(());
        };
        invocation
            .inherit_stdio(ProcessRun::inherit_stderr(ProcessFailure::Nix))
            .await
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
