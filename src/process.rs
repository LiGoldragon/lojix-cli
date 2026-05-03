use std::process::Stdio;

use process_wrap::tokio::*;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInvocation {
    program: String,
    arguments: Vec<String>,
}

impl ProcessInvocation {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            arguments: Vec::new(),
        }
    }

    pub fn with_argument(mut self, argument: impl Into<String>) -> Self {
        self.arguments.push(argument.into());
        self
    }

    pub fn with_arguments<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.arguments.extend(arguments.into_iter().map(Into::into));
        self
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    pub fn to_shell_command(&self) -> ShellCommand {
        ShellCommand::from_invocation(self)
    }

    pub async fn capture_stdout(&self, run: ProcessRun) -> Result<ProcessOutput> {
        let mut wrapper = CommandWrap::with_new(&self.program, |command: &mut Command| {
            command
                .args(&self.arguments)
                .stdin(Stdio::null())
                .stdout(Stdio::piped());
            match run.stderr {
                ProcessStream::Inherit => {
                    command.stderr(Stdio::inherit());
                }
                ProcessStream::Capture => {
                    command.stderr(Stdio::piped());
                }
            }
        });
        wrapper.wrap(ProcessGroup::leader());
        wrapper.wrap(KillOnDrop);
        let mut child = wrapper.spawn()?;

        let mut stdout = String::new();
        if let Some(mut stdout_pipe) = child.stdout().take() {
            stdout_pipe.read_to_string(&mut stdout).await?;
        }

        let mut stderr = String::new();
        if let Some(mut stderr_pipe) = child.stderr().take() {
            stderr_pipe.read_to_string(&mut stderr).await?;
        }

        let status = child.wait().await?;
        if !status.success() {
            return Err(run.failure.error(ProcessExit {
                status: status.code().unwrap_or(-1),
                stderr: run.stderr.error_text(stderr),
            }));
        }

        Ok(ProcessOutput { stdout, stderr })
    }

    pub async fn inherit_stdio(&self, run: ProcessRun) -> Result<()> {
        let mut wrapper = CommandWrap::with_new(&self.program, |command: &mut Command| {
            command
                .args(&self.arguments)
                .stdin(Stdio::null())
                .stdout(Stdio::inherit());
            match run.stderr {
                ProcessStream::Inherit => {
                    command.stderr(Stdio::inherit());
                }
                ProcessStream::Capture => {
                    command.stderr(Stdio::piped());
                }
            }
        });
        wrapper.wrap(ProcessGroup::leader());
        wrapper.wrap(KillOnDrop);
        let mut child = wrapper.spawn()?;

        let mut stderr = String::new();
        if let Some(mut stderr_pipe) = child.stderr().take() {
            stderr_pipe.read_to_string(&mut stderr).await?;
        }

        let status = child.wait().await?;
        if !status.success() {
            return Err(run.failure.error(ProcessExit {
                status: status.code().unwrap_or(-1),
                stderr: run.stderr.error_text(stderr),
            }));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCommand(String);

impl ShellCommand {
    pub fn from_invocation(invocation: &ProcessInvocation) -> Self {
        let mut command = ShellArgument::new(invocation.program()).to_command_text();
        for argument in invocation.arguments() {
            command.push(' ');
            command.push_str(&ShellArgument::new(argument).to_command_text());
        }
        Self(command)
    }

    pub fn from_raw(script: impl Into<String>) -> Self {
        Self(script.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub struct ShellArgument<'argument> {
    text: &'argument str,
}

impl<'argument> ShellArgument<'argument> {
    pub fn new(text: &'argument str) -> Self {
        Self { text }
    }

    pub fn to_command_text(&self) -> String {
        let text = self.text;
        let safe = !text.is_empty()
            && text.bytes().all(|byte| {
                matches!(
                    byte,
                    b'a'..=b'z'
                        | b'A'..=b'Z'
                        | b'0'..=b'9'
                        | b'-'
                        | b'_'
                        | b'.'
                        | b'/'
                        | b'='
                        | b':'
                        | b'#'
                        | b'+'
                        | b','
                )
            });
        if safe {
            return text.to_string();
        }
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessRun {
    failure: ProcessFailure,
    stderr: ProcessStream,
}

impl ProcessRun {
    pub fn inherit_stderr(failure: ProcessFailure) -> Self {
        Self {
            failure,
            stderr: ProcessStream::Inherit,
        }
    }

    pub fn capture_stderr(failure: ProcessFailure) -> Self {
        Self {
            failure,
            stderr: ProcessStream::Capture,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessStream {
    Inherit,
    Capture,
}

impl ProcessStream {
    fn error_text(self, stderr: String) -> String {
        match self {
            Self::Inherit => "(see streamed output)".to_string(),
            Self::Capture => stderr,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessFailure {
    Nix,
    Rsync,
    Ssh,
    Tar,
    LocalHostname,
}

impl ProcessFailure {
    fn error(self, exit: ProcessExit) -> Error {
        match self {
            Self::Nix => Error::NixFailed {
                status: exit.status,
                stderr: exit.stderr,
            },
            Self::Rsync => Error::RsyncFailed {
                status: exit.status,
                stderr: exit.stderr,
            },
            Self::Ssh => Error::SshFailed {
                status: exit.status,
                stderr: exit.stderr,
            },
            Self::Tar => Error::TarFailed {
                status: exit.status,
                stderr: exit.stderr,
            },
            Self::LocalHostname => Error::LocalHostnameFailed {
                status: exit.status,
                stderr: exit.stderr,
            },
        }
    }
}

struct ProcessExit {
    status: i32,
    stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    stdout: String,
    stderr: String,
}

impl ProcessOutput {
    pub fn stdout(&self) -> &str {
        &self.stdout
    }

    pub fn stderr(&self) -> &str {
        &self.stderr
    }
}
