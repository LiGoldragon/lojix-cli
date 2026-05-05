use std::collections::BTreeMap;

use horizon_lib::name::{ClusterName, NodeName, UserName};
use horizon_lib::node::Node;
use horizon_lib::user::User;
use horizon_lib::{Horizon, Viewpoint};
use ractor::rpc::CallResult;
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};

use crate::activate::{ActivateMsg, Activation, Activator, HomeActivation, SystemActivation};
use crate::artifact::{
    ArtifactMaterialization, ArtifactMaterializationInput, ArtifactMsg, HorizonArtifact,
};
use crate::build::{
    BuildMsg, BuildPhaseOutcome, BuildPlan, ExtraSubstituter, ExtraSubstituters, HomeMode,
    NixBuild, NixBuilder,
};
use crate::cluster::{DerivationPath, FlakeRef, ProposalSource, StorePath};
use crate::copy::{ClosureCopier, ClosureCopy, CopyMsg};
use crate::error::{Error, Result};
use crate::host::SshTarget;
use crate::project::{HorizonProjection, HorizonProjectionInput, HorizonProjector, ProjectMsg};
use crate::proposal::{ProposalMsg, ProposalReader};
use crate::stage::{BuildInputReferences, RemoteInputStage};

// No RPC timeout — `nix build` of a NixOS system from cold cache can
// take hours (substituter fetch + tons of derivations). The right fix
// for live progress is the streaming-output redesign tracked as
// `lojix-auy`; for now an unbounded RPC just lets nix-daemon finish
// in its own time. None below means no deadline on these calls.

pub struct DeployRequest {
    pub cluster: ClusterName,
    pub node: NodeName,
    pub builder: Option<NodeName>,
    pub substituters: Vec<NodeName>,
    pub plan: BuildPlan,
    pub source: ProposalSource,
    pub flake: FlakeRef,
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
        let extra_substituters =
            ExtraSubstituters::from_horizon_nodes(&horizon, &request.substituters)?;

        // The viewpoint node — the deploy target — is always
        // `horizon.node`; addressing comes from its
        // `criome_domain_name`. Per the network-neutrality rule
        // there is no `--target` flag.
        let target = SshTarget::from_node(&horizon.node);

        // Resolve the requested build host (if supplied) against the projected
        // horizon. The viewpoint node is a local build on the deployment target,
        // so it does not need to expose Nix's remote-builder service. A sibling
        // build host must be a remote Nix builder because the dispatcher connects
        // to it for the build phase.
        let builder_target = match &request.builder {
            None => None,
            Some(name) => {
                if *name == request.node {
                    Some(SshTarget::from_node(&horizon.node))
                } else {
                    let node = horizon
                        .ex_nodes
                        .get(name)
                        .ok_or_else(|| Error::UnknownBuilder(name.clone()))?;
                    if !node.is_remote_nix_builder {
                        return Err(Error::InvalidBuilder(name.clone()));
                    }
                    Some(SshTarget::from_node(node))
                }
            }
        };

        let target_system = horizon.node.system;
        let materialized = ActorCallResult::from_result(
            self.artifact
                .call(
                    |reply| ArtifactMsg::Materialize {
                        materialization: ArtifactMaterialization::from_input(
                            ArtifactMaterializationInput {
                                horizon,
                                cluster: request.cluster.clone(),
                                node: request.node.clone(),
                                deployment_shape: match request.plan {
                                    BuildPlan::System { .. } => {
                                        Some(request.plan.deployment_shape())
                                    }
                                    BuildPlan::Home { .. } => None,
                                },
                            },
                        ),
                        reply,
                    },
                    None,
                )
                .await,
        )
        .into_payload()??;

        let build_inputs = match &builder_target {
            None => BuildInputReferences::from_local_artifact(&materialized),
            Some(builder) => {
                RemoteInputStage::from_artifact(builder.clone(), &materialized)
                    .run()
                    .await?
            }
        };

        let build_flake = match &request.plan {
            BuildPlan::System { .. } => request.flake.clone(),
            BuildPlan::Home { .. } => request.flake.clone(),
        };

