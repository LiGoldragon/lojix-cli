use std::process::Stdio;

use process_wrap::tokio::*;
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use horizon_lib::name::UserName;

use crate::cluster::{FlakeRef, OverrideUri, StorePath};
use crate::error::{Error, Result};
use crate::host::SshTarget;

#[derive(Copy, Clone, Debug, PartialEq, Eq, nota_codec::NotaEnum)]
pub enum SystemAction {
    Eval,
    Build,
    Boot,
    Switch,
    Test,
    /// Install the new generation's bootloader entry, but keep the
    /// persistent default pointing at the *current* gen and set
    /// the new gen as a one-shot. Reboot 1 lands the new gen;
    /// reboot 2 (and every subsequent boot) returns to the old
    /// gen automatically. Designed for headless boxes where a
    /// permanent-default boot of an unverified gen is unsafe.
    BootOnce,
}

impl SystemAction {
    pub fn produces_closure(self) -> bool {
        !matches!(self, SystemAction::Eval)
    }

    pub fn activates(self) -> bool {
        matches!(
            self,
            SystemAction::Boot | SystemAction::Switch | SystemAction::Test | SystemAction::BootOnce,
        )
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, nota_codec::NotaEnum)]
pub enum HomeMode {
    Build,
    Profile,
    Activate,
}

