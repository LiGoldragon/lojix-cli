use std::path::PathBuf;
use std::process::Command;

const GOLDRAGON_NOTA: &str = "/home/li/git/goldragon/datom.nota";
const CRIOMOS_PATH: &str = "path:/home/li/git/CriomOS";

#[test]
fn eval_goldragon_tiger_runs_pipeline_to_nix() {
    if !PathBuf::from(GOLDRAGON_NOTA).exists() {
        eprintln!("skipping: {GOLDRAGON_NOTA} not present");
        return;
    }

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
            CRIOMOS_PATH,
        ])
        .output()
        .expect("spawn lojix");

    let stderr = String::from_utf8_lossy(&out.stderr);

    let horizon_dir = dirs_cache_lojix().join("horizon").join("goldragon").join("tiger");
    assert!(
        horizon_dir.join("horizon.json").exists(),
        "horizon.json should exist in {horizon_dir:?}; stderr was: {stderr}",
    );
    assert!(
        horizon_dir.join("flake.nix").exists(),
        "flake.nix should exist in {horizon_dir:?}",
    );

    let system_dir = dirs_cache_lojix().join("system").join("x86_64-linux");
    assert!(
        system_dir.join("flake.nix").exists(),
        "system flake.nix should exist in {system_dir:?}",
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "lojix eval should succeed end-to-end; stdout: {stdout} stderr: {stderr}",
    );
    assert!(
        stdout.contains("/nix/store/")
            && stdout.contains("nixos-system-tiger")
            && stdout.contains(".drv"),
        "expected a tiger toplevel drvPath in stdout, got: {stdout:?}",
    );
}

#[test]
fn eval_is_deterministic_across_runs() {
    if !PathBuf::from(GOLDRAGON_NOTA).exists() {
        eprintln!("skipping: {GOLDRAGON_NOTA} not present");
        return;
    }

    let run = || {
        let _ = Command::new(env!("CARGO_BIN_EXE_lojix-cli-v2"))
            .args([
                "eval",
                "--cluster",
                "goldragon",
                "--node",
                "tiger",
                "--source",
                GOLDRAGON_NOTA,
                "--criomos",
                CRIOMOS_PATH,
            ])
            .output()
            .expect("spawn lojix");
        (
            nar_hash_of(&dirs_cache_lojix().join("horizon").join("goldragon").join("tiger")),
            nar_hash_of(&dirs_cache_lojix().join("system").join("x86_64-linux")),
        )
    };

    let (h1, s1) = run();
    let (h2, s2) = run();
    assert_eq!(h1, h2, "horizon artifact should be deterministic");
    assert_eq!(s1, s2, "system artifact should be deterministic");
    assert!(h1.starts_with("sha256-"), "horizon narHash SRI: {h1}");
    assert!(s1.starts_with("sha256-"), "system narHash SRI: {s1}");
}

fn dirs_cache_lojix() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME set");
    PathBuf::from(home).join(".cache/lojix")
}

fn nar_hash_of(dir: &std::path::Path) -> String {
    let out = Command::new("nix")
        .args(["hash", "path", "--type", "sha256", "--sri"])
        .arg(dir)
        .output()
        .expect("spawn nix hash");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}
