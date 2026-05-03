//! Wire-shape tests for the three pipeline phases. Asserts the
//! exact arguments that hit `nix` / `nix copy` / `ssh` so a future
//! regression on the address derivation or activation command is
//! caught without depending on a live deploy.

use std::path::Path;

use horizon_lib::name::{ClusterName, CriomeDomainName, NodeName, UserName};
use horizon_lib::species::System;

use lojix_cli::activate::{BootEntry, HomeActivation, SystemActivation, SystemProfileLink};
use lojix_cli::build::{BuildLocation, BuildPlan, HomeBuildPlan, HomeMode, NixBuild, SystemAction};
use lojix_cli::cluster::{FlakeRef, OverrideUri, StorePath};
use lojix_cli::copy::ClosureCopy;
use lojix_cli::host::SshTarget;

fn target_for(node: &str, cluster: &str) -> SshTarget {
    let node = NodeName::try_new(node).unwrap();
    let cluster = ClusterName::try_new(cluster).unwrap();
    let domain = CriomeDomainName::for_node(&node, &cluster);
    SshTarget::from_criome_domain(&domain)
}

fn nix_build_arguments_for(plan: BuildPlan, builder: Option<SshTarget>) -> Vec<String> {
    let build = NixBuild {
        flake: FlakeRef::new("github:LiGoldragon/CriomOS/abc123"),
        system: System::X86_64Linux,
        horizon_uri: OverrideUri::from_local_path(Path::new("/cache/horizon")),
        system_uri: OverrideUri::from_local_path(Path::new("/cache/system")),
        deployment_uri: match plan {
            BuildPlan::System { .. } => Some(OverrideUri::from_local_path(Path::new(
                "/cache/deployment/home-on",
            ))),
            BuildPlan::Home { .. } => None,
        },
        plan,
        builder,
    };
    let invocation = build.nix_invocation();
    assert_eq!(invocation.program(), "nix");
    invocation.arguments().to_vec()
}

#[test]
fn nix_build_arguments_contain_target_attr_and_overrides() {
    let arguments = nix_build_arguments_for(BuildPlan::full_os(SystemAction::Boot), None);
    assert_eq!(arguments[0], "build");
    assert!(arguments.contains(&"--refresh".to_string()));
    assert!(arguments.iter().any(|argument| argument.contains(
        "github:LiGoldragon/CriomOS/abc123#nixosConfigurations.target.config.system.build.toplevel"
    )));
    let horizon_index = arguments
        .iter()
        .position(|argument| argument == "horizon")
        .expect("horizon flag");
    assert_eq!(arguments[horizon_index + 1], "path:/cache/horizon");
    let system_index = arguments
        .iter()
        .position(|argument| argument == "system")
        .expect("system flag");
    assert_eq!(arguments[system_index + 1], "path:/cache/system");
    let deployment_index = arguments
        .iter()
        .position(|argument| argument == "deployment")
        .expect("deployment flag");
    assert_eq!(
        arguments[deployment_index + 1],
        "path:/cache/deployment/home-on"
    );
}

#[test]
fn nix_eval_arguments_use_eval_operation_and_derivation_path_attr() {
    let arguments = nix_build_arguments_for(BuildPlan::full_os(SystemAction::Eval), None);
    assert_eq!(arguments[0], "eval");
    assert!(arguments.contains(&"--refresh".to_string()));
    assert!(arguments.contains(&"--raw".to_string()));
    assert!(
        arguments
            .iter()
            .any(|argument| argument.ends_with(".drvPath"))
    );
}

#[test]
fn nix_home_build_arguments_use_home_activation_package_attr() {
    let user = UserName::try_new("li").unwrap();
    let arguments = nix_build_arguments_for(
        BuildPlan::home_only(HomeBuildPlan {
            user,
            mode: HomeMode::Build,
        }),
        None,
    );
    assert_eq!(arguments[0], "build");
    assert!(
        arguments.iter().any(|argument| argument
            .contains("github:LiGoldragon/CriomOS/abc123#packages.x86_64-linux.activationPackage"))
    );
    assert!(
        !arguments.iter().any(|argument| argument == "deployment"),
        "home wrapper must not receive CriomOS deployment override"
    );
}

