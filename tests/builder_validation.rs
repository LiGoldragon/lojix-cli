//! Builder-validation integration tests. Asserts that
//! `--builder <bogus>` fails *before* any nix invocation with a
//! specific, human-actionable error — not a TCP timeout deep in
//! the build phase.

use std::path::PathBuf;
use std::process::Command;

const GOLDRAGON_NOTA: &str = "/home/li/git/goldragon/datom.nota";

fn skip_if_no_datom() -> bool {
    if !PathBuf::from(GOLDRAGON_NOTA).exists() {
        eprintln!("skipping: {GOLDRAGON_NOTA} not present");
        return true;
    }
    false
}

#[test]
fn unknown_builder_fails_with_unknown_builder_error() {
    if skip_if_no_datom() {
        return;
    }
    // tiger is a real cluster member; "definitely-not-a-node" is
    // not. Validation should fail at horizon-resolution time.
    let out = Command::new(env!("CARGO_BIN_EXE_lojix-cli-v2"))
        .args([
            "eval",
            "--cluster",
            "goldragon",
            "--node",
            "tiger",
            "--source",
            GOLDRAGON_NOTA,
            "--criomos",
            "path:/home/li/git/CriomOS",
            "--builder",
            "definitely-not-a-node",
        ])
        .output()
        .expect("spawn lojix-cli-v2");

    assert!(
        !out.status.success(),
        "expected nonzero exit on unknown builder; stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
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
    // `--node prometheus --builder prometheus` is the "build on
    // the target itself" case (per reports/0033 decision 4). The
    // viewpoint node sits in `horizon.node`, not `horizon.ex_nodes`,
    // so this must resolve there. We only run `eval` so no real
    // ssh occurs — successful exit means resolution + projection
    // both worked. prom is `is_builder=true` per goldragon's
    // datom (LargeAiRouter, size=Max, trust=Max, base pubkeys).
    let out = Command::new(env!("CARGO_BIN_EXE_lojix-cli-v2"))
        .args([
            "eval",
            "--cluster",
            "goldragon",
            "--node",
            "prometheus",
            "--source",
            GOLDRAGON_NOTA,
            "--criomos",
            "path:/home/li/git/CriomOS",
            "--builder",
            "prometheus",
        ])
        .output()
        .expect("spawn lojix-cli-v2");

    let stderr = String::from_utf8_lossy(&out.stderr);
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
        !stderr.contains("not a valid builder"),
        "prom must validate as is_builder=true; got: {stderr}"
    );
}

#[test]
fn non_builder_node_fails_with_invalid_builder_error() {
    if skip_if_no_datom() {
        return;
    }
    // balboa is in the cluster but is_builder=false (Center
    // species, Min size — fails the size>=med && is_fully_trusted
    // gates in horizon-rs's projection). Validation should reject
    // it specifically, not silently fall back.
    let out = Command::new(env!("CARGO_BIN_EXE_lojix-cli-v2"))
        .args([
            "eval",
            "--cluster",
            "goldragon",
            "--node",
            "tiger",
            "--source",
            GOLDRAGON_NOTA,
            "--criomos",
            "path:/home/li/git/CriomOS",
            "--builder",
            "balboa",
        ])
        .output()
        .expect("spawn lojix-cli-v2");

    assert!(
        !out.status.success(),
        "expected nonzero exit on non-builder; stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not a valid builder"),
        "expected InvalidBuilder message, got stderr: {stderr}"
    );
    assert!(
        stderr.contains("balboa"),
        "expected the offending builder name in the error, got: {stderr}"
    );
}
