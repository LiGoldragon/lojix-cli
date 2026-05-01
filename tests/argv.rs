//! Wire-shape tests for the three pipeline phases. Asserts the
//! exact argv that hits `nix` / `nix copy` / `ssh` so a future
//! regression on the address derivation or activation command is
//! caught without depending on a live deploy.

use std::path::Path;

use horizon_lib::name::{ClusterName, CriomeDomainName, NodeName};

use lojix_cli_v2::activate::{parse_gen_number_from_link, SystemActivation};
use lojix_cli_v2::build::{BuildAction, BuildLocation, NixBuild};
use lojix_cli_v2::cluster::{FlakeRef, OverrideUri, StorePath};
use lojix_cli_v2::copy::ClosureCopy;
use lojix_cli_v2::host::SshTarget;

fn target_for(node: &str, cluster: &str) -> SshTarget {
    let node = NodeName::try_new(node).unwrap();
    let cluster = ClusterName::try_new(cluster).unwrap();
    let domain = CriomeDomainName::for_node(&node, &cluster);
    SshTarget::from_criome_domain(&domain)
}

fn nix_build_argv_for(action: BuildAction, builder: Option<SshTarget>) -> Vec<String> {
    let build = NixBuild {
        flake: FlakeRef::new("github:LiGoldragon/CriomOS/abc123"),
        horizon_uri: OverrideUri::from_local_path(Path::new("/cache/horizon")),
        system_uri: OverrideUri::from_local_path(Path::new("/cache/system")),
        action,
        builder,
    };
    let (program, argv) = build.nix_argv();
    assert_eq!(program, "nix");
    argv
}

#[test]
fn nix_build_argv_contains_target_attr_and_overrides() {
    let argv = nix_build_argv_for(BuildAction::Boot, None);
    assert_eq!(argv[0], "build");
    assert!(argv.iter().any(|a| a.contains(
        "github:LiGoldragon/CriomOS/abc123#nixosConfigurations.target.config.system.build.toplevel"
    )));
    let i = argv.iter().position(|a| a == "horizon").expect("horizon flag");
    assert_eq!(argv[i + 1], "path:/cache/horizon");
    let j = argv.iter().position(|a| a == "system").expect("system flag");
    assert_eq!(argv[j + 1], "path:/cache/system");
}

#[test]
fn nix_eval_argv_uses_eval_subcommand_and_drvpath_attr() {
    let argv = nix_build_argv_for(BuildAction::Eval, None);
    assert_eq!(argv[0], "eval");
    assert!(argv.contains(&"--raw".to_string()));
    assert!(argv.iter().any(|a| a.ends_with(".drvPath")));
}

#[test]
fn closure_copy_skips_when_builder_equals_target() {
    let target = target_for("zeus", "goldragon");
    let same_builder = target_for("zeus", "goldragon");
    let copy = ClosureCopy {
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        source: BuildLocation::Builder(same_builder),
        target,
    };
    assert!(copy.argv().is_none(), "no copy when source == target");
}

#[test]
fn closure_copy_dispatcher_to_target_uses_to_only() {
    let target = target_for("zeus", "goldragon");
    let copy = ClosureCopy {
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        source: BuildLocation::Dispatcher,
        target,
    };
    let (program, argv) = copy.argv().expect("copy needed");
    assert_eq!(program, "nix");
    assert_eq!(argv[0], "copy");
    assert!(!argv.iter().any(|a| a == "--from"));
    let i = argv.iter().position(|a| a == "--to").expect("--to flag");
    assert_eq!(argv[i + 1], "ssh-ng://root@zeus.goldragon.criome");
    assert_eq!(argv.last().unwrap(), "/nix/store/abc-toplevel");
}

#[test]
fn closure_copy_third_party_builder_uses_from_and_to() {
    let target = target_for("zeus", "goldragon");
    let builder = target_for("prometheus", "goldragon");
    let copy = ClosureCopy {
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        source: BuildLocation::Builder(builder),
        target,
    };
    let (_, argv) = copy.argv().expect("copy needed");
    let i = argv.iter().position(|a| a == "--from").expect("--from flag");
    assert_eq!(argv[i + 1], "ssh-ng://root@prometheus.goldragon.criome");
    let j = argv.iter().position(|a| a == "--to").expect("--to flag");
    assert_eq!(argv[j + 1], "ssh-ng://root@zeus.goldragon.criome");
}

