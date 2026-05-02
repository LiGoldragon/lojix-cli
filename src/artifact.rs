use std::path::{Path, PathBuf};

use horizon_lib::Horizon;
use horizon_lib::name::{ClusterName, NodeName};
use horizon_lib::species::System;
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};

use crate::build::DeploymentShape;
use crate::cluster::{NarHashSri, OverrideUri};
use crate::error::{Error, Result};
use crate::process::{ProcessFailure, ProcessInvocation, ProcessRun};

const HORIZON_FLAKE_TEMPLATE: &str = "{\n\
\x20 outputs = _: {\n\
\x20   horizon = builtins.fromJSON (builtins.readFile ./horizon.json);\n\
\x20 };\n\
}\n";

const SYSTEM_FLAKE_TEMPLATE_PREFIX: &str = "{\n\
\x20 outputs = _: {\n\
\x20   system = \"";
const SYSTEM_FLAKE_TEMPLATE_SUFFIX: &str = "\";\n\
\x20 };\n\
}\n";

struct NixSystemName(&'static str);

impl NixSystemName {
    fn from_system(system: System) -> Self {
        match system {
            System::X86_64Linux => Self("x86_64-linux"),
            System::Aarch64Linux => Self("aarch64-linux"),
        }
    }

    fn as_str(&self) -> &str {
        self.0
    }
}

pub struct HorizonDir(PathBuf);

pub struct HorizonCacheKey<'key> {
    pub cluster: &'key ClusterName,
    pub node: &'key NodeName,
}

