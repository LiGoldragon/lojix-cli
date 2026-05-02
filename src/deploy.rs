use std::collections::BTreeMap;

use horizon_lib::Viewpoint;
use horizon_lib::name::{ClusterName, NodeName, UserName};
use horizon_lib::user::User;
use ractor::rpc::CallResult;
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};

use crate::activate::{ActivateMsg, Activation, Activator, HomeActivation, SystemActivation};
use crate::artifact::{
    ArtifactMaterialization, ArtifactMaterializationInput, ArtifactMsg, HorizonArtifact,
    MaterializedArtifact,
};
use crate::build::{BuildMsg, BuildPhaseOutcome, BuildPlan, HomeMode, NixBuild, NixBuilder};
use crate::cluster::{DerivationPath, FlakeRef, OverrideUri, ProposalSource, StorePath};
use crate::copy::{ClosureCopier, ClosureCopy, CopyMsg};
use crate::error::{Error, Result};
use crate::host::{RemoteRsync, RemoteStaging, SshTarget};
use crate::project::{HorizonProjection, HorizonProjectionInput, HorizonProjector, ProjectMsg};
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
    pub plan: BuildPlan,
    pub source: ProposalSource,
    pub criomos: FlakeRef,
}

impl DeployRequest {
    fn validate_home_user(&self, users: &BTreeMap<UserName, User>) -> Result<()> {
        if let Some(user) = self.plan.home_user()
            && !users.contains_key(user)
        {
            return Err(Error::UnknownHomeUser {
                user: user.clone(),
                cluster: self.cluster.clone(),
                node: self.node.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum DeployOutcome {
    Evaluated { derivation_path: DerivationPath },
    Realized { store_path: StorePath },
}

impl DeployOutcome {
    pub fn stdout_text(&self) -> String {
        match self {
            Self::Evaluated { derivation_path } => derivation_path.as_str().to_string(),
            Self::Realized { store_path } => store_path.as_str().to_string(),
        }
    }
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
        let proposal = ActorCallResult::from_result(
            self.proposal_reader
                .call(
                    |reply| ProposalMsg::Read {
                        source: request.source.clone(),
                        reply,
                    },
                    None,
                )
                .await,
        )
        .into_payload()??;

        let viewpoint = Viewpoint {
            cluster: request.cluster.clone(),
            node: request.node.clone(),
        };
        let horizon = ActorCallResult::from_result(
            self.projector
                .call(
                    |reply| ProjectMsg::Project {
                        projection: HorizonProjection::from_input(HorizonProjectionInput {
                            proposal,
                            viewpoint,
                        }),
                        reply,
                    },
                    None,
                )
                .await,
        )
        .into_payload()??;

        request.validate_home_user(&horizon.users)?;

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
                if !request.plan.supports_remote_builder() {
                    return Err(Error::UnsupportedHomeBuilder(name.clone()));
                }
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

        let materialized = ActorCallResult::from_result(
            self.artifact
                .call(
                    |reply| ArtifactMsg::Materialize {
                        materialization: ArtifactMaterialization::from_input(
                            ArtifactMaterializationInput {
                                horizon,
                                cluster: request.cluster.clone(),
                                node: request.node.clone(),
                                deployment_shape: request.plan.deployment_shape(),
                            },
                        ),
                        reply,
                    },
                    None,
                )
                .await,
        )
        .into_payload()??;

        // Stage override-input dirs onto the builder if we're
        // building remote. The URIs we hand to `nix build` then
        // resolve on the builder's filesystem. When no builder is
        // set the dispatcher's local cache paths are used as-is.
        let staged_inputs = self
            .stage_inputs(StageInputsRequest {
                materialized: &materialized,
                builder: builder_target.as_ref(),
            })
            .await?;

        let outcome = ActorCallResult::from_result(
            self.builder
                .call(
                    |reply| BuildMsg::Run {
                        build: NixBuild {
                            flake: request.criomos.clone(),
                            horizon_uri: staged_inputs.horizon_uri.clone(),
                            system_uri: staged_inputs.system_uri.clone(),
                            deployment_uri: staged_inputs.deployment_uri.clone(),
                            plan: request.plan.clone(),
                            builder: builder_target.clone(),
                        },
                        reply,
                    },
                    None,
                )
                .await,
        )
        .into_payload()??;

        let result = self
            .finish(FinishRequest {
                plan: request.plan,
                node: request.node.clone(),
                target,
                outcome,
            })
            .await;
        if let Some(staging) = staged_inputs.staging {
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

    async fn stage_inputs(&self, request: StageInputsRequest<'_>) -> Result<StagedInputs> {
        match request.builder {
            None => Ok(StagedInputs {
                horizon_uri: request.materialized.horizon_uri.clone(),
                system_uri: request.materialized.system_uri.clone(),
                deployment_uri: request.materialized.deployment_uri.clone(),
                staging: None,
            }),
            Some(target) => {
                let staging = RemoteStaging::try_create(target.clone()).await?;
                let horizon_uri = staging
                    .rsync(RemoteRsync {
                        local_dir: request.materialized.horizon_dir.path(),
                        name: "horizon",
                    })
                    .await?;
                let system_uri = staging
                    .rsync(RemoteRsync {
                        local_dir: request.materialized.system_dir.path(),
                        name: "system",
                    })
                    .await?;
                let deployment_uri = staging
                    .rsync(RemoteRsync {
                        local_dir: request.materialized.deployment_dir.path(),
                        name: "deployment",
                    })
                    .await?;
                Ok(StagedInputs {
                    horizon_uri,
                    system_uri,
                    deployment_uri,
                    staging: Some(staging),
                })
            }
        }
    }

    async fn finish(&self, request: FinishRequest) -> Result<DeployOutcome> {
        match request.outcome {
            BuildPhaseOutcome::EvalDone { derivation_path } => {
                Ok(DeployOutcome::Evaluated { derivation_path })
            }
            BuildPhaseOutcome::BuildDone {
                store_path,
                location,
            } => {
                match request.plan {
                    BuildPlan::System { action, .. } if action.activates() => {
                        ActorCallResult::from_result(
                            self.copier
                                .call(
                                    |reply| CopyMsg::Run {
                                        copy: ClosureCopy {
                                            store_path: store_path.clone(),
                                            source: location,
                                            target: request.target.clone(),
                                        },
                                        reply,
                                    },
                                    None,
                                )
                                .await,
                        )
                        .into_payload()??;
                        ActorCallResult::from_result(
                            self.activator
                                .call(
                                    |reply| ActivateMsg::Run {
                                        activation: Activation::System(SystemActivation {
                                            target: request.target,
                                            store_path: store_path.clone(),
                                            action,
                                        }),
                                        reply,
                                    },
                                    None,
                                )
                                .await,
                        )
                        .into_payload()??;
                    }
                    BuildPlan::System { .. } => {}
                    BuildPlan::Home {
                        mode: HomeMode::Build,
                        ..
                    } => {}
                    BuildPlan::Home { user, mode } => {
                        ActorCallResult::from_result(
                            self.activator
                                .call(
                                    |reply| ActivateMsg::Run {
                                        activation: Activation::Home(HomeActivation {
                                            node: request.node,
                                            user,
                                            store_path: store_path.clone(),
                                            mode,
                                        }),
                                        reply,
                                    },
                                    None,
                                )
                                .await,
                        )
                        .into_payload()??;
                    }
                }
                Ok(DeployOutcome::Realized { store_path })
            }
        }
    }
}

struct StageInputsRequest<'request> {
    materialized: &'request MaterializedArtifact,
    builder: Option<&'request SshTarget>,
}

struct StagedInputs {
    horizon_uri: OverrideUri,
    system_uri: OverrideUri,
    deployment_uri: OverrideUri,
    staging: Option<RemoteStaging>,
}

struct FinishRequest {
    plan: BuildPlan,
    node: NodeName,
    target: SshTarget,
    outcome: BuildPhaseOutcome,
}

struct ActorCallResult<T, E> {
    result: std::result::Result<CallResult<T>, ractor::MessagingErr<E>>,
}

impl<T, E> ActorCallResult<T, E>
where
    ractor::MessagingErr<E>: std::fmt::Display,
{
    fn from_result(result: std::result::Result<CallResult<T>, ractor::MessagingErr<E>>) -> Self {
        Self { result }
    }

    fn into_payload(self) -> Result<T> {
        match self.result {
            Ok(CallResult::Success(payload)) => Ok(payload),
            Ok(CallResult::Timeout) => Err(Error::ActorRpcFailed { reason: "timeout" }),
            Ok(CallResult::SenderError) => Err(Error::ActorRpcFailed {
                reason: "sender error",
            }),
            Err(error) => Err(Error::ActorMessagingFailed {
                message: error.to_string(),
            }),
        }
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