        let outcome = ActorCallResult::from_result(
            self.builder
                .call(
                    |reply| BuildMsg::Run {
                        build: NixBuild {
                            flake: build_flake,
                            system: target_system,
                            horizon_ref: build_inputs.horizon_ref,
                            system_ref: build_inputs.system_ref,
                            deployment_ref: build_inputs.deployment_ref,
                            extra_substituters,
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

        self.finish(FinishRequest {
            plan: request.plan,
            node: request.node.clone(),
            target,
            outcome,
        })
        .await
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
                                        activation: Activation::Home(HomeActivation {
                                            node: request.node,
                                            target: request.target,
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

trait ExtraSubstitutersFromHorizon {
    fn from_horizon_nodes(horizon: &Horizon, names: &[NodeName]) -> Result<ExtraSubstituters>;
}

impl ExtraSubstitutersFromHorizon for ExtraSubstituters {
    fn from_horizon_nodes(horizon: &Horizon, names: &[NodeName]) -> Result<ExtraSubstituters> {
        let mut entries = Vec::new();
        for name in names {
            let node = HorizonNodeLookup::new(horizon, name).node()?;
            let url = CacheEndpoint::from_node(node)
                .ok_or_else(|| Error::InvalidSubstituter(name.clone()))?;
            let Some(public_key) = &node.nix_pub_key_line else {
                return Err(Error::InvalidSubstituter(name.clone()));
            };
            entries.push(ExtraSubstituter::new(url.url(), public_key.as_str()));
        }
        Ok(ExtraSubstituters::from_entries(entries))
    }
}

struct CacheEndpoint<'node> {
    node: &'node Node,
}

impl<'node> CacheEndpoint<'node> {
    fn from_node(node: &'node Node) -> Option<Self> {
        node.nix_url.as_ref()?;
        Some(Self { node })
    }

    fn url(&self) -> String {
        match &self.node.ygg_address {
            Some(address) => format!("http://[{address}]"),
            None => self
                .node
                .nix_url
                .clone()
                .expect("CacheEndpoint only exists when nix_url exists"),
        }
    }
}

struct HorizonNodeLookup<'horizon, 'name> {
    horizon: &'horizon Horizon,
    name: &'name NodeName,
}

impl<'horizon, 'name> HorizonNodeLookup<'horizon, 'name> {
    fn new(horizon: &'horizon Horizon, name: &'name NodeName) -> Self {
        Self { horizon, name }
    }

    fn node(&self) -> Result<&'horizon Node> {
        if *self.name == self.horizon.node.name {
            return Ok(&self.horizon.node);
        }
        self.horizon
            .ex_nodes
            .get(self.name)
            .ok_or_else(|| Error::UnknownSubstituter(self.name.clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use horizon_lib::address::{YggAddress, YggSubnet};
    use horizon_lib::io::Io;
    use horizon_lib::machine::Machine;
    use horizon_lib::magnitude::Magnitude;
    use horizon_lib::name::{ClusterName, NodeName};
    use horizon_lib::proposal::{
        ClusterProposal, ClusterTrust, NodeProposal, NodePubKeys, YggPubKeyEntry,
    };
    use horizon_lib::pub_key::{NixPubKey, SshPubKey, YggPubKey};
    use horizon_lib::species::{Arch, Bootloader, Keyboard, MachineSpecies, NodeSpecies};

    use super::*;

    const NIX_KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn node_name(name: &str) -> NodeName {
        NodeName::try_new(name).unwrap()
    }

    fn cluster_name() -> ClusterName {
        ClusterName::try_new("goldragon").unwrap()
    }

    fn machine() -> Machine {
        Machine {
            species: MachineSpecies::Metal,
            arch: Some(Arch::X86_64),
            cores: 4,
            model: None,
            mother_board: None,
            super_node: None,
            super_user: None,
            chip_gen: None,
            ram_gb: None,
        }
    }

    fn io() -> Io {
        Io {
            keyboard: Keyboard::Qwerty,
            bootloader: Bootloader::Uefi,
            disks: BTreeMap::new(),
            swap_devices: Vec::new(),
        }
    }

    fn pub_keys(nix: bool, ygg: bool) -> NodePubKeys {
        NodePubKeys {
            ssh: SshPubKey::try_new("AAA=").unwrap(),
            nix: nix.then(|| NixPubKey::try_new(NIX_KEY).unwrap()),
            yggdrasil: ygg.then(|| YggPubKeyEntry {
                pub_key: YggPubKey::try_new("a".repeat(64)).unwrap(),
                address: YggAddress::try_new("200::1").unwrap(),
                subnet: YggSubnet::try_new("300:ca41:6b12:fba").unwrap(),
            }),
        }
    }

    fn node_proposal(species: NodeSpecies, size: Magnitude, nix: bool, ygg: bool) -> NodeProposal {
        NodeProposal {
            species,
            size,
            trust: Magnitude::Max,
            machine: machine(),
            io: io(),
            pub_keys: pub_keys(nix, ygg),
            link_local_ips: Vec::new(),
            node_ip: None,
            wireguard_pub_key: None,
            nordvpn: false,
            wifi_cert: false,
            wireguard_untrusted_proxies: Vec::new(),
            wants_printing: false,
            wants_hw_video_accel: false,
            router_interfaces: None,
            online: None,
            nb_of_build_cores: None,
        }
    }

    fn projected_horizon() -> Horizon {
        let target = node_name("zeus");
        let cache = node_name("prometheus");
        let mut nodes = BTreeMap::new();
        nodes.insert(
            target.clone(),
            node_proposal(NodeSpecies::Edge, Magnitude::Min, false, false),
        );
        nodes.insert(
            cache,
            node_proposal(NodeSpecies::Center, Magnitude::Min, true, true),
        );

        ClusterProposal {
            nodes,
            users: BTreeMap::new(),
            domains: BTreeMap::new(),
            trust: ClusterTrust {
                cluster: Magnitude::Max,
                clusters: BTreeMap::new(),
                nodes: BTreeMap::new(),
                users: BTreeMap::new(),
            },
        }
        .project(&Viewpoint {
            cluster: cluster_name(),
            node: target,
        })
        .unwrap()
    }

    #[test]
    fn substituter_resolution_prefers_ygg_endpoint_over_nix_url() {
        let horizon = projected_horizon();
        let substituters =
            ExtraSubstituters::from_horizon_nodes(&horizon, &[node_name("prometheus")]).unwrap();

        assert_eq!(substituters.urls_text(), "http://[200::1]");
        assert_eq!(
            substituters.public_keys_text(),
            "prometheus.goldragon.criome:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        );
    }

    #[test]
    fn substituter_resolution_falls_back_to_nix_url_without_ygg_endpoint() {
        let mut horizon = projected_horizon();
        horizon
            .ex_nodes
            .get_mut(&node_name("prometheus"))
            .unwrap()
            .ygg_address = None;

        let substituters =
            ExtraSubstituters::from_horizon_nodes(&horizon, &[node_name("prometheus")]).unwrap();

        assert_eq!(
            substituters.urls_text(),
            "http://nix.prometheus.goldragon.criome"
        );
    }

    #[test]
    fn unknown_substituter_reports_unknown_substituter() {
        let horizon = projected_horizon();
        let error =
            ExtraSubstituters::from_horizon_nodes(&horizon, &[node_name("missing")]).unwrap_err();

        assert!(
            matches!(error, Error::UnknownSubstituter(ref name) if name.as_str() == "missing"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn node_without_cache_endpoint_reports_invalid_substituter() {
        let horizon = projected_horizon();
        let error =
            ExtraSubstituters::from_horizon_nodes(&horizon, &[node_name("zeus")]).unwrap_err();

        assert!(
            matches!(error, Error::InvalidSubstituter(ref name) if name.as_str() == "zeus"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn cache_endpoint_without_public_key_reports_invalid_substituter() {
        let mut horizon = projected_horizon();
        horizon
            .ex_nodes
            .get_mut(&node_name("prometheus"))
            .unwrap()
            .nix_pub_key_line = None;

        let error = ExtraSubstituters::from_horizon_nodes(&horizon, &[node_name("prometheus")])
            .unwrap_err();

        assert!(
            matches!(error, Error::InvalidSubstituter(ref name) if name.as_str() == "prometheus"),
            "unexpected error: {error}"
        );
    }
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