#[test]
fn os_only_plan_disables_home_in_deployment_shape() {
    let plan = BuildPlan::os_only(SystemAction::Build);
    let shape = plan.deployment_shape();
    assert!(!shape.include_home());
    assert_eq!(shape.cache_name(), "home-off");
    assert!(shape.flake_text().contains("includeHome = false"));
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
    assert!(copy.invocation().is_none(), "no copy when source == target");
}

#[test]
fn closure_copy_dispatcher_to_target_uses_to_only() {
    let target = target_for("zeus", "goldragon");
    let copy = ClosureCopy {
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        source: BuildLocation::Dispatcher,
        target,
    };
    let invocation = copy.invocation().expect("copy needed");
    assert_eq!(invocation.program(), "nix");
    let arguments = invocation.arguments();
    assert_eq!(arguments[0], "copy");
    assert!(!arguments.iter().any(|argument| argument == "--from"));
    let target_index = arguments
        .iter()
        .position(|argument| argument == "--to")
        .expect("--to flag");
    assert_eq!(
        arguments[target_index + 1],
        "ssh-ng://root@zeus.goldragon.criome"
    );
    assert_eq!(arguments.last().unwrap(), "/nix/store/abc-toplevel");
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
    let invocation = copy.invocation().expect("copy needed");
    let arguments = invocation.arguments();
    let source_index = arguments
        .iter()
        .position(|argument| argument == "--from")
        .expect("--from flag");
    assert_eq!(
        arguments[source_index + 1],
        "ssh-ng://root@prometheus.goldragon.criome"
    );
    let target_index = arguments
        .iter()
        .position(|argument| argument == "--to")
        .expect("--to flag");
    assert_eq!(
        arguments[target_index + 1],
        "ssh-ng://root@zeus.goldragon.criome"
    );
}

#[test]
fn activation_boot_includes_profile_set_and_switch_to_configuration() {
    let target = target_for("zeus", "goldragon");
    let activation = SystemActivation {
        target,
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        action: SystemAction::Boot,
    };
    let invocation = activation.ssh_invocation().unwrap();
    assert_eq!(invocation.program(), "ssh");
    let arguments = invocation.arguments();
    assert_eq!(arguments[0], "-o");
    assert_eq!(arguments[1], "BatchMode=yes");
    assert_eq!(arguments[2], "root@zeus.goldragon.criome");
    let remote = &arguments[3];
    assert!(
        remote.contains("nix-env -p /nix/var/nix/profiles/system --set /nix/store/abc-toplevel")
    );
    assert!(remote.contains("/nix/store/abc-toplevel/bin/switch-to-configuration boot"));
}

#[test]
fn activation_test_skips_profile_set() {
    let target = target_for("zeus", "goldragon");
    let activation = SystemActivation {
        target,
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        action: SystemAction::Test,
    };
    let invocation = activation.ssh_invocation().unwrap();
    let arguments = invocation.arguments();
    let remote = &arguments[3];
    assert!(
        !remote.contains("nix-env"),
        "test action must not touch the system profile"
    );
    assert!(remote.contains("/nix/store/abc-toplevel/bin/switch-to-configuration test"));
}

