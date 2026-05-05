//! Builder-validation integration tests. Asserts that a bogus
//! builder in the Nota request fails *before* any nix invocation
//! with a specific, human-actionable error — not a TCP timeout
//! deep in the build phase.

use std::path::PathBuf;
use std::process::Command;

const GOLDRAGON_NOTA: &str = "/home/li/git/goldragon/datom.nota";
const CRIOMOS_PATH: &str = "path:/home/li/git/CriomOS";

fn skip_if_no_datom() -> bool {
    if !PathBuf::from(GOLDRAGON_NOTA).exists() {
        eprintln!("skipping: {GOLDRAGON_NOTA} not present");
        return true;
    }
    false
}

fn eval_request_arguments(node: &str, builder: &str) -> Vec<String> {
    vec![
        "(FullOs".to_string(),
        "goldragon".to_string(),
        node.to_string(),
        format!("\"{GOLDRAGON_NOTA}\""),
        format!("\"{CRIOMOS_PATH}\""),
        "Eval".to_string(),
        format!("{builder})"),
    ]
}

#[test]
fn unknown_builder_fails_with_unknown_builder_error() {
    if skip_if_no_datom() {
        return;
    }
    // tiger is a real cluster member; "definitely-not-a-node" is
    // not. Validation should fail at horizon-resolution time.
    let output = Command::new(env!("CARGO_BIN_EXE_lojix-cli"))
        .args(eval_request_arguments("tiger", "definitely-not-a-node"))
        .output()
        .expect("spawn lojix-cli");

    assert!(
        !output.status.success(),
        "expected nonzero exit on unknown builder; stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found in horizon ex_nodes"),
        "expected UnknownBuilder message, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("definitely-not-a-node"),
        "expected the offending builder name in the error, got: {stderr}"
    );
}

#[test]
fn builder_equals_node_resolves_against_viewpoint_node() {
    if skip_if_no_datom() {
        return;
    }
    // A request with node=zeus and builder=zeus is the
    // "build on the target itself" case (per reports/0033 decision 4). The
    // viewpoint node sits in `horizon.node`, not `horizon.ex_nodes`,
    // so this must resolve there. We only run `eval` so no real
    // ssh occurs before validation. zeus is not a remote Nix builder
    // because it is an edge node without the Nix pubkey needed for
    // nix.sshServe, but target-side builds do not use nix.sshServe.
    let output = Command::new(env!("CARGO_BIN_EXE_lojix-cli"))
        .args(eval_request_arguments("zeus", "zeus"))
        .output()
        .expect("spawn lojix-cli");

    let stderr = String::from_utf8_lossy(&output.stderr);
    // The eval may fail at the actual ssh step (no live prom in
    // CI), but it must NOT fail with UnknownBuilder — that's the
    // bug this regression test guards. InvalidBuilder is also a
    // failure mode we explicitly reject; the only acceptable
    // stderr shapes are success or a downstream ssh/nix failure.
    assert!(
        !stderr.contains("not found in horizon ex_nodes"),
        "must not raise UnknownBuilder when builder == node; got: {stderr}"
    );
    assert!(
        !stderr.contains("not a valid remote Nix builder"),
        "builder == node must not require isRemoteNixBuilder; got: {stderr}"
    );
}

#[test]
fn non_builder_node_fails_with_invalid_builder_error() {
    if skip_if_no_datom() {
        return;
    }
    // balboa is in the cluster but isRemoteNixBuilder=false (Center
    // species, Min size — fails the size>=med && is_fully_trusted
    // gates in horizon-rs's projection). Validation should reject
    // it specifically, not silently fall back.
    let output = Command::new(env!("CARGO_BIN_EXE_lojix-cli"))
        .args(eval_request_arguments("tiger", "balboa"))
        .output()
        .expect("spawn lojix-cli");

    assert!(
        !output.status.success(),
        "expected nonzero exit on non-builder; stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not a valid remote Nix builder"),
        "expected InvalidBuilder message, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("balboa"),
        "expected the offending builder name in the error, got: {stderr}"
    );
}