impl HorizonDir {
    pub fn try_create_cache(key: HorizonCacheKey<'_>) -> Result<Self> {
        let home = std::env::var("HOME").map_err(|_| Error::NoHome)?;
        let dir = PathBuf::from(home)
            .join(".cache/lojix/horizon")
            .join(key.cluster.as_str())
            .join(key.node.as_str());
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    pub fn write(&self, horizon: &Horizon) -> Result<()> {
        let json = serde_json::to_string_pretty(horizon)?;
        std::fs::write(self.0.join("horizon.json"), json)?;
        std::fs::write(self.0.join("flake.nix"), HORIZON_FLAKE_TEMPLATE)?;
        Ok(())
    }

    pub async fn nar_hash(&self) -> Result<NarHashSri> {
        NarHashInput::from_directory(&self.0).calculate().await
    }

    pub fn override_uri(&self) -> OverrideUri {
        OverrideUri::from_local_path(&self.0)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

pub struct SystemDir(PathBuf);

impl SystemDir {
    pub fn try_create_cache(system: System) -> Result<Self> {
        let home = std::env::var("HOME").map_err(|_| Error::NoHome)?;
        let dir = PathBuf::from(home)
            .join(".cache/lojix/system")
            .join(NixSystemName::from_system(system).as_str());
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    pub fn write(&self, system: System) -> Result<()> {
        let mut flake = String::new();
        flake.push_str(SYSTEM_FLAKE_TEMPLATE_PREFIX);
        flake.push_str(NixSystemName::from_system(system).as_str());
        flake.push_str(SYSTEM_FLAKE_TEMPLATE_SUFFIX);
        std::fs::write(self.0.join("flake.nix"), flake)?;
        Ok(())
    }

    pub async fn nar_hash(&self) -> Result<NarHashSri> {
        NarHashInput::from_directory(&self.0).calculate().await
    }

    pub fn override_uri(&self) -> OverrideUri {
        OverrideUri::from_local_path(&self.0)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

pub struct DeploymentDir(PathBuf);

impl DeploymentDir {
    pub fn try_create_cache(shape: DeploymentShape) -> Result<Self> {
        let home = std::env::var("HOME").map_err(|_| Error::NoHome)?;
        let dir = PathBuf::from(home)
            .join(".cache/lojix/deployment")
            .join(shape.cache_name());
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    pub fn write(&self, shape: DeploymentShape) -> Result<()> {
        std::fs::write(self.0.join("flake.nix"), shape.flake_text())?;
        Ok(())
    }

    pub async fn nar_hash(&self) -> Result<NarHashSri> {
        NarHashInput::from_directory(&self.0).calculate().await
    }

    pub fn override_uri(&self) -> OverrideUri {
        OverrideUri::from_local_path(&self.0)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

struct NarHashInput<'directory> {
    directory: &'directory Path,
}

impl<'directory> NarHashInput<'directory> {
    fn from_directory(directory: &'directory Path) -> Self {
        Self { directory }
    }

    fn invocation(&self) -> ProcessInvocation {
        ProcessInvocation::new("nix")
            .with_arguments(["hash", "path", "--type", "sha256", "--sri"])
            .with_argument(self.directory.display().to_string())
    }

    async fn calculate(&self) -> Result<NarHashSri> {
        let output = self
            .invocation()
            .capture_stdout(ProcessRun::capture_stderr(ProcessFailure::Nix))
            .await?;
        NarHashSri::try_new(output.stdout().trim().to_string())
    }
}

pub struct MaterializedArtifact {
    pub horizon_dir: HorizonDir,
    pub system_dir: SystemDir,
    pub deployment_dir: DeploymentDir,
    pub horizon_nar_hash: NarHashSri,
    pub system_nar_hash: NarHashSri,
    pub deployment_nar_hash: NarHashSri,
    pub horizon_uri: OverrideUri,
    pub system_uri: OverrideUri,
    pub deployment_uri: OverrideUri,
}

pub struct HorizonArtifact;

pub struct ArtifactMaterialization {
    horizon: Horizon,
    cluster: ClusterName,
    node: NodeName,
    deployment_shape: DeploymentShape,
}

pub struct ArtifactMaterializationInput {
    pub horizon: Horizon,
    pub cluster: ClusterName,
    pub node: NodeName,
    pub deployment_shape: DeploymentShape,
}

impl ArtifactMaterialization {
    pub fn from_input(input: ArtifactMaterializationInput) -> Self {
        Self {
            horizon: input.horizon,
            cluster: input.cluster,
            node: input.node,
            deployment_shape: input.deployment_shape,
        }
    }

    pub async fn materialize(&self) -> Result<MaterializedArtifact> {
        let horizon_dir = HorizonDir::try_create_cache(HorizonCacheKey {
            cluster: &self.cluster,
            node: &self.node,
        })?;
        horizon_dir.write(&self.horizon)?;
        let horizon_nar_hash = horizon_dir.nar_hash().await?;
        let horizon_uri = horizon_dir.override_uri();

        let system_dir = SystemDir::try_create_cache(self.horizon.node.system)?;
        system_dir.write(self.horizon.node.system)?;
        let system_nar_hash = system_dir.nar_hash().await?;
        let system_uri = system_dir.override_uri();

        let deployment_dir = DeploymentDir::try_create_cache(self.deployment_shape)?;
        deployment_dir.write(self.deployment_shape)?;
        let deployment_nar_hash = deployment_dir.nar_hash().await?;
        let deployment_uri = deployment_dir.override_uri();

        Ok(MaterializedArtifact {
            horizon_dir,
            system_dir,
            deployment_dir,
            horizon_nar_hash,
            system_nar_hash,
            deployment_nar_hash,
            horizon_uri,
            system_uri,
            deployment_uri,
        })
    }
}

pub enum ArtifactMsg {
    Materialize {
        materialization: ArtifactMaterialization,
        reply: RpcReplyPort<Result<MaterializedArtifact>>,
    },
}

#[ractor::async_trait]
impl Actor for HorizonArtifact {
    type Msg = ArtifactMsg;
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
            ArtifactMsg::Materialize {
                materialization,
                reply,
            } => {
                let _ = reply.send(materialization.materialize().await);
            }
        }
        Ok(())
    }
}
