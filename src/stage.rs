use std::path::{Path, PathBuf};

use crate::artifact::MaterializedArtifact;
use crate::cluster::{FlakeInputRef, NarHashSri};
use crate::error::Result;
use crate::host::SshTarget;
use crate::process::{ProcessFailure, ProcessInvocation, ProcessRun, ShellArgument, ShellCommand};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInputReferences {
    pub horizon_ref: FlakeInputRef,
    pub system_ref: FlakeInputRef,
    pub deployment_ref: Option<FlakeInputRef>,
}

impl BuildInputReferences {
    pub fn from_local_artifact(artifact: &MaterializedArtifact) -> Self {
        Self {
            horizon_ref: artifact.horizon_ref.clone(),
            system_ref: artifact.system_ref.clone(),
            deployment_ref: artifact.deployment_ref.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratedInputName {
    Horizon,
    System,
    Deployment,
}

impl GeneratedInputName {
    fn as_str(self) -> &'static str {
        match self {
            Self::Horizon => "horizon",
            Self::System => "system",
            Self::Deployment => "deployment",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedInput {
    name: GeneratedInputName,
    local_path: PathBuf,
    nar_hash: NarHashSri,
}

impl GeneratedInput {
    pub fn new(name: GeneratedInputName, local_path: PathBuf, nar_hash: NarHashSri) -> Self {
        Self {
            name,
            local_path,
            nar_hash,
        }
    }

    fn from_artifact(artifact: &MaterializedArtifact) -> Vec<Self> {
        let mut inputs = vec![
            Self::new(
                GeneratedInputName::Horizon,
                artifact.horizon_dir.path().to_path_buf(),
                artifact.horizon_nar_hash.clone(),
            ),
            Self::new(
                GeneratedInputName::System,
                artifact.system_dir.path().to_path_buf(),
                artifact.system_nar_hash.clone(),
            ),
        ];
        if let (Some(directory), Some(nar_hash)) =
            (&artifact.deployment_dir, &artifact.deployment_nar_hash)
        {
            inputs.push(Self::new(
                GeneratedInputName::Deployment,
                directory.path().to_path_buf(),
                nar_hash.clone(),
            ));
        }
        inputs
    }

    fn root_segment(&self) -> String {
        format!("{}-{}", self.name.as_str(), self.nar_hash.short_code())
    }

    fn remote_directory(&self, root: &RemoteInputRoot) -> RemoteInputDirectory {
        root.input_directory(self.name)
    }

    fn flake_ref(&self, root: &RemoteInputRoot) -> FlakeInputRef {
        FlakeInputRef::from_local_path(self.remote_directory(root).as_path(), self.nar_hash.clone())
    }

    fn stage_commands(
        &self,
        target: &SshTarget,
        root: &RemoteInputRoot,
    ) -> [RemoteInputStageCommand; 2] {
        let remote_directory = self.remote_directory(root);
        [
            RemoteInputStageCommand::MakeDirectory {
                invocation: remote_directory.create_invocation(target),
            },
            RemoteInputStageCommand::Synchronize {
                invocation: remote_directory.rsync_invocation(target, &self.local_path),
            },
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteInputRoot(PathBuf);

impl RemoteInputRoot {
    fn from_inputs(inputs: &[GeneratedInput]) -> Self {
        let key = inputs
            .iter()
            .map(GeneratedInput::root_segment)
            .collect::<Vec<_>>()
            .join("_");
        Self(PathBuf::from("/var/tmp/lojix/generated-inputs").join(key))
    }

    fn input_directory(&self, name: GeneratedInputName) -> RemoteInputDirectory {
        RemoteInputDirectory(self.0.join(name.as_str()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteInputDirectory(PathBuf);

impl RemoteInputDirectory {
    fn as_path(&self) -> &Path {
        &self.0
    }

    fn create_invocation(&self, target: &SshTarget) -> ProcessInvocation {
        target.remote_invocation(ShellCommand::from_raw(format!(
            "mkdir -p {}",
            ShellArgument::new(&self.0.display().to_string()).to_command_text()
        )))
    }

    fn rsync_invocation(&self, target: &SshTarget, local_path: &Path) -> ProcessInvocation {
        ProcessInvocation::new("rsync").with_arguments([
            "-a".to_string(),
            "--delete".to_string(),
            format!("{}/", local_path.display()),
            format!("{}:{}/", target, self.0.display()),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteInputStage {
    target: SshTarget,
    inputs: Vec<GeneratedInput>,
}

impl RemoteInputStage {
    pub fn new(target: SshTarget, inputs: Vec<GeneratedInput>) -> Self {
        Self { target, inputs }
    }

    pub fn from_artifact(target: SshTarget, artifact: &MaterializedArtifact) -> Self {
        Self::new(target, GeneratedInput::from_artifact(artifact))
    }

    pub fn plan(&self) -> RemoteInputStagePlan {
        let root = RemoteInputRoot::from_inputs(&self.inputs);
        let mut commands = Vec::new();
        for input in &self.inputs {
            commands.extend(input.stage_commands(&self.target, &root));
        }
        RemoteInputStagePlan {
            commands,
            references: BuildInputReferences {
                horizon_ref: self
                    .input_ref(GeneratedInputName::Horizon, &root)
                    .expect("remote input stage always includes horizon"),
                system_ref: self
                    .input_ref(GeneratedInputName::System, &root)
                    .expect("remote input stage always includes system"),
                deployment_ref: self.input_ref(GeneratedInputName::Deployment, &root),
            },
        }
    }

    pub async fn run(&self) -> Result<BuildInputReferences> {
        let plan = self.plan();
        plan.run().await
    }

    fn input_ref(&self, name: GeneratedInputName, root: &RemoteInputRoot) -> Option<FlakeInputRef> {
        self.inputs
            .iter()
            .find(|input| input.name == name)
            .map(|input| input.flake_ref(root))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteInputStagePlan {
    commands: Vec<RemoteInputStageCommand>,
    references: BuildInputReferences,
}

impl RemoteInputStagePlan {
    pub fn commands(&self) -> &[RemoteInputStageCommand] {
        &self.commands
    }

    pub fn references(&self) -> &BuildInputReferences {
        &self.references
    }

    pub async fn run(self) -> Result<BuildInputReferences> {
        for command in self.commands {
            command.run().await?;
        }
        Ok(self.references)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteInputStageCommand {
    MakeDirectory { invocation: ProcessInvocation },
    Synchronize { invocation: ProcessInvocation },
}

impl RemoteInputStageCommand {
    pub fn invocation(&self) -> &ProcessInvocation {
        match self {
            Self::MakeDirectory { invocation } | Self::Synchronize { invocation } => invocation,
        }
    }

    async fn run(&self) -> Result<()> {
        match self {
            Self::MakeDirectory { invocation } => {
                invocation
                    .inherit_stdio(ProcessRun::inherit_stderr(ProcessFailure::Ssh))
                    .await
            }
            Self::Synchronize { invocation } => {
                invocation
                    .inherit_stdio(ProcessRun::inherit_stderr(ProcessFailure::Rsync))
                    .await
            }
        }
    }
}
