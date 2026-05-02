use std::path::{Path, PathBuf};

use horizon_lib::name::CriomeDomainName;
use horizon_lib::node::Node;

use crate::cluster::OverrideUri;
use crate::error::Result;
use crate::process::{ProcessFailure, ProcessInvocation, ProcessRun, ShellCommand};

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

    pub fn remote_invocation(&self, remote_command: ShellCommand) -> ProcessInvocation {
        ProcessInvocation::new("ssh").with_arguments([
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            self.as_ssh_arg().to_string(),
            remote_command.as_str().to_string(),
        ])
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
        let output = ProcessInvocation::new("ssh")
            .with_arguments([
                "-o".to_string(),
                "BatchMode=yes".to_string(),
                target.as_ssh_arg().to_string(),
                "mktemp".to_string(),
                "-d".to_string(),
                "/tmp/lojix-stage.XXXXXX".to_string(),
            ])
            .capture_stdout(ProcessRun::capture_stderr(ProcessFailure::Ssh))
            .await?;
        let remote_root = PathBuf::from(output.stdout().trim().to_string());
        Ok(Self {
            target,
            remote_root,
        })
    }

    /// Rsync `local_dir`'s contents into `<remote_root>/<name>/` on
    /// the remote, returning an `OverrideUri` that resolves to the
    /// remote path. The URI string is interpreted by `nix` running
    /// on the *remote*, where the path now exists.
    pub async fn rsync(&self, request: RemoteRsync<'_>) -> Result<OverrideUri> {
        let remote_path = self.remote_root.join(request.name);
        // rsync expects a trailing slash on the source for "copy
        // contents into target dir" semantics. Build the spec
        // explicitly to avoid path-display ambiguity.
        let mut source = request.local_dir.as_os_str().to_os_string();
        source.push("/");
        let destination = format!("{}:{}/", self.target.as_ssh_arg(), remote_path.display(),);
        ProcessInvocation::new("rsync")
            .with_arguments([
                "-a".to_string(),
                "--delete".to_string(),
                "--mkpath".to_string(),
                "-e".to_string(),
                "ssh -o BatchMode=yes".to_string(),
                source.to_string_lossy().to_string(),
                destination,
            ])
            .capture_stdout(ProcessRun::capture_stderr(ProcessFailure::Rsync))
            .await?;
        Ok(OverrideUri::from_local_path(&remote_path))
    }

    pub async fn cleanup(self) -> Result<()> {
        ProcessInvocation::new("ssh")
            .with_arguments([
                "-o".to_string(),
                "BatchMode=yes".to_string(),
                self.target.as_ssh_arg().to_string(),
                "rm".to_string(),
                "-rf".to_string(),
                self.remote_root.display().to_string(),
            ])
            .capture_stdout(ProcessRun::capture_stderr(ProcessFailure::Ssh))
            .await?;
        Ok(())
    }

    pub fn target(&self) -> &SshTarget {
        &self.target
    }
}

pub struct RemoteRsync<'request> {
    pub local_dir: &'request Path,
    pub name: &'request str,
}
