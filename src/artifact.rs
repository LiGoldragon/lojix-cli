use std::path::{Path, PathBuf};
use std::process::Command;

use horizon_lib::Horizon;
use horizon_lib::name::{ClusterName, NodeName};
use horizon_lib::species::System;
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};

use crate::cluster::{NarHashSri, OverrideUri};
use crate::error::{Error, Result};

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

fn nix_system(s: System) -> &'static str {
    match s {
        System::X86_64Linux => "x86_64-linux",
        System::Aarch64Linux => "aarch64-linux",
    }
}

pub struct HorizonDir(PathBuf);

impl HorizonDir {
    pub fn try_create_cache(cluster: &ClusterName, node: &NodeName) -> Result<Self> {
        let home = std::env::var("HOME").map_err(|_| Error::NoHome)?;
        let dir = PathBuf::from(home)
            .join(".cache/lojix/horizon")
            .join(cluster.as_str())
            .join(node.as_str());
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    pub fn write(&self, horizon: &Horizon) -> Result<()> {
        let json = serde_json::to_string_pretty(horizon)?;
        std::fs::write(self.0.join("horizon.json"), json)?;
        std::fs::write(self.0.join("flake.nix"), HORIZON_FLAKE_TEMPLATE)?;
        Ok(())
    }

    pub fn nar_hash(&self) -> Result<NarHashSri> {
        nar_hash_of(&self.0)
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
            .join(nix_system(system));
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    pub fn write(&self, system: System) -> Result<()> {
        let mut flake = String::new();
        flake.push_str(SYSTEM_FLAKE_TEMPLATE_PREFIX);
        flake.push_str(nix_system(system));
        flake.push_str(SYSTEM_FLAKE_TEMPLATE_SUFFIX);
        std::fs::write(self.0.join("flake.nix"), flake)?;
        Ok(())
    }

    pub fn nar_hash(&self) -> Result<NarHashSri> {
        nar_hash_of(&self.0)
    }

    pub fn override_uri(&self) -> OverrideUri {
        OverrideUri::from_local_path(&self.0)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

fn nar_hash_of(dir: &Path) -> Result<NarHashSri> {
    let out = Command::new("nix")
        .args(["hash", "path", "--type", "sha256", "--sri"])
        .arg(dir)
        .output()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        return Err(Error::NixFailed {
            status: out.status.code().unwrap_or(-1),
            stderr,
        });
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    NarHashSri::try_new(s)
}

pub struct MaterializedArtifact {
    pub horizon_dir: HorizonDir,
    pub system_dir: SystemDir,
    pub horizon_nar_hash: NarHashSri,
    pub system_nar_hash: NarHashSri,
    pub horizon_uri: OverrideUri,
    pub system_uri: OverrideUri,
}

pub struct HorizonArtifact;

impl HorizonArtifact {
    pub fn materialize(
        horizon: &Horizon,
        cluster: &ClusterName,
        node: &NodeName,
    ) -> Result<MaterializedArtifact> {
        let horizon_dir = HorizonDir::try_create_cache(cluster, node)?;
        horizon_dir.write(horizon)?;
        let horizon_nar_hash = horizon_dir.nar_hash()?;
        let horizon_uri = horizon_dir.override_uri();

        let system_dir = SystemDir::try_create_cache(horizon.node.system)?;
        system_dir.write(horizon.node.system)?;
        let system_nar_hash = system_dir.nar_hash()?;
        let system_uri = system_dir.override_uri();

        Ok(MaterializedArtifact {
            horizon_dir,
            system_dir,
            horizon_nar_hash,
            system_nar_hash,
            horizon_uri,
            system_uri,
        })
    }
}

pub enum ArtifactMsg {
    Materialize {
        horizon: Horizon,
        cluster: ClusterName,
        node: NodeName,
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
            ArtifactMsg::Materialize { horizon, cluster, node, reply } => {
                let _ = reply.send(Self::materialize(&horizon, &cluster, &node));
            }
        }
        Ok(())
    }
}
