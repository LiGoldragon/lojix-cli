//! Substituter resolution tests for `ExtraSubstituters::from_horizon_nodes`.
//!
//! Asserts the cache-endpoint URL preference (yggdrasil over nix-url),
//! the fallback when no yggdrasil address is present, and the typed
//! error shapes for unknown / non-cache nodes.
//!
//! Extracted from the inline `mod tests` in `src/deploy.rs` per
//! `~/primary/skills/rust-discipline.md` §"Tests live in separate
//! files".

use std::collections::BTreeMap;

use horizon_lib::address::{YggAddress, YggSubnet};
use horizon_lib::io::Io;
use horizon_lib::machine::Machine;
use horizon_lib::magnitude::Magnitude;
use horizon_lib::name::{ClusterName, NodeName};
use horizon_lib::proposal::{
    ClusterProposal, ClusterTrust, NodeProposal, NodePubKeys, NodeService, YggPubKeyEntry,
};
use horizon_lib::pub_key::{NixPubKey, SshPubKey, YggPubKey};
use horizon_lib::species::{Arch, Bootloader, Keyboard, MachineSpecies, NodeSpecies};
use horizon_lib::{Horizon, Viewpoint};

use lojix_cli::build::ExtraSubstituters;
use lojix_cli::error::Error;

const NIX_KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

fn node_name(name: &str) -> NodeName {
    NodeName::try_new(name).unwrap()
}

fn cluster_name() -> ClusterName {
    ClusterName::try_new("goldragon").unwrap()
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
        compressed_swap: None,
    }
}

fn pub_keys(nix: bool, ygg: bool) -> NodePubKeys {
    NodePubKeys {
        ssh: SshPubKey::try_new("AAA=").unwrap(),
        nix: nix.then(|| NixPubKey::try_new(NIX_KEY).unwrap()),
        yggdrasil: ygg.then(|| YggPubKeyEntry {
            pub_key: YggPubKey::try_new("a".repeat(64)).unwrap(),
            address: YggAddress::try_new("200::1").unwrap(),
            subnet: YggSubnet::try_new("300:ca41:6b12:fba").unwrap(),
        }),
    }
}

fn node_proposal(species: NodeSpecies, size: Magnitude, nix: bool, ygg: bool) -> NodeProposal {
    NodeProposal {
        species,
        size,
        trust: Magnitude::Max,
        machine: machine(),
        io: io(),
        pub_keys: pub_keys(nix, ygg),
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

fn projected_horizon() -> Horizon {
    let target = node_name("zeus");
    let cache = node_name("prometheus");
    let mut nodes = BTreeMap::new();
    nodes.insert(
        target.clone(),
        node_proposal(NodeSpecies::Edge, Magnitude::Min, false, false),
    );
    let mut cache_proposal = node_proposal(NodeSpecies::Center, Magnitude::Min, true, true);
    cache_proposal.services.push(NodeService::NixCache {});
    nodes.insert(cache, cache_proposal);

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
        cluster: cluster_name(),
        node: target,
    })
    .unwrap()
}

#[test]
fn substituter_resolution_prefers_ygg_endpoint_over_nix_url() {
    let horizon = projected_horizon();
    let substituters =
        ExtraSubstituters::from_horizon_nodes(&horizon, &[node_name("prometheus")]).unwrap();

    assert_eq!(substituters.urls_text(), "http://[200::1]");
    assert_eq!(
        substituters.public_keys_text(),
        "prometheus.goldragon.criome:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    );
}

#[test]
fn substituter_resolution_falls_back_to_nix_url_without_ygg_endpoint() {
    let mut horizon = projected_horizon();
    horizon
        .ex_nodes
        .get_mut(&node_name("prometheus"))
        .unwrap()
        .ygg_address = None;

    let substituters =
        ExtraSubstituters::from_horizon_nodes(&horizon, &[node_name("prometheus")]).unwrap();

    assert_eq!(
        substituters.urls_text(),
        "http://nix.prometheus.goldragon.criome"
    );
}

#[test]
fn unknown_substituter_reports_unknown_substituter() {
    let horizon = projected_horizon();
    let error =
        ExtraSubstituters::from_horizon_nodes(&horizon, &[node_name("missing")]).unwrap_err();

    assert!(
        matches!(error, Error::UnknownSubstituter(ref name) if name.as_str() == "missing"),
        "unexpected error: {error}"
    );
}

#[test]
fn node_without_cache_endpoint_reports_invalid_substituter() {
    let horizon = projected_horizon();
    let error = ExtraSubstituters::from_horizon_nodes(&horizon, &[node_name("zeus")]).unwrap_err();

    assert!(
        matches!(error, Error::InvalidSubstituter(ref name) if name.as_str() == "zeus"),
        "unexpected error: {error}"
    );
}

#[test]
fn cache_endpoint_without_public_key_reports_invalid_substituter() {
    let mut horizon = projected_horizon();
    horizon
        .ex_nodes
        .get_mut(&node_name("prometheus"))
        .unwrap()
        .nix_pub_key_line = None;

    let error =
        ExtraSubstituters::from_horizon_nodes(&horizon, &[node_name("prometheus")]).unwrap_err();

    assert!(
        matches!(error, Error::InvalidSubstituter(ref name) if name.as_str() == "prometheus"),
        "unexpected error: {error}"
    );
}
