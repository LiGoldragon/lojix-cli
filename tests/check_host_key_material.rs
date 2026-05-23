//! Witness tests for `CheckHostKeyMaterial`'s diff logic.
//!
//! Constructs a synthetic horizon (one viewpoint node) plus a synthetic
//! publication, then calls `check::diff` and inspects the produced
//! `Report`. Covers the matrix of (ssh-match / ssh-mismatch) ×
//! (yggdrasil-{both-match, both-mismatch-key, both-match-key-mismatch-address,
//! horizon-only, publication-only, neither}).
//!
//! The async `CheckHostKeyMaterial::run` path (proposal load → horizon
//! projection → ssh into host → publication decode → diff) is not
//! exercised here — those steps are I/O. Only the deterministic
//! `diff` function is witnessed.
//!
//! Spec: `reports/system-assistant/07-criomos-stack-deep-audit.md` §6
//! (missing tests) — "lojix-cli lacks explicit test of
//! `CheckHostKeyMaterial` diff logic (documented in skills.md but not
//! witnessed)".

use std::collections::BTreeMap;

use clavifaber::publication::PublicKeyPublication;
use clavifaber::yggdrasil::YggdrasilProjection;
use horizon_lib::address::{YggAddress, YggSubnet};
use horizon_lib::io::Io;
use horizon_lib::machine::Machine;
use horizon_lib::magnitude::Magnitude;
use horizon_lib::name::{ClusterName, NodeName};
use horizon_lib::proposal::{
    ClusterProposal, ClusterTrust, NodeProposal, NodePubKeys, YggPubKeyEntry,
};
use horizon_lib::pub_key::{SshPubKey, YggPubKey};
use horizon_lib::species::{Arch, Bootloader, Keyboard, MachineSpecies, NodeSpecies};
use horizon_lib::{Horizon, Viewpoint};

use lojix_cli::check::diff;

const SSH_KEY_PRIMARY: &str = "AAAAAAAA";
const SSH_KEY_OTHER: &str = "BBBBBBBB";

fn ygg_key() -> String {
    "a".repeat(64)
}

fn machine() -> Machine {
    Machine {
        species: MachineSpecies::Metal,
        arch: Some(Arch::X86_64),
        cores: 4,
        model: None,
        mother_board: None,
        super_node: None,
        super_user: None,
        chip_gen: None,
        ram_gb: None,
    }
}

fn io() -> Io {
    Io {
        keyboard: Keyboard::Qwerty,
        bootloader: Bootloader::Uefi,
        disks: BTreeMap::new(),
        swap_devices: Vec::new(),
    }
}

fn node_proposal_with_keys(ssh: &str, yggdrasil: Option<YggPubKeyEntry>) -> NodeProposal {
    NodeProposal {
        species: NodeSpecies::Edge,
        size: Magnitude::Min,
        trust: Magnitude::Max,
        machine: machine(),
        io: io(),
        pub_keys: NodePubKeys {
            ssh: SshPubKey::try_new(ssh).unwrap(),
            nix: None,
            yggdrasil,
        },
        link_local_ips: Vec::new(),
        node_ip: None,
        wireguard_pub_key: None,
        nordvpn: false,
        wifi_cert: false,
        wireguard_untrusted_proxies: Vec::new(),
        wants_printing: false,
        wants_hw_video_accel: false,
        router_interfaces: None,
        online: None,
        services: Vec::new(),
    }
}

fn horizon_for(ssh: &str, yggdrasil: Option<YggPubKeyEntry>) -> Horizon {
    let node_name = NodeName::try_new("dune").unwrap();
    let cluster = ClusterName::try_new("fieldlab").unwrap();
    let mut nodes = BTreeMap::new();
    nodes.insert(node_name.clone(), node_proposal_with_keys(ssh, yggdrasil));
    ClusterProposal {
        nodes,
        users: BTreeMap::new(),
        domains: BTreeMap::new(),
        trust: ClusterTrust {
            cluster: Magnitude::Max,
            clusters: BTreeMap::new(),
            nodes: BTreeMap::new(),
            users: BTreeMap::new(),
        },
    }
    .project(&Viewpoint {
        cluster,
        node: node_name,
    })
    .unwrap()
}

fn ssh_line(ssh_base64: &str) -> String {
    format!("ssh-ed25519 {ssh_base64} dune@fieldlab")
}

fn publication(ssh_base64: &str, yggdrasil: Option<YggdrasilProjection>) -> PublicKeyPublication {
    PublicKeyPublication {
        node_name: "dune".to_string(),
        open_ssh_public_key: ssh_line(ssh_base64),
        yggdrasil,
        wifi_client_certificate: None,
    }
}

