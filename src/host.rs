use horizon_lib::name::{CriomeDomainName, UserName};
use horizon_lib::node::Node;

use crate::process::{ProcessInvocation, ShellCommand};

/// `root@<node>.<cluster>.criome` — the addressing used by `ssh`,
/// `nix copy --to ssh-ng://…`, and `--from ssh-ng://…`. Constructed
/// from the projected horizon's `criome_domain_name`; never built
/// from a literal hostname.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget(String);

impl SshTarget {
    pub fn from_node(node: &Node) -> Self {
        Self::from_criome_domain(&node.criome_domain_name)
    }

    pub fn from_criome_domain(domain: &CriomeDomainName) -> Self {
        Self(format!("root@{}", domain.as_str()))
    }

    pub fn from_user_at_domain(user: &UserName, domain: &CriomeDomainName) -> Self {
        Self(format!("{}@{}", user.as_str(), domain.as_str()))
    }

    pub fn with_user(&self, user: &UserName) -> Self {
        let domain = self
            .0
            .split_once('@')
            .map_or(self.0.as_str(), |(_, domain)| domain);
        Self(format!("{}@{}", user.as_str(), domain))
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