impl HomeMode {
    pub fn activates(self) -> bool {
        matches!(self, HomeMode::Profile | HomeMode::Activate)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SystemKind {
    FullOs,
    OsOnly,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DeploymentShape {
    include_home: bool,
}

impl DeploymentShape {
    pub fn home_enabled() -> Self {
        Self { include_home: true }
    }

    pub fn home_disabled() -> Self {
        Self {
            include_home: false,
        }
    }

    pub fn include_home(self) -> bool {
        self.include_home
    }

    pub fn cache_name(self) -> &'static str {
        if self.include_home {
            "home-on"
        } else {
            "home-off"
        }
    }

    pub fn flake_text(self) -> &'static str {
        if self.include_home {
            "{\n  outputs = _: {\n    deployment = {\n      includeHome = true;\n    };\n  };\n}\n"
        } else {
            "{\n  outputs = _: {\n    deployment = {\n      includeHome = false;\n    };\n  };\n}\n"
        }
    }
}

impl SystemKind {
    pub fn deployment_shape(self) -> DeploymentShape {
        match self {
            Self::FullOs => DeploymentShape::home_enabled(),
            Self::OsOnly => DeploymentShape::home_disabled(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildPlan {
    System {
        kind: SystemKind,
        action: SystemAction,
    },
    Home {
        user: UserName,
        mode: HomeMode,
    },
}

impl BuildPlan {
    pub fn full_os(action: SystemAction) -> Self {
        Self::System {
            kind: SystemKind::FullOs,
            action,
        }
    }

    pub fn os_only(action: SystemAction) -> Self {
        Self::System {
            kind: SystemKind::OsOnly,
            action,
        }
    }

    pub fn home_only(user: UserName, mode: HomeMode) -> Self {
        Self::Home { user, mode }
    }

    pub fn deployment_shape(&self) -> DeploymentShape {
        match self {
            Self::System { kind, .. } => kind.deployment_shape(),
            Self::Home { .. } => DeploymentShape::home_enabled(),
        }
    }

    pub fn system_action(&self) -> Option<SystemAction> {
        match self {
            Self::System { action, .. } => Some(*action),
            Self::Home { .. } => None,
        }
    }

    pub fn home_mode(&self) -> Option<HomeMode> {
        match self {
            Self::System { .. } => None,
            Self::Home { mode, .. } => Some(*mode),
        }
    }

    pub fn home_user(&self) -> Option<&UserName> {
        match self {
            Self::System { .. } => None,
            Self::Home { user, .. } => Some(user),
        }
    }

    pub fn supports_remote_builder(&self) -> bool {
        matches!(self, Self::System { .. })
    }

    fn nix_operation(&self) -> NixOperation {
        match self {
            Self::System {
                action: SystemAction::Eval,
                ..
            } => NixOperation::EvalDrvPath,
            Self::System { .. } | Self::Home { .. } => NixOperation::BuildClosure,
        }
    }

    fn target_attr(&self, flake: &FlakeRef) -> String {
        match self {
            Self::System { .. } => format!(
                "{}#nixosConfigurations.target.config.system.build.toplevel",
                flake.as_str(),
            ),
            Self::Home { user, .. } => format!(
                "{}#nixosConfigurations.target.config.home-manager.users.{}.home.activationPackage",
                flake.as_str(),
                user.as_str(),
            ),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum NixOperation {
    EvalDrvPath,
    BuildClosure,
}

/// Where the build phase's closure landed. Drives the copy phase:
/// `Dispatcher` → `nix copy --to`; `Builder(t)` → `nix copy --from
/// <t> --to <target>` (or skip when builder == target).
#[derive(Debug, Clone)]
pub enum BuildLocation {
    Dispatcher,
    Builder(SshTarget),
}

#[derive(Debug)]
pub enum BuildPhaseOutcome {
    /// `Eval` action — drvPath only, no closure.
    EvalDone { drv_path: String },
    /// `Build`/`Boot`/`Switch`/`Test` — closure realised somewhere.
    BuildDone {
        store_path: StorePath,
        location: BuildLocation,
    },
}

pub struct NixBuild {
    pub flake: FlakeRef,
    pub horizon_uri: OverrideUri,
    pub system_uri: OverrideUri,
    pub deployment_uri: OverrideUri,
    pub plan: BuildPlan,
    pub builder: Option<SshTarget>,
}

impl NixBuild {
    /// (program, argv) for the nix invocation. Pure — the same
    /// values are run locally or wrapped into an ssh argv when a
    /// `builder` is set. Exposed so tests can assert wire shape
    /// without spawning nix.
    pub fn nix_argv(&self) -> (&'static str, Vec<String>) {
        let target_attr = self.plan.target_attr(&self.flake);
        let mut argv = match self.plan.nix_operation() {
            NixOperation::EvalDrvPath => vec![
                "eval".to_string(),
                "--raw".to_string(),
                format!("{target_attr}.drvPath"),
            ],
            NixOperation::BuildClosure => vec![
                "build".to_string(),
                "--no-link".to_string(),
                "--print-out-paths".to_string(),
                target_attr,
            ],
        };
        argv.push("--override-input".to_string());
        argv.push("horizon".to_string());
        argv.push(self.horizon_uri.as_str().to_string());
        argv.push("--override-input".to_string());
        argv.push("system".to_string());
        argv.push(self.system_uri.as_str().to_string());
        argv.push("--override-input".to_string());
        argv.push("deployment".to_string());
        argv.push(self.deployment_uri.as_str().to_string());
        ("nix", argv)
    }

    pub async fn run(&self) -> Result<BuildPhaseOutcome> {
        let (program, argv) = self.nix_argv();
        // stderr inherits the dispatcher's terminal so nix's
        // progress (and ssh diagnostics, when running remote)
        // stream live. stdout is piped — drvPath / store path is
        // returned to the caller. ProcessGroup + KillOnDrop reap
        // the whole nix child tree (and any ssh tunnel) on
        // Ctrl-C / future-drop.
        let stdout = match &self.builder {
            None => run_local(program, &argv).await?,
            Some(target) => run_remote(target, program, &argv).await?,
        };

        match self.plan.nix_operation() {
            NixOperation::EvalDrvPath => Ok(BuildPhaseOutcome::EvalDone {
                drv_path: stdout.trim().to_string(),
            }),
            NixOperation::BuildClosure => {
                let store_path = StorePath::try_new(stdout)?;
                let location = match &self.builder {
                    None => BuildLocation::Dispatcher,
                    Some(target) => BuildLocation::Builder(target.clone()),
                };
                Ok(BuildPhaseOutcome::BuildDone {
                    store_path,
                    location,
                })
            }
        }
    }
}

async fn run_local(program: &str, argv: &[String]) -> Result<String> {
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
        return Err(Error::NixFailed {
            status: status.code().unwrap_or(-1),
            stderr: "(streamed to terminal — see above)".to_string(),
        });
    }
    Ok(stdout)
}

async fn run_remote(target: &SshTarget, program: &str, argv: &[String]) -> Result<String> {
    // OpenSSH joins the trailing argv with single spaces into one
    // command string the remote $SHELL re-parses. Quote each token
    // up-front so a value containing whitespace or a shell metachar
    // survives the round-trip intact.
    let mut remote_command = shell_quote(program);
    for arg in argv {
        remote_command.push(' ');
        remote_command.push_str(&shell_quote(arg));
    }
    run_local(
        "ssh",
        &[
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            target.as_ssh_arg().to_string(),
            remote_command,
        ],
    )
    .await
}

/// Single-quote `s` for safe inclusion in a POSIX shell command
/// line. Returns the input unchanged when every byte is from a
/// known-safe alphabet (saves visual noise on flag-like args).
fn shell_quote(s: &str) -> String {
    let safe = !s.is_empty()
        && s.bytes().all(|b| {
            matches!(b,
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9'
                | b'-' | b'_' | b'.' | b'/' | b'=' | b':' | b'#' | b'+' | b','
            )
        });
    if safe {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub struct NixBuilder;

pub enum BuildMsg {
    Run {
        build: NixBuild,
        reply: RpcReplyPort<Result<BuildPhaseOutcome>>,
    },
}

#[ractor::async_trait]
impl Actor for NixBuilder {
    type Msg = BuildMsg;
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
            BuildMsg::Run { build, reply } => {
                let _ = reply.send(build.run().await);
            }
        }
        Ok(())
    }
}