#[test]
fn boot_once_systemd_run_uses_wait_collect_and_oneshot_service_type() {
    let activation = SystemActivation {
        target: target_for("prometheus", "goldragon"),
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        action: SystemAction::BootOnce,
    };
    let unit = "lojix-boot-once-test-fixture";
    let invocation = activation.systemd_run_invocation(unit);
    assert_eq!(invocation.program(), "ssh");
    let arguments = invocation.arguments();
    assert_eq!(arguments[2], "root@prometheus.goldragon.criome");
    let remote = &arguments[3];
    assert!(
        remote.contains("systemd-run"),
        "must use systemd-run; got: {remote}"
    );
    assert!(
        remote.contains(&format!("--unit={unit}")),
        "must pass --unit; got: {remote}"
    );
    assert!(
        remote.contains("--collect"),
        "must use --collect; got: {remote}"
    );
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
        action: SystemAction::BootOnce,
    };
    let script = activation.boot_once_script();
    // OLD captures the *currently-running* generation via bootctl
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
    // boot installs the new generation, then default reverts to OLD and
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
        action: SystemAction::BootOnce,
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
        action: SystemAction::BootOnce,
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
fn boot_once_ssh_invocation_returns_error_directing_caller_to_systemd_run() {
    let activation = SystemActivation {
        target: target_for("prometheus", "goldragon"),
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        action: SystemAction::BootOnce,
    };
    // BootOnce uses systemd_run_invocation, not the simple ssh_invocation;
    // misuse-of-API safeguard.
    assert!(
        activation.ssh_invocation().is_err(),
        "ssh_invocation must refuse for BootOnce"
    );
}

#[test]
fn activation_switch_includes_profile_set() {
    let target = target_for("zeus", "goldragon");
    let activation = SystemActivation {
        target,
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        action: SystemAction::Switch,
    };
    let invocation = activation.ssh_invocation().unwrap();
    let arguments = invocation.arguments();
    let remote = &arguments[3];
    assert!(remote.contains("nix-env -p /nix/var/nix/profiles/system --set"));
    assert!(remote.contains("switch-to-configuration switch"));
}

#[test]
fn home_profile_activation_sets_home_manager_profile() {
    let activation = HomeActivation {
        node: NodeName::try_new("ouranos").unwrap(),
        target: target_for("ouranos", "goldragon"),
        user: UserName::try_new("li").unwrap(),
        store_path: StorePath::try_new("/nix/store/abc-home-manager-generation").unwrap(),
        mode: HomeMode::Profile,
    };
    let invocation = activation.profile_invocation(Path::new("/home/li"));
    assert_eq!(invocation.program(), "nix-env");
    let arguments = invocation.arguments();
    assert_eq!(arguments[0], "-p");
    assert_eq!(
        arguments[1],
        "/home/li/.local/state/nix/profiles/home-manager"
    );
    assert_eq!(arguments[2], "--set");
    assert_eq!(arguments[3], "/nix/store/abc-home-manager-generation");
}

#[test]
fn home_activate_runs_activation_script_from_generation() {
    let activation = HomeActivation {
        node: NodeName::try_new("ouranos").unwrap(),
        target: target_for("ouranos", "goldragon"),
        user: UserName::try_new("li").unwrap(),
        store_path: StorePath::try_new("/nix/store/abc-home-manager-generation").unwrap(),
        mode: HomeMode::Activate,
    };
    let invocation = activation.activate_invocation();
    assert_eq!(
        invocation.program(),
        "/nix/store/abc-home-manager-generation/activate"
    );
    assert!(invocation.arguments().is_empty());
}

#[test]
fn remote_home_profile_runs_as_requested_user_on_target() {
    let activation = HomeActivation {
        node: NodeName::try_new("ouranos").unwrap(),
        target: target_for("ouranos", "goldragon"),
        user: UserName::try_new("li").unwrap(),
        store_path: StorePath::try_new("/nix/store/abc-home-manager-generation").unwrap(),
        mode: HomeMode::Profile,
    };
    let invocation = activation.remote_profile_invocation();
    assert_eq!(invocation.program(), "ssh");
    let arguments = invocation.arguments();
    assert_eq!(arguments[2], "li@ouranos.goldragon.criome");
    assert!(arguments[3].contains("nix-env -p \"$HOME/.local/state/nix/profiles/home-manager\""));
    assert!(arguments[3].contains("--set /nix/store/abc-home-manager-generation"));
}

