use horizon_lib::name::{ClusterName, NodeName};
use horizon_lib::Viewpoint;
use ractor::rpc::CallResult;
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};

use crate::activate::{ActivateMsg, Activator, SystemActivation};
use crate::artifact::{ArtifactMsg, HorizonArtifact, MaterializedArtifact};
use crate::build::{BuildAction, BuildMsg, BuildPhaseOutcome, NixBuild, NixBuilder};
use crate::cluster::{FlakeRef, OverrideUri, ProposalSource};
use crate::copy::{ClosureCopier, ClosureCopy, CopyMsg};
use crate::error::{Error, Result};
use crate::host::{RemoteStaging, SshTarget};
use crate::project::{HorizonProjector, ProjectMsg};
use crate::proposal::{ProposalMsg, ProposalReader};

// No RPC timeout — `nix build` of a NixOS system from cold cache can
// take hours (substituter fetch + tons of derivations). The right fix
// for live progress is the streaming-output redesign tracked as
// `lojix-auy`; for now an unbounded RPC just lets nix-daemon finish
// in its own time. None below means no deadline on these calls.

pub struct DeployRequest {
    pub cluster: ClusterName,
    pub node: NodeName,
    pub builder: Option<NodeName>,
    pub action: BuildAction,
    pub source: ProposalSource,
    pub criomos: FlakeRef,
}

#[derive(Debug)]
pub struct DeployOutcome {
    /// `Eval` returns a drvPath; everything else returns the
    /// realised `/nix/store/...` toplevel.
    pub stdout: String,
}

pub struct DeployState {
    proposal_reader: ActorRef<ProposalMsg>,
    projector: ActorRef<ProjectMsg>,
    artifact: ActorRef<ArtifactMsg>,
    builder: ActorRef<BuildMsg>,
    copier: ActorRef<CopyMsg>,
    activator: ActorRef<ActivateMsg>,
}

impl DeployState {
    async fn run(&self, request: DeployRequest) -> Result<DeployOutcome> {
        let proposal = unwrap_call(
            self.proposal_reader
                .call(
                    |reply| ProposalMsg::Read {
                        source: request.source,
                        reply,
                    },
                    None,
                )
                .await,
        )??;

        let viewpoint = Viewpoint {
            cluster: request.cluster.clone(),
            node: request.node.clone(),
        };
        let horizon = unwrap_call(
            self.projector
                .call(
                    |reply| ProjectMsg::Project {
                        proposal,
                        viewpoint,
                        reply,
                    },
                    None,
                )
                .await,
        )??;

        // The viewpoint node — the deploy target — is always
        // `horizon.node`; addressing comes from its
        // `criome_domain_name`. Per the network-neutrality rule
        // there is no `--target` flag.
        let target = SshTarget::from_node(&horizon.node);

        // Resolve the requested builder (if supplied) against the projected
        // ex-nodes. Validates `is_builder && online` *before* any
        // nix invocation so an offline / non-builder name fails
        // with a clear error rather than a TCP timeout deep into
        // the build.
        let builder_target = match &request.builder {
            None => None,
            Some(name) => {
                // The builder resolves against the full horizon —
                // including the viewpoint node, so builder
                // prometheus with node prometheus is the legitimate
                // "build on the target" case (offload from a thin
                // dispatcher to a beefy target). The viewpoint
                // sits in `horizon.node`; ex-nodes in
                // `horizon.ex_nodes`.
                let node = if *name == request.node {
                    &horizon.node
                } else {
                    horizon
                        .ex_nodes
                        .get(name)
                        .ok_or_else(|| Error::UnknownBuilder(name.clone()))?
                };
                // `is_builder` projection gates on `online &&
                // is_fully_trusted && size>=med && has base
                // pubkeys`, so a single check covers all the
                // disqualifications.
                if !node.is_builder {
                    return Err(Error::InvalidBuilder(name.clone()));
                }
                Some(SshTarget::from_node(node))
            }
        };

        let materialized = unwrap_call(
            self.artifact
                .call(
                    |reply| ArtifactMsg::Materialize {
                        horizon,
                        cluster: request.cluster.clone(),
                        node: request.node.clone(),
                        reply,
                    },
                    None,
                )
                .await,
        )??;

        // Stage override-input dirs onto the builder if we're
        // building remote. The URIs we hand to `nix build` then
        // resolve on the builder's filesystem. When no builder is
        // set the dispatcher's local cache paths are used as-is.
        let (horizon_uri, system_uri, staging) = self
            .stage_inputs(&materialized, builder_target.as_ref())
            .await?;

        let outcome = unwrap_call(
            self.builder
                .call(
                    |reply| BuildMsg::Run {
                        build: NixBuild {
                            flake: request.criomos.clone(),
                            horizon_uri,
                            system_uri,
                            action: request.action,
                            builder: builder_target.clone(),
                        },
                        reply,
                    },
                    None,
                )
                .await,
        )??;

        let result = self.finish(request.action, target, outcome).await;
        if let Some(staging) = staging {
            // Best-effort cleanup. A leftover /tmp/lojix-stage.*
            // dir doesn't break anything but we surface failures
            // rather than swallow them silently — only by
            // overwriting `result` if it was Ok and cleanup
            // failed; if `result` was already Err, that takes
            // priority.
            match staging.cleanup().await {
                Ok(()) => {}
                Err(error) if result.is_ok() => return Err(error),
                Err(error) => {
                    eprintln!("warning: staging cleanup failed: {error}");
                }
            }
        }
        result
    }