#[test]
fn activation_boot_includes_profile_set_and_switch_to_configuration() {
    let target = target_for("zeus", "goldragon");
    let activation = SystemActivation {
        target,
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        action: BuildAction::Boot,
    };
    let (program, argv) = activation.ssh_argv().unwrap();
    assert_eq!(program, "ssh");
    assert_eq!(argv[0], "-o");
    assert_eq!(argv[1], "BatchMode=yes");
    assert_eq!(argv[2], "root@zeus.goldragon.criome");
    let remote = &argv[3];
    assert!(remote.contains("nix-env -p /nix/var/nix/profiles/system --set /nix/store/abc-toplevel"));
    assert!(remote.contains("/nix/store/abc-toplevel/bin/switch-to-configuration boot"));
}

#[test]
fn activation_test_skips_profile_set() {
    let target = target_for("zeus", "goldragon");
    let activation = SystemActivation {
        target,
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        action: BuildAction::Test,
    };
    let (_, argv) = activation.ssh_argv().unwrap();
    let remote = &argv[3];
    assert!(!remote.contains("nix-env"), "test action must not touch the system profile");
    assert!(remote.contains("/nix/store/abc-toplevel/bin/switch-to-configuration test"));
}

#[test]
fn boot_once_systemd_run_uses_wait_collect_and_oneshot_service_type() {
    let activation = SystemActivation {
        target: target_for("prometheus", "goldragon"),
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        action: BuildAction::BootOnce,
    };
    let unit = "lojix-boot-once-test-fixture";
    let (program, argv) = activation.systemd_run_argv(unit);
    assert_eq!(program, "ssh");
    assert_eq!(argv[2], "root@prometheus.goldragon.criome");
    let remote = &argv[3];
    assert!(remote.contains("systemd-run"), "must use systemd-run; got: {remote}");
    assert!(remote.contains(&format!("--unit={unit}")), "must pass --unit; got: {remote}");
    assert!(remote.contains("--collect"), "must use --collect; got: {remote}");
    assert!(
        remote.contains("--wait"),
        "must use --wait so ssh holds open for live feedback + returns the unit's exit code; \
         got: {remote}"
    );
    assert!(
        remote.contains("--service-type=oneshot"),
        "must declare oneshot service type; got: {remote}"
    );
}

#[test]
fn boot_once_script_uses_current_entry_as_rollback_target() {
    let activation = SystemActivation {
        target: target_for("prometheus", "goldragon"),
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        action: BuildAction::BootOnce,
    };
    let script = activation.boot_once_script();
    // OLD captures the *currently-running* gen via bootctl
    // status's Current Entry field — not /boot/loader/loader.conf's
    // default line, which can hold a stale "next intended boot"
    // from an earlier nixos-rebuild boot.
    assert!(
        script.contains("OLD=$(bootctl status | awk -F': *' '/Current Entry:/ {print $2}')"),
        "OLD must come from bootctl status's Current Entry; got:\n{script}"
    );
    assert!(
        !script.contains("/boot/loader/loader.conf"),
        "must not read loader.conf default (stale-state hazard); got:\n{script}"
    );
    // Closure pinned by absolute path, switch-to-configuration
    // boot installs the new gen, then default reverts to OLD and
    // oneshot is armed to NEW.
    assert!(script.contains("CLOSURE='/nix/store/abc-toplevel'"));
    assert!(script.contains("\"$CLOSURE/bin/switch-to-configuration\" boot"));
    assert!(script.contains("bootctl set-default \"$OLD\""));
    assert!(script.contains("bootctl set-oneshot \"$NEW\""));
}

#[test]
fn boot_once_script_derives_new_from_system_profile_link_not_efi_var() {
    let activation = SystemActivation {
        target: target_for("prometheus", "goldragon"),
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        action: BuildAction::BootOnce,
    };
    let script = activation.boot_once_script();
    // NEW must come from /nix/var/nix/profiles/system's target
    // (system-N-link → nixos-generation-N.conf), not from
    // bootctl's Default Entry which reflects EFI vars and can
    // be stale on same-closure redeploys.
    assert!(
        script.contains("readlink /nix/var/nix/profiles/system"),
        "NEW must derive from the system profile symlink; got:\n{script}"
    );
    assert!(
        !script.contains("'/Default Entry:/"),
        "must not read Default Entry from bootctl status (stale-EFI hazard); got:\n{script}"
    );
    // Sanity-check: refuse if the entry file isn't actually on disk.
    assert!(
        script.contains("[ -f \"/boot/loader/entries/$NEW\" ]"),
        "must verify the bootloader entry exists before set-oneshot; got:\n{script}"
    );
}