#[test]
fn remote_home_activate_runs_generation_activate_as_requested_user() {
    let activation = HomeActivation {
        node: NodeName::try_new("ouranos").unwrap(),
        target: target_for("ouranos", "goldragon"),
        user: UserName::try_new("li").unwrap(),
        store_path: StorePath::try_new("/nix/store/abc-home-manager-generation").unwrap(),
        mode: HomeMode::Activate,
    };
    let invocation = activation.remote_activate_invocation();
    assert_eq!(invocation.program(), "ssh");
    let arguments = invocation.arguments();
    assert_eq!(arguments[2], "li@ouranos.goldragon.criome");
    assert_eq!(
        arguments[3],
        "/nix/store/abc-home-manager-generation/activate"
    );
}

#[test]
fn requires_efi_reconcile_only_for_boot_and_switch() {
    let cases = [
        (SystemAction::Eval, false),
        (SystemAction::Build, false),
        (SystemAction::Boot, true),
        (SystemAction::Switch, true),
        (SystemAction::Test, false),
        (SystemAction::BootOnce, false),
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
        action: SystemAction::Boot,
    };
    let invocation = activation.step_readlink_system_profile_invocation();
    assert_eq!(invocation.program(), "ssh");
    let arguments = invocation.arguments();
    assert_eq!(arguments[2], "root@prometheus.goldragon.criome");
    assert_eq!(arguments[3], "readlink /nix/var/nix/profiles/system");
}

#[test]
fn efi_reconcile_set_default_passes_entry_id_through() {
    let activation = SystemActivation {
        target: target_for("prometheus", "goldragon"),
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        action: SystemAction::Boot,
    };
    let entry = BootEntry::new("nixos-generation-33.conf");
    let invocation = activation.step_set_efi_default_invocation(&entry);
    let arguments = invocation.arguments();
    assert_eq!(arguments[3], "bootctl set-default nixos-generation-33.conf");
}

#[test]
fn efi_reconcile_clear_oneshot_uses_empty_string_argument() {
    let activation = SystemActivation {
        target: target_for("prometheus", "goldragon"),
        store_path: StorePath::try_new("/nix/store/abc-toplevel").unwrap(),
        action: SystemAction::Boot,
    };
    let invocation = activation.step_clear_efi_oneshot_invocation();
    let arguments = invocation.arguments();
    // Empty string clears the EFI variable per `bootctl(1)`. The
    // POSIX shell on the remote re-parses `bootctl set-oneshot ''`
    // as three argument tokens with the third being the empty string;
    // bootctl reads that and unsets LoaderEntryOneShot.
    assert_eq!(arguments[3], "bootctl set-oneshot ''");
}

#[test]
fn system_profile_link_extracts_generation_number() {
    assert_eq!(
        SystemProfileLink::try_new("system-33-link")
            .unwrap()
            .generation()
            .unwrap()
            .number(),
        33
    );
    assert_eq!(
        SystemProfileLink::try_new("system-1-link")
            .unwrap()
            .generation()
            .unwrap()
            .number(),
        1
    );
    assert_eq!(
        SystemProfileLink::try_new("system-12345-link")
            .unwrap()
            .generation()
            .unwrap()
            .number(),
        12345
    );
}

#[test]
fn system_profile_link_rejects_malformed_inputs() {
    // Wrong prefix.
    assert!(SystemProfileLink::try_new("home-33-link").is_err());
    // Missing -link suffix.
    assert!(SystemProfileLink::try_new("system-33").is_err());
    // Non-numeric.
    assert!(SystemProfileLink::try_new("system-abc-link").is_err());
    // Empty.
    assert!(SystemProfileLink::try_new("").is_err());
    // Negative is not supported (u64).
    assert!(SystemProfileLink::try_new("system--1-link").is_err());
}
