use horizon_lib::name::{CriomeDomainName, UserName};
use horizon_lib::view::Node;

use crate::process::{ProcessInvocation, ShellCommand};

/// SSH target — `<user>@<node>.<cluster>.criome` — the addressing
/// used by `ssh`, `nix copy --to ssh-ng://…`, and
/// `--from ssh-ng://…`. Built from the projected horizon's
/// `criome_domain_name`; never from a literal hostname.
///
/// `user` and `domain` live as separate typed fields so
/// `with_user` is a struct rebuild rather than a string parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget {
    user: String,
    domain: CriomeDomainName,
}

impl SshTarget {
    pub fn from_node(node: &Node) -> Self {
        Self::root_at(node.criome_domain_name.clone())
    }

    pub fn from_criome_domain(domain: &CriomeDomainName) -> Self {
        Self::root_at(domain.clone())
    }

    pub fn from_user_at_domain(user: &UserName, domain: &CriomeDomainName) -> Self {
        Self {
            user: user.as_str().to_string(),
            domain: domain.clone(),
        }
    }

    fn root_at(domain: CriomeDomainName) -> Self {
        Self {
            user: "root".to_string(),
            domain,
        }
    }

    pub fn with_user(&self, user: &UserName) -> Self {
        Self {
            user: user.as_str().to_string(),
            domain: self.domain.clone(),
        }
    }

    pub fn ssh_uri(&self) -> String {
        format!("ssh-ng://{}@{}", self.user, self.domain.as_str())
    }

    pub fn as_ssh_arg(&self) -> String {
        format!("{}@{}", self.user, self.domain.as_str())
    }

    pub fn remote_invocation(&self, remote_command: ShellCommand) -> ProcessInvocation {
        ProcessInvocation::new("ssh").with_arguments([
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            self.as_ssh_arg(),
            remote_command.as_str().to_string(),
        ])
    }
}

impl std::fmt::Display for SshTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}@{}", self.user, self.domain.as_str())
    }
}
