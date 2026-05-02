use std::path::{Path, PathBuf};
use std::process::Stdio;

use horizon_lib::name::CriomeDomainName;
use horizon_lib::node::Node;
use process_wrap::tokio::*;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::cluster::OverrideUri;
use crate::error::{Error, Result};

/// `root@<node>.<cluster>.criome` — the addressing used by `ssh`,
/// `nix copy --to ssh-ng://…`, and `--from ssh-ng://…`. Constructed
/// from the projected horizon's `criome_domain_name`; never built
/// from a literal hostname.
#[derive(Debug, Clone)]
pub struct SshTarget(String);

impl SshTarget {
    pub fn from_node(node: &Node) -> Self {
        Self::from_criome_domain(&node.criome_domain_name)
    }

    pub fn from_criome_domain(domain: &CriomeDomainName) -> Self {
        Self(format!("root@{}", domain.as_str()))
    }

    pub fn ssh_uri(&self) -> String {
        format!("ssh-ng://{}", self.0)
    }

    pub fn as_ssh_arg(&self) -> &str {
        &self.0
    }
}

/// A scratch directory on a remote host into which override-input
/// flake dirs are rsynced before invoking `nix build`/`nix eval`
/// there. The remote root is created via `ssh root@host mktemp -d`
/// and removed via [`RemoteStaging::cleanup`] when the deploy is
/// done.
#[derive(Debug)]
pub struct RemoteStaging {
    target: SshTarget,
    remote_root: PathBuf,
}

impl RemoteStaging {
    pub async fn try_create(target: SshTarget) -> Result<Self> {
        let output = Command::new("ssh")
            .args([
                "-o",
                "BatchMode=yes",
                target.as_ssh_arg(),
                "mktemp",
                "-d",
                "/tmp/lojix-stage.XXXXXX",
            ])
            .output()
            .await?;
        if !output.status.success() {
            return Err(Error::SshFailed {
                status: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        let remote_root = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim().to_string());
        Ok(Self {
            target,
            remote_root,
        })
    }

    /// Rsync `local_dir`'s contents into `<remote_root>/<name>/` on
    /// the remote, returning an `OverrideUri` that resolves to the
    /// remote path. The URI string is interpreted by `nix` running
    /// on the *remote*, where the path now exists.
    pub async fn rsync(&self, local_dir: &Path, name: &str) -> Result<OverrideUri> {
        let remote_path = self.remote_root.join(name);
        // rsync expects a trailing slash on the source for "copy
        // contents into target dir" semantics. Build the spec
        // explicitly to avoid path-display ambiguity.
        let mut source = local_dir.as_os_str().to_os_string();
        source.push("/");
        let dest = format!("{}:{}/", self.target.as_ssh_arg(), remote_path.display(),);
        let mut wrap = CommandWrap::with_new("rsync", |c: &mut Command| {
            c.arg("-a")
                .arg("--delete")
                .arg("--mkpath")
                .arg("-e")
                .arg("ssh -o BatchMode=yes")
                .arg(&source)
                .arg(&dest)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
        });
        wrap.wrap(ProcessGroup::leader());
        wrap.wrap(KillOnDrop);
        let mut child = wrap.spawn()?;
        let mut stderr = String::new();
        if let Some(mut s) = child.stderr().take() {
            s.read_to_string(&mut stderr).await?;
        }
        let status = child.wait().await?;
        if !status.success() {
            return Err(Error::RsyncFailed {
                status: status.code().unwrap_or(-1),
                stderr,
            });
        }
        Ok(OverrideUri::from_local_path(&remote_path))
    }

    pub async fn cleanup(self) -> Result<()> {
        let output = Command::new("ssh")
            .args([
                "-o",
                "BatchMode=yes",
                self.target.as_ssh_arg(),
                "rm",
                "-rf",
                &self.remote_root.display().to_string(),
            ])
            .output()
            .await?;
        if !output.status.success() {
            return Err(Error::SshFailed {
                status: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(())
    }

    pub fn target(&self) -> &SshTarget {
        &self.target
    }
}
