use std::path::Path;
use std::process::Stdio;
use std::time::SystemTime;

use horizon_lib::name::{NodeName, UserName};
use process_wrap::tokio::*;
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::build::{HomeMode, SystemAction};
use crate::cluster::StorePath;
use crate::error::{Error, Result};
use crate::host::SshTarget;

/// Activate the new closure on the target node.
///
/// `Boot`/`Switch`/`Test`: one ssh call running
/// `switch-to-configuration <action>` directly. Synchronous.
///
/// `BootOnce`: one ssh call wrapping the boot-once script in
/// `systemd-run --wait --unit=<name> --collect /bin/sh -c '…'`.
/// The unit runs on the target as a transient systemd service —
/// owned by PID 1, not by the dispatcher's ssh — so a network
/// blip that kills the ssh leaves the unit running on the target
/// to completion. ssh holds open during normal operation as a
/// live-feedback channel (the unit's stdout/stderr stream over
/// it); when the unit terminates, `--wait` returns the unit's
/// exit code through the ssh channel. If the ssh dies before
/// the unit terminates, the dispatcher loses sight but the unit
/// finishes regardless; the deployer re-attaches manually with
/// `ssh root@<target> journalctl -u <unit>` to inspect the
/// outcome.
pub struct SystemActivation {
    pub target: SshTarget,
    pub store_path: StorePath,
    pub action: SystemAction,
}

impl SystemActivation {
    /// (program, argv) for the simple Boot/Switch/Test path.
    /// Returns an error for `BootOnce` (which uses a different
    /// shape — `systemd_run_argv`).
    pub fn ssh_argv(&self) -> Result<(&'static str, Vec<String>)> {
        let action_word = match self.action {
            SystemAction::Boot => "boot",
            SystemAction::Switch => "switch",
            SystemAction::Test => "test",
            SystemAction::BootOnce => {
                return Err(Error::NixFailed {
                    status: -1,
                    stderr: "ssh_argv called for BootOnce; use systemd_run_argv".into(),
                });
            }
            other => {
                return Err(Error::NixFailed {
                    status: -1,
                    stderr: format!("activator invoked with non-activating action {other:?}"),
                });
            }
        };
        let store = self.store_path.as_str();
        let remote_command = if matches!(self.action, SystemAction::Test) {
            format!("{store}/bin/switch-to-configuration {action_word}")
        } else {
            format!(
                "nix-env -p /nix/var/nix/profiles/system --set {store} \
                 && {store}/bin/switch-to-configuration {action_word}"
            )
        };
        Ok((
            "ssh",
            vec![
                "-o".to_string(),
                "BatchMode=yes".to_string(),
                self.target.as_ssh_arg().to_string(),
                remote_command,
            ],
        ))
    }

    /// Unique transient unit name for this deploy. Includes a
    /// time + pid suffix so concurrent deploys don't collide and
    /// so the deployer can grep the right one in the journal
    /// after a disconnect.
    pub fn unit_name(&self) -> String {
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let pid = std::process::id();
        format!("lojix-boot-once-{secs:x}-{pid:x}")
    }

