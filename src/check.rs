//! `CheckHostKeyMaterial` — read-only diff between horizon-expected
//! per-host public material and what the host actually has on disk
//! (its `publication.nota`).
//!
//! Per option 1 of primary-da7 (orchestrator queries cluster DB;
//! clavifaber stays cluster-unaware): lojix is the orchestrator,
//! goldragon's datom is the cluster DB (haywire stage), horizon-rs
//! is the projection from datom to per-host expectations.
//!
//! No mutation. Prints what mismatches. Operator decides what to
//! do — typically `rm` the offending file on the host and re-run
//! clavifaber's matching verb (per the loud-fail policy in
//! clavifaber/skills.md §"Force-rotate").

use clavifaber::publication::PublicKeyPublication;
use horizon_lib::Horizon;
use horizon_lib::address::YggAddress;
use horizon_lib::name::{ClusterName, NodeName};
use horizon_lib::pub_key::{SshPubKey, YggPubKey};
use nota_codec::{Decoder, NotaDecode, NotaRecord};
use std::fmt::Write;

use crate::cluster::ProposalSource;
use crate::error::{Error, Result};
use crate::host::SshTarget;
use crate::process::{ProcessFailure, ProcessRun, ShellCommand};

#[derive(Debug, Clone, PartialEq, Eq, NotaRecord)]
pub struct CheckHostKeyMaterial {
    pub cluster: ClusterName,
    pub node: NodeName,
    pub source: ProposalSource,
}

impl CheckHostKeyMaterial {
    pub async fn run(self) -> Result<Report> {
        let horizon = self.project_horizon()?;
        let publication_text = collect_publication(&horizon).await?;
        let publication = parse_publication(&publication_text)?;
        Ok(diff(&horizon, &publication))
    }

    fn project_horizon(&self) -> Result<Horizon> {
        let proposal = self.source.load()?;
        let viewpoint = horizon_lib::Viewpoint {
            cluster: self.cluster.clone(),
            node: self.node.clone(),
        };
        Ok(proposal.project(&viewpoint)?)
    }
}

async fn collect_publication(horizon: &Horizon) -> Result<String> {
    let target = SshTarget::from_node(&horizon.node);
    let invocation = target.remote_invocation(ShellCommand::from_raw(
        "cat /etc/criomOS/complex/publication.nota",
    ));
    let output = invocation
        .capture_stdout(ProcessRun::capture_stderr(ProcessFailure::Ssh))
        .await?;
    Ok(output.stdout().to_string())
}

fn parse_publication(text: &str) -> Result<PublicKeyPublication> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(Error::CheckHostKeyMaterial(
            "host returned empty publication.nota; the file may not exist on the host yet"
                .to_string(),
        ));
    }
    let mut decoder = Decoder::new(trimmed);
    let publication = PublicKeyPublication::decode(&mut decoder).map_err(|error| {
        Error::CheckHostKeyMaterial(format!("decode publication.nota: {error}"))
    })?;
    Ok(publication)
}

/// One mismatch entry. Each carries a short tag (the concern), what
/// the cluster expects, what the host has on disk, and a one-line
/// operator hint pointing at the clavifaber verb that fixes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    pub concern: &'static str,
    pub expected: String,
    pub actual: String,
    pub operator_hint: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub node: NodeName,
    pub mismatches: Vec<Mismatch>,
}

impl Report {
    pub fn is_consistent(&self) -> bool {
        self.mismatches.is_empty()
    }

    pub fn render_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "=== CheckHostKeyMaterial: {} ===", self.node.as_str());
        if self.is_consistent() {
            let _ = writeln!(
                out,
                "    no mismatches — host's publication.nota matches horizon"
            );
            return out;
        }
        let _ = writeln!(
            out,
            "    {} mismatch(es) between horizon's expected and the host's publication.nota:",
            self.mismatches.len()
        );
        for mismatch in &self.mismatches {
            let _ = writeln!(out);
            let _ = writeln!(out, "  [{}]", mismatch.concern);
            let _ = writeln!(out, "    expected (horizon): {}", mismatch.expected);
            let _ = writeln!(out, "    actual (host):      {}", mismatch.actual);
            let _ = writeln!(out, "    operator hint:      {}", mismatch.operator_hint);
        }
        out
    }
}

