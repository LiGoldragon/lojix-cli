use std::path::{Path, PathBuf};

use horizon_lib::Horizon;
use horizon_lib::name::{ClusterName, NodeName};
use horizon_lib::species::System;
use crate::build::DeploymentShape;
use crate::cluster::{FlakeInputRef, NarHashSri, ProposalSource};
use crate::error::{Error, Result};
use crate::process::{ProcessFailure, ProcessInvocation, ProcessRun};

const HORIZON_FLAKE_TEMPLATE: &str = "{
  outputs = _: {
    horizon = builtins.fromJSON (builtins.readFile ./horizon.json);
  };
}
";

const SOPS_FILE_EXTENSION: &str = "sops";

struct NixSystemName(&'static str);

impl NixSystemName {
    fn from_system(system: System) -> Self {
        match system {
            System::X86_64Linux => Self("x86_64-linux"),
            System::Aarch64Linux => Self("aarch64-linux"),
        }
    }

    fn as_str(&self) -> &'static str {
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
        let name = NixSystemName::from_system(system).as_str();
        let flake = format!(
            "{{
  outputs = _: {{
    system = \"{name}\";
  }};
}}
"
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

pub struct SecretsDir(PathBuf);

impl SecretsDir {
    pub fn try_create_cache(key: HorizonCacheKey<'_>) -> Result<Self> {
        let home = std::env::var("HOME").map_err(|_| Error::NoHome)?;
        let dir = PathBuf::from(home)
            .join(".cache/lojix/secrets")
            .join(key.cluster.as_str())
            .join(key.node.as_str());
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    fn write(&self, source: &ClusterSecrets) -> Result<()> {
        for entry in source.entries() {
            std::fs::copy(&entry.source_path, self.0.join(&entry.file_name))?;
        }
        std::fs::write(self.0.join("flake.nix"), source.flake_text())?;
        Ok(())
    }

    pub async fn nar_hash(&self) -> Result<NarHashSri> {
        NarHashInput::from_directory(&self.0).calculate().await
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

struct ClusterSecrets {
    entries: Vec<SecretEntry>,
}

struct SecretEntry {
    /// SecretReference name as authored in the datom (e.g.
    /// "router-wifi-sae-passwords"). Used as the attrset key in the
    /// generated `sopsFiles` flake output, and matches the lookup
    /// the consumer module performs (`sopsFiles.${name}`).
    secret_name: String,
    /// Filename inside the staged secrets directory (e.g.
    /// "router-wifi-sae-passwords.sops"). The on-disk basename
    /// matches `<secret_name>.sops` by convention.
    file_name: String,
    /// Absolute path to the source `.sops` file in the cluster repo.
    source_path: PathBuf,
}

impl ClusterSecrets {
    /// Scan the cluster repo's `secrets/` directory for every `.sops`
    /// file. Each file's stem becomes a SecretReference name; the file
    /// is staged into the generated secrets flake so consumer modules
    /// can resolve `inputs.secrets.sopsFiles.<name>`. Returns `None`
    /// when no secrets directory exists or it contains no `.sops`
    /// files.
    fn from_proposal_source(source: &ProposalSource) -> Option<Self> {
        let root = source.as_path().parent().unwrap_or_else(|| Path::new("."));
        let secrets_dir = root.join("secrets");
        let read = std::fs::read_dir(&secrets_dir).ok()?;
        let mut entries = Vec::new();
        for item in read.flatten() {
            let path = item.path();
            if path.extension().and_then(|e| e.to_str()) != Some(SOPS_FILE_EXTENSION) {
                continue;
            }
            let file_name = path.file_name()?.to_str()?.to_string();
            let secret_name = path.file_stem()?.to_str()?.to_string();
            entries.push(SecretEntry {
                secret_name,
                file_name,
                source_path: path,
            });
        }
        if entries.is_empty() {
            return None;
        }
        entries.sort_by(|a, b| a.secret_name.cmp(&b.secret_name));
        Some(Self { entries })
    }

    fn entries(&self) -> &[SecretEntry] {
        &self.entries
    }

    /// Generate the `flake.nix` whose `sopsFiles` attrset maps each
    /// SecretReference name to its staged file. Quoted attr keys
    /// because SecretReference names use kebab-case.
    fn flake_text(&self) -> String {
        let mut body = String::from("{\n  outputs = _: {\n    sopsFiles = {\n");
        for entry in &self.entries {
            body.push_str(&format!(
                "      \"{}\" = ./{};\n",
                entry.secret_name, entry.file_name,
            ));
        }
        body.push_str("    };\n  };\n}\n");
        body
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
    pub secrets_dir: Option<SecretsDir>,
    pub horizon_nar_hash: NarHashSri,
    pub system_nar_hash: NarHashSri,
    pub deployment_nar_hash: Option<NarHashSri>,
    pub secrets_nar_hash: Option<NarHashSri>,
    pub horizon_ref: FlakeInputRef,
    pub system_ref: FlakeInputRef,
    pub deployment_ref: Option<FlakeInputRef>,
    pub secrets_ref: Option<FlakeInputRef>,
}

pub struct ArtifactMaterialization {
    horizon: Horizon,
    cluster: ClusterName,
    node: NodeName,
    proposal_source: ProposalSource,
    deployment_shape: Option<DeploymentShape>,
}

impl ArtifactMaterialization {
    pub fn new(
        horizon: Horizon,
        cluster: ClusterName,
        node: NodeName,
        proposal_source: ProposalSource,
        deployment_shape: Option<DeploymentShape>,
    ) -> Self {
        Self {
            horizon,
            cluster,
            node,
            proposal_source,
            deployment_shape,
        }
    }

    pub async fn materialize(&self) -> Result<MaterializedArtifact> {
        let horizon_dir = HorizonDir::try_create_cache(HorizonCacheKey {
            cluster: &self.cluster,
            node: &self.node,
        })?;
        horizon_dir.write(&self.horizon)?;
        let horizon_nar_hash = horizon_dir.nar_hash().await?;
        let horizon_ref =
            FlakeInputRef::from_local_path(horizon_dir.path(), horizon_nar_hash.clone());

        let system_dir = SystemDir::try_create_cache(self.horizon.node.system)?;
        system_dir.write(self.horizon.node.system)?;
        let system_nar_hash = system_dir.nar_hash().await?;
        let system_ref = FlakeInputRef::from_local_path(system_dir.path(), system_nar_hash.clone());

        let (deployment_dir, deployment_nar_hash, deployment_ref) = match self.deployment_shape {
            None => (None, None, None),
            Some(shape) => {
                let dir = DeploymentDir::try_create_cache(shape)?;
                dir.write(shape)?;
                let nar_hash = dir.nar_hash().await?;
                let input_ref = FlakeInputRef::from_local_path(dir.path(), nar_hash.clone());
                (Some(dir), Some(nar_hash), Some(input_ref))
            }
        };

        let (secrets_dir, secrets_nar_hash, secrets_ref) =
            match ClusterSecrets::from_proposal_source(&self.proposal_source) {
                None => (None, None, None),
                Some(source) => {
                    let dir = SecretsDir::try_create_cache(HorizonCacheKey {
                        cluster: &self.cluster,
                        node: &self.node,
                    })?;
                    dir.write(&source)?;
                    let nar_hash = dir.nar_hash().await?;
                    let input_ref = FlakeInputRef::from_local_path(dir.path(), nar_hash.clone());
                    (Some(dir), Some(nar_hash), Some(input_ref))
                }
            };

        Ok(MaterializedArtifact {
            horizon_dir,
            system_dir,
            deployment_dir,
            secrets_dir,
            horizon_nar_hash,
            system_nar_hash,
            deployment_nar_hash,
            secrets_nar_hash,
            horizon_ref,
            system_ref,
            deployment_ref,
            secrets_ref,
        })
    }
}