    async fn stage_inputs(
        &self,
        materialized: &MaterializedArtifact,
        builder: Option<&SshTarget>,
    ) -> Result<(OverrideUri, OverrideUri, Option<RemoteStaging>)> {
        match builder {
            None => Ok((
                materialized.horizon_uri.clone(),
                materialized.system_uri.clone(),
                None,
            )),
            Some(target) => {
                let staging = RemoteStaging::try_create(target.clone()).await?;
                let horizon_uri = staging
                    .rsync(materialized.horizon_dir.path(), "horizon")
                    .await?;
                let system_uri = staging
                    .rsync(materialized.system_dir.path(), "system")
                    .await?;
                Ok((horizon_uri, system_uri, Some(staging)))
            }
        }
    }

    async fn finish(
        &self,
        action: BuildAction,
        target: SshTarget,
        outcome: BuildPhaseOutcome,
    ) -> Result<DeployOutcome> {
        match outcome {
            BuildPhaseOutcome::EvalDone { drv_path } => Ok(DeployOutcome { stdout: drv_path }),
            BuildPhaseOutcome::BuildDone {
                store_path,
                location,
            } => {
                if !action.activates() {
                    return Ok(DeployOutcome {
                        stdout: store_path.as_str().to_string(),
                    });
                }
                unwrap_call(
                    self.copier
                        .call(
                            |reply| CopyMsg::Run {
                                copy: ClosureCopy {
                                    store_path: store_path.clone(),
                                    source: location,
                                    target: target.clone(),
                                },
                                reply,
                            },
                            None,
                        )
                        .await,
                )??;
                unwrap_call(
                    self.activator
                        .call(
                            |reply| ActivateMsg::Run {
                                activation: SystemActivation {
                                    target,
                                    store_path: store_path.clone(),
                                    action,
                                },
                                reply,
                            },
                            None,
                        )
                        .await,
                )??;
                Ok(DeployOutcome {
                    stdout: store_path.as_str().to_string(),
                })
            }
        }
    }
}

fn unwrap_call<T, E>(r: std::result::Result<CallResult<T>, ractor::MessagingErr<E>>) -> Result<T>
where
    ractor::MessagingErr<E>: std::fmt::Display,
{
    match r {
        Ok(CallResult::Success(t)) => Ok(t),
        Ok(CallResult::Timeout) => Err(Error::Ractor("rpc timeout".into())),
        Ok(CallResult::SenderError) => Err(Error::Ractor("rpc sender error".into())),
        Err(e) => Err(Error::Ractor(e.to_string())),
    }
}

pub struct DeployCoordinator;

pub enum DeployMsg {
    Run {
        request: DeployRequest,
        reply: RpcReplyPort<Result<DeployOutcome>>,
    },
}

#[ractor::async_trait]
impl Actor for DeployCoordinator {
    type Msg = DeployMsg;
    type State = DeployState;
    type Arguments = ();

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        _args: (),
    ) -> std::result::Result<Self::State, ActorProcessingErr> {
        let (proposal_reader, _) = Actor::spawn_linked(
            Some("proposal".into()),
            ProposalReader,
            (),
            myself.get_cell(),
        )
        .await?;
        let (projector, _) = Actor::spawn_linked(
            Some("project".into()),
            HorizonProjector,
            (),
            myself.get_cell(),
        )
        .await?;
        let (artifact, _) = Actor::spawn_linked(
            Some("artifact".into()),
            HorizonArtifact,
            (),
            myself.get_cell(),
        )
        .await?;
        let (builder, _) =
            Actor::spawn_linked(Some("build".into()), NixBuilder, (), myself.get_cell()).await?;
        let (copier, _) =
            Actor::spawn_linked(Some("copy".into()), ClosureCopier, (), myself.get_cell()).await?;
        let (activator, _) =
            Actor::spawn_linked(Some("activate".into()), Activator, (), myself.get_cell()).await?;
        Ok(DeployState {
            proposal_reader,
            projector,
            artifact,
            builder,
            copier,
            activator,
        })
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        msg: Self::Msg,
        state: &mut Self::State,
    ) -> std::result::Result<(), ActorProcessingErr> {
        match msg {
            DeployMsg::Run { request, reply } => {
                let result = state.run(request).await;
                let _ = reply.send(result);
            }
        }
        Ok(())
    }
}