/// Compute the diff between horizon's per-host expectations and what
/// the host's `publication.nota` actually declares. Pure: no I/O, no
/// mutation. Exposed for integration tests (see
/// `tests/check_host_key_material.rs`); the runtime caller is the
/// `CheckHostKeyMaterial::run` async pipeline above.
pub fn diff(horizon: &Horizon, publication: &PublicKeyPublication) -> Report {
    let mut mismatches = Vec::new();

    // ssh public key. horizon stores the base64 without the
    // `ssh-ed25519 ` prefix or comment; the publication carries
    // sshd's full line. Compare base64.
    let expected_ssh = ssh_base64(&horizon.node.ssh_pub_key);
    let actual_ssh = base64_from_ssh_line(&publication.open_ssh_public_key);
    if expected_ssh != actual_ssh {
        mismatches.push(Mismatch {
            concern: "ssh-public-key",
            expected: expected_ssh,
            actual: actual_ssh,
            operator_hint:
                "host's /etc/ssh/ssh_host_ed25519_key.pub disagrees with goldragon's NodePubKeys.ssh — either rotate sshd's key (rm + restart sshd) and re-run PublicKeyPublicationWriting, or update goldragon and redeploy",
        });
    }

    // Yggdrasil. horizon flattens to `ygg_pub_key` + `ygg_address`
    // (both `Option`); the publication's `yggdrasil` is `Option`.
    // Three cases: both Some (compare); horizon Some + publication
    // None (host hasn't run YggdrasilKeypairSetup); horizon None +
    // publication Some (host has identity goldragon doesn't know
    // about).
    let expected_ygg = horizon.node.ygg_pub_key.as_ref();
    let expected_ygg_address = horizon.node.ygg_address.as_ref();
    let actual_ygg = publication.yggdrasil.as_ref();
    match (expected_ygg, actual_ygg) {
        (Some(expected_key), Some(actual)) => {
            if ygg_pub_key_text(expected_key) != actual.public_key {
                mismatches.push(Mismatch {
                    concern: "yggdrasil-public-key",
                    expected: ygg_pub_key_text(expected_key),
                    actual: actual.public_key.clone(),
                    operator_hint:
                        "host's yggdrasil keypair derives a different public key than goldragon expects — either rm the host's yggdrasil keypair file and re-run YggdrasilKeypairSetup + PublicKeyPublicationWriting, or update goldragon",
                });
            }
            if let Some(expected_address) = expected_ygg_address
                && ygg_address_text(expected_address) != actual.address
            {
                mismatches.push(Mismatch {
                    concern: "yggdrasil-address",
                    expected: ygg_address_text(expected_address),
                    actual: actual.address.clone(),
                    operator_hint:
                        "yggdrasil address mismatch (typically follows a public-key mismatch)",
                });
            }
        }
        (Some(expected_key), None) => {
            mismatches.push(Mismatch {
                concern: "yggdrasil-public-key",
                expected: ygg_pub_key_text(expected_key),
                actual: "<absent in publication>".to_string(),
                operator_hint:
                    "goldragon declares a yggdrasil identity for this host but the publication has none — run clavifaber's YggdrasilKeypairSetup + PublicKeyPublicationWriting on the host",
            });
        }
        (None, Some(actual)) => {
            mismatches.push(Mismatch {
                concern: "yggdrasil-public-key",
                expected: "<absent in goldragon>".to_string(),
                actual: actual.public_key.clone(),
                operator_hint:
                    "host publishes a yggdrasil identity goldragon doesn't know about — either add it to goldragon (preferred) or rm the keypair file on the host and re-run PublicKeyPublicationWriting without yggdrasil",
            });
        }
        (None, None) => {}
    }

    Report {
        node: horizon.node.name.clone(),
        mismatches,
    }
}

/// Extract just the base64 portion of an ssh-ed25519 line.
/// `ssh-ed25519 AAAAC3... [comment]` → `AAAAC3...`.
fn base64_from_ssh_line(line: &str) -> String {
    let trimmed = line.trim();
    let mut parts = trimmed.splitn(3, ' ');
    let _algorithm = parts.next();
    parts.next().unwrap_or(trimmed).to_string()
}

fn ssh_base64(key: &SshPubKey) -> String {
    key.as_str().to_string()
}

fn ygg_pub_key_text(key: &YggPubKey) -> String {
    key.as_str().to_string()
}

fn ygg_address_text(address: &YggAddress) -> String {
    address.clone().ipv6().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_strip_handles_a_simple_ssh_line() {
        assert_eq!(
            base64_from_ssh_line("ssh-ed25519 AAAABBBB host-comment"),
            "AAAABBBB"
        );
    }

    #[test]
    fn base64_strip_handles_no_comment() {
        assert_eq!(base64_from_ssh_line("ssh-ed25519 AAAABBBB"), "AAAABBBB");
    }

    #[test]
    fn base64_strip_handles_trailing_newline() {
        assert_eq!(
            base64_from_ssh_line("ssh-ed25519 AAAABBBB host\n"),
            "AAAABBBB"
        );
    }
}
