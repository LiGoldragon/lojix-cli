use std::path::{Path, PathBuf};

use crate::cluster::{FlakeInputRef, NarHashSri};
use crate::error::{Error, Result};
use crate::process::{ProcessFailure, ProcessInvocation, ProcessRun};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratedInputKind {
    Horizon,
    System,
    Deployment,
    HomeWrapper,
}

impl GeneratedInputKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Horizon => "horizon",
            Self::System => "system",
            Self::Deployment => "deployment",
            Self::HomeWrapper => "home-wrapper",
        }
    }
}

#[derive(Debug)]
pub struct GeneratedInputArchive<'input> {
    pub kind: GeneratedInputKind,
    pub directory: &'input Path,
    pub nar_hash: &'input NarHashSri,
}

impl<'input> GeneratedInputArchive<'input> {
    pub fn archive_name(&self) -> ArchiveName {
        ArchiveName::new(self.kind, self.nar_hash)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveName {
    kind: GeneratedInputKind,
    short_code: String,
}

impl ArchiveName {
    pub fn new(kind: GeneratedInputKind, nar_hash: &NarHashSri) -> Self {
        Self {
            kind,
            short_code: nar_hash.short_code(),
        }
    }

    pub fn relative_path(&self) -> String {
        format!("{}/{}.tar.gz", self.kind.as_str(), self.short_code)
    }
}

#[derive(Debug, Clone)]
pub struct ArchivePublisher {
    target: String,
    remote_root: PathBuf,
    base_url: String,
}

impl ArchivePublisher {
    pub fn from_environment() -> Result<Self> {
        let target = std::env::var("LOJIX_ARCHIVE_SSH_TARGET")
            .unwrap_or_else(|_| "root@prometheus.goldragon.criome".to_string());
        let remote_root = std::env::var_os("LOJIX_ARCHIVE_REMOTE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/lib/lojix-inputs"));
        let base_url = std::env::var("LOJIX_ARCHIVE_BASE_URL")
            .unwrap_or_else(|_| "http://prometheus.goldragon.criome/lojix-inputs".to_string());
        Ok(Self {
            target,
            remote_root,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    pub async fn publish(&self, archive: GeneratedInputArchive<'_>) -> Result<FlakeInputRef> {
        let name = archive.archive_name();
        let archive_path = self.archive_path(&name)?;
        TarArchive::new(archive.directory, &archive_path)
            .create()
            .await?;
        self.upload(&archive_path, &name).await?;
        Ok(FlakeInputRef::from_archive_url(
            self.archive_url(&name),
            archive.nar_hash.clone(),
        ))
    }

    fn archive_path(&self, name: &ArchiveName) -> Result<PathBuf> {
        let home = std::env::var("HOME").map_err(|_| Error::NoHome)?;
        let path = PathBuf::from(home)
            .join(".cache/lojix/archives")
            .join(name.relative_path());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(path)
    }

    fn archive_url(&self, name: &ArchiveName) -> String {
        format!("{}/{}", self.base_url, name.relative_path())
    }

    async fn upload(&self, archive_path: &Path, name: &ArchiveName) -> Result<()> {
        let remote_dir = self.remote_root.join(name.kind.as_str());
        ProcessInvocation::new("ssh")
            .with_arguments([
                "-o".to_string(),
                "BatchMode=yes".to_string(),
                self.target.clone(),
                format!("mkdir -p {}", remote_dir.display()),
            ])
            .capture_stdout(ProcessRun::capture_stderr(ProcessFailure::Ssh))
            .await?;

        let destination = format!("{}:{}/", self.target, remote_dir.display());
        ProcessInvocation::new("rsync")
            .with_arguments([
                "-a".to_string(),
                "-e".to_string(),
                "ssh -o BatchMode=yes".to_string(),
                archive_path.display().to_string(),
                destination,
            ])
            .capture_stdout(ProcessRun::capture_stderr(ProcessFailure::Rsync))
            .await?;
        Ok(())
    }
}

struct TarArchive<'archive> {
    source_directory: &'archive Path,
    archive_path: &'archive Path,
}

impl<'archive> TarArchive<'archive> {
    fn new(source_directory: &'archive Path, archive_path: &'archive Path) -> Self {
        Self {
            source_directory,
            archive_path,
        }
    }

    async fn create(&self) -> Result<()> {
        ProcessInvocation::new("tar")
            .with_arguments([
                "-C".to_string(),
                self.source_directory.display().to_string(),
                "-czf".to_string(),
                self.archive_path.display().to_string(),
                ".".to_string(),
            ])
            .capture_stdout(ProcessRun::capture_stderr(ProcessFailure::Tar))
            .await?;
        Ok(())
    }
}