    /// The bash script that runs inside the transient unit on the
    /// target. The `OLD` rollback target is read from `bootctl
    /// status`'s `Current Entry` (= `LoaderEntrySelected` EFI
    /// variable, written by systemd-boot at OS entry to the entry
    /// that booted the running OS) — *not* from
    /// `/boot/loader/loader.conf`'s `default` line, which can
    /// hold a stale "next intended boot" set by an earlier
    /// `nixos-rebuild boot` that hasn't been rebooted into.
    pub fn boot_once_script(&self) -> String {
        let store = self.store_path.as_str();
        // systemd transient units inherit a minimal PATH that
        // does not include `/run/current-system/sw/bin` on NixOS,
        // so `awk`, `nix-env`, `bootctl` resolve to "command not
        // found" without an explicit seed. Setting it inside the
        // script (rather than via systemd-run --setenv=) keeps
        // the override + body co-located.
        // NEW is derived from `/nix/var/nix/profiles/system`'s
        // target — the canonical source of truth for "the
        // currently-installed latest gen." Reading NEW from
        // `bootctl status`'s `Default Entry` is unreliable: it
        // reflects the EFI `LoaderEntryDefault` variable, which
        // can hold a stale value from a prior `bootctl
        // set-default` (e.g. our own previous boot-once run),
        // and doesn't move when `switch-to-configuration boot`
        // is a no-op (same-closure redeploy).
        format!(
            "export PATH=/run/current-system/sw/bin:/run/wrappers/bin:$PATH\n\
             set -eu\n\
             CLOSURE='{store}'\n\
             OLD=$(bootctl status | awk -F': *' '/Current Entry:/ {{print $2}}')\n\
             [ -n \"$OLD\" ]\n\
             nix-env -p /nix/var/nix/profiles/system --set \"$CLOSURE\"\n\
             \"$CLOSURE/bin/switch-to-configuration\" boot\n\
             SYSTEM_LINK=$(readlink /nix/var/nix/profiles/system)\n\
             GEN=$(echo \"$SYSTEM_LINK\" | sed -E 's/^system-([0-9]+)-link$/\\1/')\n\
             NEW=\"nixos-generation-$GEN.conf\"\n\
             [ -f \"/boot/loader/entries/$NEW\" ]\n\
             [ \"$NEW\" != \"$OLD\" ]\n\
             bootctl set-default \"$OLD\"\n\
             bootctl set-oneshot \"$NEW\"\n\
             echo \"boot-once: oneshot=$NEW persistent-default=$OLD (=running gen)\"\n",
        )
    }

    /// (program, argv) for the BootOnce ssh call. Wraps the
    /// boot-once script in `systemd-run --wait`: ssh holds open
    /// while the unit runs on the target, stdout/stderr stream
    /// back as live feedback, ssh exits with the unit's exit
    /// code. If the ssh dies mid-run, the unit continues to
    /// completion on the target and the deployer recovers via
    /// `ssh root@<target> journalctl -u <unit>.service`.
    pub fn systemd_run_argv(&self, unit_name: &str) -> (&'static str, Vec<String>) {
        let remote_command = format!(
            "systemd-run \
             --unit={unit_name} \
             --collect \
             --wait \
             --service-type=oneshot \
             /bin/sh -c {script}",
            script = shell_single_quote(&self.boot_once_script()),
        );
        (
            "ssh",
            vec![
                "-o".to_string(),
                "BatchMode=yes".to_string(),
                self.target.as_ssh_arg().to_string(),
                remote_command,
            ],
        )
    }

    /// Whether this action's contract includes claiming ownership
    /// of EFI bootloader vars after activation. `Boot` and
    /// `Switch` write `loader.conf`'s `default` line via
    /// `switch-to-configuration` but don't touch EFI's
    /// `LoaderEntryDefault` — leaving the door open for a stale
    /// value (e.g. from a prior `BootOnce` that set EFI default
    /// to a rollback target) to override the just-installed gen
    /// at next boot. We reconcile by explicitly setting EFI
    /// default to the new gen and clearing any pending one-shot.
    /// `Test` is non-persistent and never touches the bootloader.
    /// `BootOnce` is its own thing.
    pub fn requires_efi_reconcile(&self) -> bool {
        matches!(self.action, SystemAction::Boot | SystemAction::Switch)
    }

    /// (program, argv) for the ssh that reads
    /// `/nix/var/nix/profiles/system`'s symlink target. Stdout is
    /// captured by the dispatcher, parsed via
    /// [`parse_gen_number_from_link`] to derive the
    /// `nixos-generation-N.conf` entry name.
    pub fn step_readlink_system_profile(&self) -> (&'static str, Vec<String>) {
        (
            "ssh",
            vec![
                "-o".to_string(),
                "BatchMode=yes".to_string(),
                self.target.as_ssh_arg().to_string(),
                "readlink /nix/var/nix/profiles/system".to_string(),
            ],
        )
    }

    /// (program, argv) for the ssh that points EFI
    /// `LoaderEntryDefault` at the just-installed gen.
    pub fn step_set_efi_default(&self, entry: &str) -> (&'static str, Vec<String>) {
        (
            "ssh",
            vec![
                "-o".to_string(),
                "BatchMode=yes".to_string(),
                self.target.as_ssh_arg().to_string(),
                format!("bootctl set-default {entry}"),
            ],
        )
    }

