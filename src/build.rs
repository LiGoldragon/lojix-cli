use horizon_lib::name::UserName;
use horizon_lib::species::System;

use crate::cluster::{DerivationPath, FlakeInputRef, FlakeRef, StorePath};
use crate::error::Result;
use crate::host::SshTarget;
use crate::process::{ProcessFailure, ProcessInvocation, ProcessRun};

#[derive(Copy, Clone, Debug, PartialEq, Eq, nota_codec::NotaEnum)]
pub enum SystemAction {
    Eval,
    Build,
    Boot,
    Switch,
    Test,
    /// Install the new generation's bootloader entry, but keep the
    /// persistent default pointing at the *current* generation and set
    /// the new generation as a one-shot. Reboot 1 lands the new generation;
    /// reboot 2 (and every subsequent boot) returns to the old
    /// generation automatically. Designed for headless boxes where a
    /// permanent-default boot of an unverified generation is unsafe.
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HomeBuildPlan {
    pub user: UserName,
    pub mode: HomeMode,
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

    pub fn home_only(home: HomeBuildPlan) -> Self {
        Self::Home {
            user: home.user,
            mode: home.mode,
        }
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
            Self::Home { .. } => format!(
                "{}#homeConfigurations.{}.activationPackage",
                flake.as_str(),
                self.home_user().expect("home plan has home user").as_str(),
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
    /// `Eval` action — derivation path only, no closure.
    EvalDone { derivation_path: DerivationPath },
    /// `Build`/`Boot`/`Switch`/`Test` — closure realised somewhere.
    BuildDone {
        store_path: StorePath,
        location: BuildLocation,
    },
}

pub struct NixBuild {
    pub flake: FlakeRef,
    pub system: System,
    pub horizon_ref: FlakeInputRef,
    pub system_ref: FlakeInputRef,
    pub deployment_ref: Option<FlakeInputRef>,
    pub secrets_ref: Option<FlakeInputRef>,
    pub extra_substituters: ExtraSubstituters,
    pub plan: BuildPlan,
    pub builder: Option<SshTarget>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtraSubstituters {
    entries: Vec<ExtraSubstituter>,
}

impl ExtraSubstituters {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn from_entries(entries: Vec<ExtraSubstituter>) -> Self {
        Self { entries }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn urls_text(&self) -> String {
        self.entries
            .iter()
            .map(|entry| entry.url.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn public_keys_text(&self) -> String {
        self.entries
            .iter()
            .map(|entry| entry.public_key.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtraSubstituter {
    url: String,
    public_key: String,
}

impl ExtraSubstituter {
    pub fn new(url: impl Into<String>, public_key: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            public_key: public_key.into(),
        }
    }
}

impl NixBuild {
    /// Invocation for the nix command. Pure — the same values are run
    /// locally or wrapped into an ssh invocation when a
    /// `builder` is set. Exposed so tests can assert wire shape
    /// without spawning nix.
    pub fn nix_invocation(&self) -> ProcessInvocation {
        let target_attr = self.plan.target_attr(&self.flake);
        let arguments = match self.plan.nix_operation() {
            NixOperation::EvalDrvPath => vec![
                "eval".to_string(),
                "--refresh".to_string(),
                "--raw".to_string(),
                format!("{target_attr}.drvPath"),
            ],
            NixOperation::BuildClosure => vec![
                "build".to_string(),
                "--refresh".to_string(),
                "--no-link".to_string(),
                "--print-out-paths".to_string(),
                target_attr,
            ],
        };
        let mut invocation = ProcessInvocation::new("nix")
            .with_arguments(arguments)
            .with_arguments([
                "--override-input".to_string(),
                "horizon".to_string(),
                self.horizon_ref.flake_ref(),
                "--override-input".to_string(),
                "system".to_string(),
                self.system_ref.flake_ref(),
            ]);
        if let Some(deployment_ref) = &self.deployment_ref {
            invocation = invocation.with_arguments([
                "--override-input".to_string(),
                "deployment".to_string(),
                deployment_ref.flake_ref(),
            ]);
        }
        if let Some(secrets_ref) = &self.secrets_ref {
            invocation = invocation.with_arguments([
                "--override-input".to_string(),
                "secrets".to_string(),
                secrets_ref.flake_ref(),
            ]);
        }
        if !self.extra_substituters.is_empty() {
            invocation = invocation.with_arguments([
                "--option".to_string(),
                "extra-substituters".to_string(),
                self.extra_substituters.urls_text(),
                "--option".to_string(),
                "extra-trusted-public-keys".to_string(),
                self.extra_substituters.public_keys_text(),
            ]);
        }
        invocation
    }

    pub async fn run(&self) -> Result<BuildPhaseOutcome> {
        let invocation = self.execution_invocation();
        // stderr inherits the dispatcher's terminal so nix's
        // progress (and ssh diagnostics, when running remote)
        // stream live. stdout is piped — derivation path / store path is
        // returned to the caller. ProcessGroup + KillOnDrop reap
        // the whole nix child tree (and any ssh tunnel) on
        // Ctrl-C / future-drop.
        let output = invocation
            .capture_stdout(ProcessRun::inherit_stderr(ProcessFailure::Nix))
            .await?;
        let stdout = output.stdout();

        match self.plan.nix_operation() {
            NixOperation::EvalDrvPath => Ok(BuildPhaseOutcome::EvalDone {
                derivation_path: DerivationPath::try_new(stdout)?,
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

    fn execution_invocation(&self) -> ProcessInvocation {
        let nix_invocation = self.nix_invocation();
        match &self.builder {
            None => nix_invocation,
            Some(target) => target.remote_invocation(nix_invocation.to_shell_command()),
        }
    }
}
