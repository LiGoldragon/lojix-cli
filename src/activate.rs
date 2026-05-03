use std::path::Path;
use std::time::SystemTime;

use horizon_lib::name::{NodeName, UserName};
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};

use crate::build::{HomeMode, SystemAction};
use crate::cluster::StorePath;
use crate::error::{Error, Result};
use crate::host::SshTarget;
use crate::process::{ProcessFailure, ProcessInvocation, ProcessRun, ShellArgument, ShellCommand};

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
    /// Invocation for the simple Boot/Switch/Test path.
    /// Returns an error for `BootOnce` (which uses a different
    /// shape — `systemd_run_invocation`).
    pub fn ssh_invocation(&self) -> Result<ProcessInvocation> {
        let action_word = match self.action {
            SystemAction::Boot => "boot",
            SystemAction::Switch => "switch",
            SystemAction::Test => "test",
            SystemAction::BootOnce => {
                return Err(Error::InvalidSystemActivation {
                    action: self.action,
                    reason: "simple ssh invocation requested for BootOnce",
                });
            }
            other => {
                return Err(Error::InvalidSystemActivation {
                    action: other,
                    reason: "simple ssh invocation requested for non-activating action",
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
        Ok(self
            .target
            .remote_invocation(ShellCommand::from_raw(remote_command)))
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
        // currently-installed latest generation." Reading NEW from
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
             GENERATION=$(echo \"$SYSTEM_LINK\" | sed -E 's/^system-([0-9]+)-link$/\\1/')\n\
             NEW=\"nixos-generation-$GENERATION.conf\"\n\
             [ -f \"/boot/loader/entries/$NEW\" ]\n\
             [ \"$NEW\" != \"$OLD\" ]\n\
             bootctl set-default \"$OLD\"\n\
             bootctl set-oneshot \"$NEW\"\n\
             echo \"boot-once: oneshot=$NEW persistent-default=$OLD (=running generation)\"\n",
        )
    }

    /// Invocation for the BootOnce ssh call. Wraps the
    /// boot-once script in `systemd-run --wait`: ssh holds open
    /// while the unit runs on the target, stdout/stderr stream
    /// back as live feedback, ssh exits with the unit's exit
    /// code. If the ssh dies mid-run, the unit continues to
    /// completion on the target and the deployer recovers via
    /// `ssh root@<target> journalctl -u <unit>.service`.
    pub fn systemd_run_invocation(&self, unit_name: &str) -> ProcessInvocation {
        let remote_command = format!(
            "systemd-run \
             --unit={unit_name} \
             --collect \
             --wait \
             --service-type=oneshot \
             /bin/sh -c {script}",
            script = ShellArgument::new(&self.boot_once_script()).to_command_text(),
        );
        self.target
            .remote_invocation(ShellCommand::from_raw(remote_command))
    }

    /// Whether this action's contract includes claiming ownership
    /// of EFI bootloader vars after activation. `Boot` and
    /// `Switch` write `loader.conf`'s `default` line via
    /// `switch-to-configuration` but don't touch EFI's
    /// `LoaderEntryDefault` — leaving the door open for a stale
    /// value (e.g. from a prior `BootOnce` that set EFI default
    /// to a rollback target) to override the just-installed generation
    /// at next boot. We reconcile by explicitly setting EFI
    /// default to the new generation and clearing any pending one-shot.
    /// `Test` is non-persistent and never touches the bootloader.
    /// `BootOnce` is its own thing.
    pub fn requires_efi_reconcile(&self) -> bool {
        matches!(self.action, SystemAction::Boot | SystemAction::Switch)
    }

    /// Invocation for the ssh that reads
    /// `/nix/var/nix/profiles/system`'s symlink target. Stdout is
    /// captured by the dispatcher, parsed via
    /// [`SystemProfileLink`] to derive the
    /// `nixos-generation-N.conf` entry name.
    pub fn step_readlink_system_profile_invocation(&self) -> ProcessInvocation {
        self.target.remote_invocation(ShellCommand::from_raw(
            "readlink /nix/var/nix/profiles/system",
        ))
    }

    /// Invocation for the ssh that points EFI
    /// `LoaderEntryDefault` at the just-installed generation.
    pub fn step_set_efi_default_invocation(&self, entry: &BootEntry) -> ProcessInvocation {
        self.target
            .remote_invocation(ShellCommand::from_raw(format!(
                "bootctl set-default {}",
                entry.as_str()
            )))
    }

    /// Invocation for the ssh that clears EFI
    /// `LoaderEntryOneShot`. After `--action boot/switch`, any
    /// pending one-shot from a prior `--action boot-once` would
    /// otherwise consume the next reboot before the user lands
    /// on the generation they just `--action boot`'d.
    pub fn step_clear_efi_oneshot_invocation(&self) -> ProcessInvocation {
        self.target
            .remote_invocation(ShellCommand::from_raw("bootctl set-oneshot ''"))
    }

    pub async fn run(&self) -> Result<()> {
        match self.action {
            SystemAction::BootOnce => self.run_boot_once().await,
            _ => self.run_simple().await,
        }
    }

    async fn run_simple(&self) -> Result<()> {
        self.ssh_invocation()?
            .inherit_stdio(ProcessRun::inherit_stderr(ProcessFailure::Ssh))
            .await?;
        if self.requires_efi_reconcile() {
            self.reconcile_efi().await?;
        }
        Ok(())
    }

    async fn reconcile_efi(&self) -> Result<()> {
        let output = self
            .step_readlink_system_profile_invocation()
            .capture_stdout(ProcessRun::inherit_stderr(ProcessFailure::Ssh))
            .await?;
        let link = SystemProfileLink::try_new(output.stdout().trim())?;
        let entry = link.generation()?.boot_entry();
        self.step_set_efi_default_invocation(&entry)
            .inherit_stdio(ProcessRun::inherit_stderr(ProcessFailure::Ssh))
            .await?;
        self.step_clear_efi_oneshot_invocation()
            .inherit_stdio(ProcessRun::inherit_stderr(ProcessFailure::Ssh))
            .await?;
        eprintln!(
            "efi: LoaderEntryDefault={}, LoaderEntryOneShot=(cleared)",
            entry.as_str()
        );
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
        match self
            .systemd_run_invocation(&unit_name)
            .inherit_stdio(ProcessRun::inherit_stderr(ProcessFailure::Ssh))
            .await
        {
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
    pub target: SshTarget,
    pub user: UserName,
    pub store_path: StorePath,
    pub mode: HomeMode,
}

impl HomeActivation {
    pub fn profile_invocation(&self, home: &Path) -> ProcessInvocation {
        ProcessInvocation::new("nix-env").with_arguments([
            "-p".to_string(),
            home.join(".local/state/nix/profiles/home-manager")
                .display()
                .to_string(),
            "--set".to_string(),
            self.store_path.as_str().to_string(),
        ])
    }

    pub fn activate_invocation(&self) -> ProcessInvocation {
        ProcessInvocation::new(format!("{}/activate", self.store_path.as_str()))
    }

    pub fn remote_profile_invocation(&self) -> ProcessInvocation {
        self.user_target()
            .remote_invocation(ShellCommand::from_raw(format!(
                "nix-env -p \"$HOME/.local/state/nix/profiles/home-manager\" --set {}",
                ShellArgument::new(self.store_path.as_str()).to_command_text(),
            )))
    }

    pub fn remote_activate_invocation(&self) -> ProcessInvocation {
        self.user_target()
            .remote_invocation(ShellCommand::from_invocation(&self.activate_invocation()))
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
        if !self.is_local_context().await? {
            return self
                .remote_profile_invocation()
                .inherit_stdio(ProcessRun::inherit_stderr(ProcessFailure::Ssh))
                .await;
        }
        let home = std::env::var("HOME").map_err(|_| Error::NoHome)?;
        self.profile_invocation(Path::new(&home))
            .inherit_stdio(ProcessRun::inherit_stderr(ProcessFailure::Nix))
            .await
    }

    async fn run_activate(&self) -> Result<()> {
        if !self.is_local_context().await? {
            return self
                .remote_activate_invocation()
                .inherit_stdio(ProcessRun::inherit_stderr(ProcessFailure::Ssh))
                .await;
        }
        self.activate_invocation()
            .inherit_stdio(ProcessRun::inherit_stderr(ProcessFailure::Nix))
            .await
    }

    async fn is_local_context(&self) -> Result<bool> {
        Ok(self.current_user()? == self.user.as_str()
            && self.current_node().await? == self.node.as_str())
    }

    fn user_target(&self) -> SshTarget {
        self.target.with_user(&self.user)
    }

    fn current_user(&self) -> Result<String> {
        std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .map_err(|_| Error::NoUser)
    }

    async fn current_node(&self) -> Result<String> {
        let output = ProcessInvocation::new("hostname")
            .with_argument("-s")
            .capture_stdout(ProcessRun::capture_stderr(ProcessFailure::LocalHostname))
            .await?;
        Ok(output.stdout().trim().to_string())
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemProfileLink(String);

impl SystemProfileLink {
    pub fn try_new(link: impl Into<String>) -> Result<Self> {
        let link = link.into();
        let stripped = link
            .strip_prefix("system-")
            .and_then(|rest| rest.strip_suffix("-link"));
        if stripped
            .and_then(|number| number.parse::<u64>().ok())
            .is_some()
        {
            Ok(Self(link))
        } else {
            Err(Error::InvalidSystemProfileLink { got: link })
        }
    }

    pub fn generation(&self) -> Result<SystemGeneration> {
        let number = self
            .0
            .strip_prefix("system-")
            .and_then(|rest| rest.strip_suffix("-link"))
            .and_then(|number| number.parse::<u64>().ok())
            .ok_or_else(|| Error::InvalidSystemProfileLink {
                got: self.0.clone(),
            })?;
        Ok(SystemGeneration(number))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemGeneration(u64);

impl SystemGeneration {
    pub fn new(number: u64) -> Self {
        Self(number)
    }

    pub fn number(self) -> u64 {
        self.0
    }

    pub fn boot_entry(self) -> BootEntry {
        BootEntry(format!("nixos-generation-{}.conf", self.0))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootEntry(String);

impl BootEntry {
    pub fn new(entry: impl Into<String>) -> Self {
        Self(entry.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
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
