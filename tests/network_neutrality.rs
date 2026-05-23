//! Architectural witness: lojix-cli is network-neutral.
//!
//! Per `skills.md` and `ARCHITECTURE.md`, lojix-cli carries no cluster
//! or node identity in source. Addressing comes from
//! `horizon.node.criome_domain_name` after projection; the operator's
//! intent enters as a single Nota request decoded by `src/request.rs`.
//! Literal cluster/node names — `"prometheus"`, `"ouranos"`,
//! `"goldragon"`, `"criome"` — must not appear in `src/`.
//!
//! This test reads every `.rs` file under `src/` and asserts that none
//! of the historical literal names appear. New node/cluster names
//! must not regress as literals — they must enter via
//! `horizon-lib::name::{NodeName, ClusterName, ClusterTld}`.
//!
//! Spec: `reports/system-assistant/07-criomos-stack-deep-audit.md` §6
//! (missing tests) and §P2.4 of the cloud-host plan
//! (literal-name regression scan).

use std::fs;
use std::path::{Path, PathBuf};

/// Historical literal names that must never re-appear in `src/`.
/// Each pair is (literal, explanation).
const FORBIDDEN_LITERALS: &[(&str, &str)] = &[
    (
        "\"prometheus\"",
        "historical node name; use horizon.node.* projection",
    ),
    (
        "\"ouranos\"",
        "historical node name; use horizon.node.* projection",
    ),
    (
        "\"goldragon\"",
        "historical cluster name; use horizon.cluster.name",
    ),
    (
        "\"criome\"",
        "historical cluster TLD; use horizon.cluster.tld (added in horizon-rs primary-a70)",
    ),
];

fn src_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let entry = entry.expect("read dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn line_is_comment_or_empty(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.is_empty() || trimmed.starts_with("//")
}

#[test]
fn no_literal_cluster_or_node_names_in_src() {
    let mut files = Vec::new();
    collect_rs_files(&src_dir(), &mut files);
    assert!(
        !files.is_empty(),
        "expected at least one .rs file under {:?}",
        src_dir()
    );

    let mut violations: Vec<String> = Vec::new();
    for file in &files {
        let content = fs::read_to_string(file).expect("read src file");
        for (lineno, line) in content.lines().enumerate() {
            if line_is_comment_or_empty(line) {
                continue;
            }
            for (literal, explanation) in FORBIDDEN_LITERALS {
                if line.contains(literal) {
                    let rel = file
                        .strip_prefix(env!("CARGO_MANIFEST_DIR"))
                        .unwrap_or(file)
                        .display();
                    violations.push(format!(
                        "{rel}:{}: literal {literal} — {explanation}",
                        lineno + 1
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "network-neutrality violation — {} site(s):\n  {}",
        violations.len(),
        violations.join("\n  ")
    );
}