#[test]
fn boot_once_script_seeds_path_for_systemd_transient_unit() {
    let activation = SystemActivation {
        target: target_for("prometheus", "goldragon"),
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        action: BuildAction::BootOnce,
    };
    let script = activation.boot_once_script();
    // systemd transient units inherit a minimal PATH that
    // excludes NixOS's system bin dir; explicit seeding is
    // required for awk/nix-env/bootctl to resolve.
    assert!(
        script.contains("export PATH=/run/current-system/sw/bin:/run/wrappers/bin:$PATH"),
        "must seed PATH for the systemd transient unit; got:\n{script}"
    );
}

#[test]
fn boot_once_ssh_argv_returns_error_directing_caller_to_systemd_run() {
    let activation = SystemActivation {
        target: target_for("prometheus", "goldragon"),
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        action: BuildAction::BootOnce,
    };
    // BootOnce uses systemd_run_argv, not the simple ssh_argv;
    // misuse-of-API safeguard.
    assert!(activation.ssh_argv().is_err(), "ssh_argv must refuse for BootOnce");
}

#[test]
fn activation_switch_includes_profile_set() {
    let target = target_for("zeus", "goldragon");
    let activation = SystemActivation {
        target,
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        action: BuildAction::Switch,
    };
    let (_, argv) = activation.ssh_argv().unwrap();
    let remote = &argv[3];
    assert!(remote.contains("nix-env -p /nix/var/nix/profiles/system --set"));
    assert!(remote.contains("switch-to-configuration switch"));
}

#[test]
fn requires_efi_reconcile_only_for_boot_and_switch() {
    let cases = [
        (BuildAction::Eval, false),
        (BuildAction::Build, false),
        (BuildAction::Boot, true),
        (BuildAction::Switch, true),
        (BuildAction::Test, false),
        (BuildAction::BootOnce, false),
    ];
    for (action, want) in cases {
        let activation = SystemActivation {
            target: target_for("prometheus", "goldragon"),
            store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
            action,
        };
        assert_eq!(
            activation.requires_efi_reconcile(),
            want,
            "{action:?} reconcile expectation mismatch",
        );
    }
}

#[test]
fn efi_reconcile_readlink_targets_system_profile() {
    let activation = SystemActivation {
        target: target_for("prometheus", "goldragon"),
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        action: BuildAction::Boot,
    };
    let (program, argv) = activation.step_readlink_system_profile();
    assert_eq!(program, "ssh");
    assert_eq!(argv[2], "root@prometheus.goldragon.criome");
    assert_eq!(argv[3], "readlink /nix/var/nix/profiles/system");
}

#[test]
fn efi_reconcile_set_default_passes_entry_id_through() {
    let activation = SystemActivation {
        target: target_for("prometheus", "goldragon"),
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        action: BuildAction::Boot,
    };
    let (_, argv) = activation.step_set_efi_default("nixos-generation-33.conf");
    assert_eq!(argv[3], "bootctl set-default nixos-generation-33.conf");
}

#[test]
fn efi_reconcile_clear_oneshot_uses_empty_string_argument() {
    let activation = SystemActivation {
        target: target_for("prometheus", "goldragon"),
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        action: BuildAction::Boot,
    };
    let (_, argv) = activation.step_clear_efi_oneshot();
    // Empty string clears the EFI variable per `bootctl(1)`. The
    // POSIX shell on the remote re-parses `bootctl set-oneshot ''`
    // as three argv tokens with the third being the empty string;
    // bootctl reads that and unsets LoaderEntryOneShot.
    assert_eq!(argv[3], "bootctl set-oneshot ''");
}

#[test]
fn parse_gen_number_from_link_extracts_n_from_system_n_link() {
    assert_eq!(parse_gen_number_from_link("system-33-link").unwrap(), 33);
    assert_eq!(parse_gen_number_from_link("system-1-link").unwrap(), 1);
    assert_eq!(
        parse_gen_number_from_link("system-12345-link").unwrap(),
        12345
    );
}

#[test]
fn parse_gen_number_from_link_rejects_malformed_inputs() {
    // Wrong prefix.
    assert!(parse_gen_number_from_link("home-33-link").is_err());
    // Missing -link suffix.
    assert!(parse_gen_number_from_link("system-33").is_err());
    // Non-numeric.
    assert!(parse_gen_number_from_link("system-abc-link").is_err());
    // Empty.
    assert!(parse_gen_number_from_link("").is_err());
    // Negative is not supported (u64).
    assert!(parse_gen_number_from_link("system--1-link").is_err());
}
