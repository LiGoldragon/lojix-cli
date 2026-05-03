use std::path::{Path, PathBuf};

use horizon_lib::Horizon;
use horizon_lib::name::{ClusterName, NodeName, UserName};
use horizon_lib::species::System;
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};

use crate::build::DeploymentShape;
use crate::cluster::{FlakeInputRef, FlakeRef, NarHashSri};
use crate::error::{Error, Result};
use crate::process::{ProcessFailure, ProcessInvocation, ProcessRun};
use crate::publish::{ArchivePublisher, GeneratedInputArchive, GeneratedInputKind};

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

    pub fn path(&self) -> &Path {
        &self.0
    }
}

pub struct HomeWrapperDir(PathBuf);

pub struct HomeWrapperCacheKey<'key> {
    pub cluster: &'key ClusterName,
    pub node: &'key NodeName,
    pub user: &'key UserName,
}

pub struct HomeWrapperSpec<'spec> {
    pub home: &'spec FlakeRef,
    pub horizon_ref: &'spec FlakeInputRef,
    pub system_ref: &'spec FlakeInputRef,
    pub user: &'spec UserName,
    pub system: System,
}

impl HomeWrapperDir {
    pub fn try_create_cache(key: HomeWrapperCacheKey<'_>) -> Result<Self> {
        let home = std::env::var("HOME").map_err(|_| Error::NoHome)?;
        let dir = PathBuf::from(home)
            .join(".cache/lojix/home-wrapper")
            .join(key.cluster.as_str())
            .join(key.node.as_str())
            .join(key.user.as_str());
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    pub fn write(&self, spec: HomeWrapperSpec<'_>) -> Result<()> {
        let lock = self.0.join("flake.lock");
        match std::fs::remove_file(lock) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        let system = NixSystemName::from_system(spec.system);
        let user = spec.user.as_str();
        let home = spec.home.nix_string_literal();
        let horizon = spec.horizon_ref.nix_string_literal();
        let target_system = spec.system_ref.nix_string_literal();
        let flake = format!(
            "{{\n\
             \x20 inputs = {{\n\
             \x20   criomos-home.url = {home};\n\
             \x20   home-manager.follows = \"criomos-home/home-manager\";\n\
             \x20   nixpkgs.follows = \"pkgs/nixpkgs\";\n\
             \x20   criomos-home.inputs.nixpkgs.follows = \"nixpkgs\";\n\
             \x20   criomos-lib.url = \"github:LiGoldragon/CriomOS-lib/main\";\n\
             \x20   criomos-home.inputs.criomos-lib.follows = \"criomos-lib\";\n\
             \x20   system.url = {target_system};\n\
             \x20   pkgs.url = \"github:LiGoldragon/CriomOS-pkgs/main\";\n\
             \x20   pkgs.inputs.system.follows = \"system\";\n\
             \x20   horizon.url = {horizon};\n\
             \x20 }};\n\
             \n\
             \x20 outputs = inputs:\n\
             \x20 let\n\
             \x20   system = inputs.system.system;\n\
             \x20   pkgs = inputs.pkgs.pkgs;\n\
             \x20   horizon = inputs.horizon.horizon;\n\
             \x20   userName = \"{user}\";\n\
             \x20   user = horizon.users.${{userName}};\n\
             \x20   home = inputs.home-manager.lib.homeManagerConfiguration {{\n\
             \x20     inherit pkgs;\n\
             \x20     extraSpecialArgs = {{ inherit horizon user; }};\n\
             \x20     modules = [\n\
             \x20       inputs.criomos-home.homeModules.default\n\
             \x20       ({{ lib, ... }}: {{\n\
             \x20         nixpkgs.overlays = lib.mkForce pkgs.overlays;\n\
             \x20         home.username = userName;\n\
             \x20         home.homeDirectory = \"/home/${{userName}}\";\n\
             \x20         home.stateVersion = \"26.05\";\n\
             \x20       }})\n\
             \x20     ];\n\
             \x20   }};\n\
             \x20 in {{\n\
             \x20   packages.{system}.activationPackage = home.activationPackage;\n\
             \x20   homeConfigurations.${{userName}} = home;\n\
             \x20 }};\n\
             }}\n",
            system = system.as_str(),
        );
        std::fs::write(self.0.join("flake.nix"), flake)?;
        Ok(())
    }

    pub async fn nar_hash(&self) -> Result<NarHashSri> {
        NarHashInput::from_directory(&self.0).calculate().await
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
    pub deployment_dir: Option<DeploymentDir>,
    pub home_wrapper_dir: Option<HomeWrapperDir>,
    pub horizon_nar_hash: NarHashSri,
    pub system_nar_hash: NarHashSri,
    pub deployment_nar_hash: Option<NarHashSri>,
    pub home_wrapper_nar_hash: Option<NarHashSri>,
    pub horizon_ref: FlakeInputRef,
    pub system_ref: FlakeInputRef,
    pub deployment_ref: Option<FlakeInputRef>,
    pub home_wrapper_ref: Option<FlakeInputRef>,
}

pub struct HorizonArtifact;

pub struct ArtifactMaterialization {
    horizon: Horizon,
    cluster: ClusterName,
    node: NodeName,
    deployment_shape: Option<DeploymentShape>,
    home: Option<HomeMaterialization>,
}

pub struct ArtifactMaterializationInput {
    pub horizon: Horizon,
    pub cluster: ClusterName,
    pub node: NodeName,
    pub deployment_shape: Option<DeploymentShape>,
    pub home: Option<HomeMaterialization>,
}

#[derive(Clone)]
pub struct HomeMaterialization {
    pub user: UserName,
    pub home: FlakeRef,
}

impl ArtifactMaterialization {
    pub fn from_input(input: ArtifactMaterializationInput) -> Self {
        Self {
            horizon: input.horizon,
            cluster: input.cluster,
            node: input.node,
            deployment_shape: input.deployment_shape,
            home: input.home,
        }
    }

    pub async fn materialize(&self) -> Result<MaterializedArtifact> {
        let publisher = ArchivePublisher::from_environment()?;

        let horizon_dir = HorizonDir::try_create_cache(HorizonCacheKey {
            cluster: &self.cluster,
            node: &self.node,
        })?;
        horizon_dir.write(&self.horizon)?;
        let horizon_nar_hash = horizon_dir.nar_hash().await?;
        let horizon_ref = publisher
            .publish(GeneratedInputArchive {
                kind: GeneratedInputKind::Horizon,
                directory: horizon_dir.path(),
                nar_hash: &horizon_nar_hash,
            })
            .await?;

        let system_dir = SystemDir::try_create_cache(self.horizon.node.system)?;
        system_dir.write(self.horizon.node.system)?;
        let system_nar_hash = system_dir.nar_hash().await?;
        let system_ref = publisher
            .publish(GeneratedInputArchive {
                kind: GeneratedInputKind::System,
                directory: system_dir.path(),
                nar_hash: &system_nar_hash,
            })
            .await?;

        let (deployment_dir, deployment_nar_hash, deployment_ref) = match self.deployment_shape {
            None => (None, None, None),
            Some(shape) => {
                let dir = DeploymentDir::try_create_cache(shape)?;
                dir.write(shape)?;
                let nar_hash = dir.nar_hash().await?;
                let input_ref = publisher
                    .publish(GeneratedInputArchive {
                        kind: GeneratedInputKind::Deployment,
                        directory: dir.path(),
                        nar_hash: &nar_hash,
                    })
                    .await?;
                (Some(dir), Some(nar_hash), Some(input_ref))
            }
        };

        let (home_wrapper_dir, home_wrapper_nar_hash, home_wrapper_ref) = match self.home.as_ref() {
            None => (None, None, None),
            Some(home) => {
                let dir = HomeWrapperDir::try_create_cache(HomeWrapperCacheKey {
                    cluster: &self.cluster,
                    node: &self.node,
                    user: &home.user,
                })?;
                dir.write(HomeWrapperSpec {
                    home: &home.home,
                    horizon_ref: &horizon_ref,
                    system_ref: &system_ref,
                    user: &home.user,
                    system: self.horizon.node.system,
                })?;
                let nar_hash = dir.nar_hash().await?;
                let input_ref = publisher
                    .publish(GeneratedInputArchive {
                        kind: GeneratedInputKind::HomeWrapper,
                        directory: dir.path(),
                        nar_hash: &nar_hash,
                    })
                    .await?;
                (Some(dir), Some(nar_hash), Some(input_ref))
            }
        };

        Ok(MaterializedArtifact {
            horizon_dir,
            system_dir,
            deployment_dir,
            home_wrapper_dir,
            horizon_nar_hash,
            system_nar_hash,
            deployment_nar_hash,
            home_wrapper_nar_hash,
            horizon_ref,
            system_ref,
            deployment_ref,
            home_wrapper_ref,
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