    /// (program, argv) for the ssh that clears EFI
    /// `LoaderEntryOneShot`. After `--action boot/switch`, any
    /// pending one-shot from a prior `--action boot-once` would
    /// otherwise consume the next reboot before the user lands
    /// on the gen they just `--action boot`'d.
    pub fn step_clear_efi_oneshot(&self) -> (&'static str, Vec<String>) {
        (
            "ssh",
            vec![
                "-o".to_string(),
                "BatchMode=yes".to_string(),
                self.target.as_ssh_arg().to_string(),
                "bootctl set-oneshot ''".to_string(),
            ],
        )
    }

    pub async fn run(&self) -> Result<()> {
        match self.action {
            SystemAction::BootOnce => self.run_boot_once().await,
            _ => self.run_simple().await,
        }
    }

    async fn run_simple(&self) -> Result<()> {
        let (program, argv) = self.ssh_argv()?;
        run_ssh_inherit(program, &argv, "switch-to-configuration").await?;
        if self.requires_efi_reconcile() {
            self.reconcile_efi().await?;
        }
        Ok(())
    }

    async fn reconcile_efi(&self) -> Result<()> {
        let (program, argv) = self.step_readlink_system_profile();
        let link = run_ssh_capture(program, &argv, "read system profile").await?;
        let generation = parse_gen_number_from_link(link.trim())?;
        let entry = format!("nixos-generation-{generation}.conf");
        let (program, argv) = self.step_set_efi_default(&entry);
        run_ssh_inherit(program, &argv, "set EFI default").await?;
        let (program, argv) = self.step_clear_efi_oneshot();
        run_ssh_inherit(program, &argv, "clear EFI oneshot").await?;
        eprintln!("efi: LoaderEntryDefault={entry}, LoaderEntryOneShot=(cleared)");
        Ok(())
    }

    async fn run_boot_once(&self) -> Result<()> {
        let unit_name = self.unit_name();
        eprintln!(
            "boot-once: dispatching as transient unit {unit_name}.service on {target}",
            target = self.target.as_ssh_arg(),
        );
        eprintln!(
            "boot-once: ssh holds open for live feedback; if it drops the unit \
             keeps running — re-attach with: ssh {target} journalctl -u {unit_name}.service",
            target = self.target.as_ssh_arg(),
        );
        let (program, argv) = self.systemd_run_argv(&unit_name);
        match run_ssh_inherit(program, &argv, "boot-once unit").await {
            Ok(()) => Ok(()),
            Err(error) => {
                eprintln!(
                    "boot-once: ssh exited with error — the unit {unit_name}.service \
                     may still be running on {target}; re-check with: \
                     ssh {target} systemctl status {unit_name}.service",
                    target = self.target.as_ssh_arg(),
                );
                Err(error)
            }
        }
    }
}

pub struct HomeActivation {
    pub node: NodeName,
    pub user: UserName,
    pub store_path: StorePath,
    pub mode: HomeMode,
}

impl HomeActivation {
    pub fn profile_argv(&self, home: &Path) -> (&'static str, Vec<String>) {
        (
            "nix-env",
            vec![
                "-p".to_string(),
                home.join(".local/state/nix/profiles/home-manager")
                    .display()
                    .to_string(),
                "--set".to_string(),
                self.store_path.as_str().to_string(),
            ],
        )
    }

    pub fn activate_argv(&self) -> (String, Vec<String>) {
        (format!("{}/activate", self.store_path.as_str()), Vec::new())
    }

    pub async fn run(&self) -> Result<()> {
        match self.mode {
            HomeMode::Build => Ok(()),
            HomeMode::Profile => self.run_profile().await,
            HomeMode::Activate => {
                self.run_profile().await?;
                self.run_activate().await
            }
        }
    }

    async fn run_profile(&self) -> Result<()> {
        self.require_local_context().await?;
        let home = std::env::var("HOME").map_err(|_| Error::NoHome)?;
        let (program, argv) = self.profile_argv(Path::new(&home));
        run_local_inherit(program, &argv, "home profile").await
    }

    async fn run_activate(&self) -> Result<()> {
        self.require_local_context().await?;
        let (program, argv) = self.activate_argv();
        run_local_inherit(&program, &argv, "home activate").await
    }

    async fn require_local_context(&self) -> Result<()> {
        let current_user = self.current_user()?;
        if current_user != self.user.as_str() {
            return Err(Error::LocalHomeUserMismatch {
                requested: self.user.clone(),
                actual: current_user,
            });
        }

        let current_node = self.current_node().await?;
        if current_node != self.node.as_str() {
            return Err(Error::LocalHomeNodeMismatch {
                requested: self.node.clone(),
                actual: current_node,
            });
        }

        Ok(())
    }

