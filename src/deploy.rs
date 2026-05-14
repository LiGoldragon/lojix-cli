use std::collections::BTreeMap;

use horizon_lib::name::{ClusterName, NodeName, UserName};
use horizon_lib::node::Node;
use horizon_lib::user::User;
use horizon_lib::{Horizon, Viewpoint};

use crate::activate::{Activation, HomeActivation, SystemActivation};
use crate::artifact::ArtifactMaterialization;
use crate::build::{
    BuildPhaseOutcome, BuildPlan, ExtraSubstituter, ExtraSubstituters, HomeMode, NixBuild,
};
use crate::cluster::{DerivationPath, FlakeRef, ProposalSource, StorePath};
use crate::copy::ClosureCopy;
use crate::error::{Error, Result};
use crate::host::SshTarget;
use crate::project::HorizonProjection;
use crate::stage::{BuildInputReferences, RemoteInputStage};

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

    /// Resolve the requested build host (if any) against the projected
    /// horizon. The viewpoint node is a local build on the deployment
    /// target, so it does not need to expose Nix's remote-builder
    /// service. A sibling build host must be a remote Nix builder
    /// because the dispatcher connects to it for the build phase.
    fn resolve_builder_target(&self, horizon: &Horizon) -> Result<Option<SshTarget>> {
        let Some(name) = &self.builder else {
            return Ok(None);
        };
        if *name == self.node {
            return Ok(Some(SshTarget::from_node(&horizon.node)));
        }
        let node = horizon
            .ex_nodes
            .get(name)
            .ok_or_else(|| Error::UnknownBuilder(name.clone()))?;
        if !node.is_remote_nix_builder {
            return Err(Error::InvalidBuilder(name.clone()));
        }
        Ok(Some(SshTarget::from_node(node)))
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

/// Execute a deploy request end-to-end: load proposal, project horizon,
/// materialize override flake inputs, run `nix`, then optionally copy
/// the closure and activate it on the target.
///
/// The pipeline stages are direct method calls on data nouns — there is
/// no actor framework. `nix build` of a NixOS system from cold cache can
/// take hours; the awaits below have no timeout.
pub async fn deploy(request: DeployRequest) -> Result<DeployOutcome> {
    let proposal = request.source.load()?;

    let viewpoint = Viewpoint {
        cluster: request.cluster.clone(),
        node: request.node.clone(),
    };
    let horizon = HorizonProjection::new(proposal, viewpoint).project()?;

    request.validate_home_user(&horizon.users)?;
    let extra_substituters =
        ExtraSubstituters::from_horizon_nodes(&horizon, &request.substituters)?;

    // The viewpoint node — the deploy target — is always
    // `horizon.node`; addressing comes from its
    // `criome_domain_name`. Per the network-neutrality rule there
    // is no `--target` flag.
    let target = SshTarget::from_node(&horizon.node);
    let builder_target = request.resolve_builder_target(&horizon)?;
    let target_system = horizon.node.system;

    let deployment_shape = match request.plan {
        BuildPlan::System { .. } => Some(request.plan.deployment_shape()),
        BuildPlan::Home { .. } => None,
    };
    let materialized = ArtifactMaterialization::new(
        horizon,
        request.cluster.clone(),
        request.node.clone(),
        request.source.clone(),
        deployment_shape,
    )
    .materialize()
    .await?;

    let build_inputs = match &builder_target {
        None => BuildInputReferences::from_local_artifact(&materialized),
        Some(builder) => {
            RemoteInputStage::from_artifact(builder.clone(), &materialized)
                .run()
                .await?
        }
    };

    let outcome = NixBuild {
        flake: request.flake.clone(),
        system: target_system,
        horizon_ref: build_inputs.horizon_ref,
        system_ref: build_inputs.system_ref,
        deployment_ref: build_inputs.deployment_ref,
        secrets_ref: build_inputs.secrets_ref,
        extra_substituters,
        plan: request.plan.clone(),
        builder: builder_target.clone(),
    }
    .run()
    .await?;

    finish_deploy(request.plan, request.node, target, outcome).await
}

async fn finish_deploy(
    plan: BuildPlan,
    node: NodeName,
    target: SshTarget,
    outcome: BuildPhaseOutcome,
) -> Result<DeployOutcome> {
    match outcome {
        BuildPhaseOutcome::EvalDone { derivation_path } => {
            Ok(DeployOutcome::Evaluated { derivation_path })
        }
        BuildPhaseOutcome::BuildDone {
            store_path,
            location,
        } => {
            match plan {
                BuildPlan::System { action, .. } if action.activates() => {
                    ClosureCopy {
                        store_path: store_path.clone(),
                        source: location,
                        target: target.clone(),
                    }
                    .run()
                    .await?;
                    Activation::System(SystemActivation {
                        target,
                        store_path: store_path.clone(),
                        action,
                    })
                    .run()
                    .await?;
                }
                BuildPlan::System { .. } => {}
                BuildPlan::Home {
                    mode: HomeMode::Build,
                    ..
                } => {}
                BuildPlan::Home { user, mode } => {
                    ClosureCopy {
                        store_path: store_path.clone(),
                        source: location,
                        target: target.clone(),
                    }
                    .run()
                    .await?;
                    Activation::Home(HomeActivation {
                        node,
                        target,
                        user,
                        store_path: store_path.clone(),
                        mode,
                    })
                    .run()
                    .await?;
                }
            }
            Ok(DeployOutcome::Realized { store_path })
        }
    }
}

impl ExtraSubstituters {
    pub fn from_horizon_nodes(horizon: &Horizon, names: &[NodeName]) -> Result<Self> {
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
        Ok(Self::from_entries(entries))
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