fn ygg_pub_key_entry() -> YggPubKeyEntry {
    YggPubKeyEntry {
        pub_key: YggPubKey::try_new(ygg_key()).unwrap(),
        address: YggAddress::try_new("200::1").unwrap(),
        subnet: YggSubnet::try_new("300:ca41:6b12:fba").unwrap(),
    }
}

fn ygg_projection_matching() -> YggdrasilProjection {
    YggdrasilProjection {
        address: "200::1".to_string(),
        public_key: ygg_key(),
    }
}

#[test]
fn consistent_when_ssh_matches_and_neither_has_yggdrasil() {
    let horizon = horizon_for(SSH_KEY_PRIMARY, None);
    let pub_ = publication(SSH_KEY_PRIMARY, None);
    let report = diff(&horizon, &pub_);
    assert!(
        report.is_consistent(),
        "expected no mismatches, got: {:?}",
        report.mismatches
    );
}

#[test]
fn mismatch_when_ssh_pubkey_differs() {
    let horizon = horizon_for(SSH_KEY_PRIMARY, None);
    let pub_ = publication(SSH_KEY_OTHER, None);
    let report = diff(&horizon, &pub_);
    assert_eq!(report.mismatches.len(), 1);
    assert_eq!(report.mismatches[0].concern, "ssh-public-key");
    assert_eq!(report.mismatches[0].expected, SSH_KEY_PRIMARY);
    assert_eq!(report.mismatches[0].actual, SSH_KEY_OTHER);
}

#[test]
fn mismatch_when_horizon_expects_yggdrasil_but_publication_has_none() {
    let horizon = horizon_for(SSH_KEY_PRIMARY, Some(ygg_pub_key_entry()));
    let pub_ = publication(SSH_KEY_PRIMARY, None);
    let report = diff(&horizon, &pub_);
    assert_eq!(report.mismatches.len(), 1);
    assert_eq!(report.mismatches[0].concern, "yggdrasil-public-key");
    assert!(
        report.mismatches[0]
            .actual
            .contains("absent in publication")
    );
}

#[test]
fn mismatch_when_publication_has_yggdrasil_not_in_horizon() {
    let horizon = horizon_for(SSH_KEY_PRIMARY, None);
    let pub_ = publication(SSH_KEY_PRIMARY, Some(ygg_projection_matching()));
    let report = diff(&horizon, &pub_);
    assert_eq!(report.mismatches.len(), 1);
    assert_eq!(report.mismatches[0].concern, "yggdrasil-public-key");
    assert!(
        report.mismatches[0]
            .expected
            .contains("absent in goldragon")
    );
}

#[test]
fn mismatch_when_yggdrasil_pubkey_differs() {
    let horizon = horizon_for(SSH_KEY_PRIMARY, Some(ygg_pub_key_entry()));
    let pub_ = publication(
        SSH_KEY_PRIMARY,
        Some(YggdrasilProjection {
            address: "200::1".to_string(),
            public_key: "b".repeat(64),
        }),
    );
    let report = diff(&horizon, &pub_);
    assert_eq!(report.mismatches.len(), 1);
    assert_eq!(report.mismatches[0].concern, "yggdrasil-public-key");
    assert_ne!(report.mismatches[0].expected, report.mismatches[0].actual);
}

#[test]
fn mismatch_when_yggdrasil_address_differs_with_matching_key() {
    let horizon = horizon_for(SSH_KEY_PRIMARY, Some(ygg_pub_key_entry()));
    let pub_ = publication(
        SSH_KEY_PRIMARY,
        Some(YggdrasilProjection {
            address: "200::2".to_string(),
            public_key: ygg_key(),
        }),
    );
    let report = diff(&horizon, &pub_);
    assert_eq!(report.mismatches.len(), 1);
    assert_eq!(report.mismatches[0].concern, "yggdrasil-address");
}

#[test]
fn render_text_is_clean_when_consistent() {
    let horizon = horizon_for(SSH_KEY_PRIMARY, None);
    let pub_ = publication(SSH_KEY_PRIMARY, None);
    let text = diff(&horizon, &pub_).render_text();
    assert!(text.contains("CheckHostKeyMaterial: dune"));
    assert!(text.contains("no mismatches"));
}

#[test]
fn render_text_lists_each_mismatch_with_operator_hint() {
    let horizon = horizon_for(SSH_KEY_PRIMARY, Some(ygg_pub_key_entry()));
    let pub_ = publication(SSH_KEY_OTHER, None);
    let text = diff(&horizon, &pub_).render_text();
    assert!(text.contains("ssh-public-key"));
    assert!(text.contains("yggdrasil-public-key"));
    assert!(text.contains("operator hint:"));
}