    fn current_user(&self) -> Result<String> {
        std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .map_err(|_| Error::NoUser)
    }

    async fn current_node(&self) -> Result<String> {
        let output = Command::new("hostname").arg("-s").output().await?;
        if !output.status.success() {
            return Err(Error::SshFailed {
                status: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

pub enum Activation {
    System(SystemActivation),
    Home(HomeActivation),
}

impl Activation {
    pub async fn run(&self) -> Result<()> {
        match self {
            Self::System(activation) => activation.run().await,
            Self::Home(activation) => activation.run().await,
        }
    }
}

/// Wrap `s` in single quotes for safe inclusion in a POSIX shell
/// command line, escaping any embedded single quotes via the
/// `'\''` idiom.
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Extract the integer N from a `system-N-link` string — the shape
/// `/nix/var/nix/profiles/system` resolves to via `readlink`. The
/// caller composes `nixos-generation-{N}.conf` from the result.
pub fn parse_gen_number_from_link(link: &str) -> Result<u64> {
    let stripped = link
        .strip_prefix("system-")
        .and_then(|rest| rest.strip_suffix("-link"));
    match stripped.and_then(|s| s.parse::<u64>().ok()) {
        Some(n) => Ok(n),
        None => Err(Error::SshFailed {
            status: -1,
            stderr: format!(
                "expected /nix/var/nix/profiles/system to symlink to \
                 system-<N>-link; got {link:?}"
            ),
        }),
    }
}

async fn run_ssh_capture(program: &str, argv: &[String], label: &str) -> Result<String> {
    let mut wrap = CommandWrap::with_new(program, |c: &mut Command| {
        c.args(argv)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
    });
    wrap.wrap(ProcessGroup::leader());
    wrap.wrap(KillOnDrop);
    let mut child = wrap.spawn()?;
    let mut stdout = String::new();
    if let Some(mut s) = child.stdout().take() {
        s.read_to_string(&mut stdout).await?;
    }
    let status = child.wait().await?;
    if !status.success() {
        return Err(Error::SshFailed {
            status: status.code().unwrap_or(-1),
            stderr: format!("{label}: ssh non-zero exit"),
        });
    }
    Ok(stdout)
}

async fn run_ssh_inherit(program: &str, argv: &[String], label: &str) -> Result<()> {
    let mut wrap = CommandWrap::with_new(program, |c: &mut Command| {
        c.args(argv)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
    });
    wrap.wrap(ProcessGroup::leader());
    wrap.wrap(KillOnDrop);
    let mut child = wrap.spawn()?;
    let status = child.wait().await?;
    if !status.success() {
        return Err(Error::SshFailed {
            status: status.code().unwrap_or(-1),
            stderr: format!("{label}: ssh non-zero exit (see streamed output)"),
        });
    }
    Ok(())
}

async fn run_local_inherit(program: &str, argv: &[String], label: &str) -> Result<()> {
    let mut wrap = CommandWrap::with_new(program, |c: &mut Command| {
        c.args(argv)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
    });
    wrap.wrap(ProcessGroup::leader());
    wrap.wrap(KillOnDrop);
    let mut child = wrap.spawn()?;
    let status = child.wait().await?;
    if !status.success() {
        return Err(Error::NixFailed {
            status: status.code().unwrap_or(-1),
            stderr: format!("{label}: non-zero exit (see streamed output)"),
        });
    }
    Ok(())
}

pub struct Activator;

pub enum ActivateMsg {
    Run {
        activation: Activation,
        reply: RpcReplyPort<Result<()>>,
    },
}

#[ractor::async_trait]
impl Actor for Activator {
    type Msg = ActivateMsg;
    type State = ();
    type Arguments = ();

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        _args: (),
    ) -> std::result::Result<Self::State, ActorProcessingErr> {
        Ok(())
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        msg: Self::Msg,
        _state: &mut Self::State,
    ) -> std::result::Result<(), ActorProcessingErr> {
        match msg {
            ActivateMsg::Run { activation, reply } => {
                let _ = reply.send(activation.run().await);
            }
        }
        Ok(())
    }
}
