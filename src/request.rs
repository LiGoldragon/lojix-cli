use std::ffi::OsString;
use std::path::PathBuf;

use horizon_lib::name::{ClusterName, NodeName};
use nota_codec::{Decoder, Encoder, NotaDecode, NotaEncode, NotaRecord};

use crate::build::BuildAction;
use crate::cluster::{FlakeRef, ProposalSource};
use crate::deploy::DeployRequest;
use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, NotaRecord)]
pub struct Eval {
    pub cluster: ClusterName,
    pub node: NodeName,
    pub source: ProposalSource,
    pub criomos: FlakeRef,
    pub builder: Option<NodeName>,
}

#[derive(Debug, Clone, PartialEq, Eq, NotaRecord)]
pub struct Build {
    pub cluster: ClusterName,
    pub node: NodeName,
    pub source: ProposalSource,
    pub criomos: FlakeRef,
    pub builder: Option<NodeName>,
}

#[derive(Debug, Clone, PartialEq, Eq, NotaRecord)]
pub struct Deploy {
    pub cluster: ClusterName,
    pub node: NodeName,
    pub source: ProposalSource,
    pub criomos: FlakeRef,
    pub action: BuildAction,
    pub builder: Option<NodeName>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LojixRequest {
    Eval(Eval),
    Build(Build),
    Deploy(Deploy),
}

impl LojixRequest {
    pub fn from_nota(text: &str) -> Result<Self> {
        let mut decoder = Decoder::nota(text);
        let request = Self::decode(&mut decoder)?;
        if let Some(token) = decoder.peek_token()? {
            return Err(nota_codec::Error::UnexpectedToken {
                expected: "end of input",
                got: token,
            }
            .into());
        }
        Ok(request)
    }

    pub fn into_deploy_request(self) -> DeployRequest {
        match self {
            Self::Eval(request) => request.into_deploy_request(),
            Self::Build(request) => request.into_deploy_request(),
            Self::Deploy(request) => request.into_deploy_request(),
        }
    }
}

impl Eval {
    pub fn into_deploy_request(self) -> DeployRequest {
        DeployRequest {
            cluster: self.cluster,
            node: self.node,
            builder: self.builder,
            action: BuildAction::Eval,
            source: self.source,
            criomos: self.criomos,
        }
    }
}

impl Build {
    pub fn into_deploy_request(self) -> DeployRequest {
        DeployRequest {
            cluster: self.cluster,
            node: self.node,
            builder: self.builder,
            action: BuildAction::Build,
            source: self.source,
            criomos: self.criomos,
        }
    }
}

impl Deploy {
    pub fn into_deploy_request(self) -> DeployRequest {
        DeployRequest {
            cluster: self.cluster,
            node: self.node,
            builder: self.builder,
            action: self.action,
            source: self.source,
            criomos: self.criomos,
        }
    }
}

impl NotaEncode for LojixRequest {
    fn encode(&self, encoder: &mut Encoder) -> nota_codec::Result<()> {
        match self {
            Self::Eval(request) => request.encode(encoder),
            Self::Build(request) => request.encode(encoder),
            Self::Deploy(request) => request.encode(encoder),
        }
    }
}

impl NotaDecode for LojixRequest {
    fn decode(decoder: &mut Decoder<'_>) -> nota_codec::Result<Self> {
        let head = decoder.peek_record_head()?;
        match head.as_str() {
            "Eval" => Ok(Self::Eval(Eval::decode(decoder)?)),
            "Build" => Ok(Self::Build(Build::decode(decoder)?)),
            "Deploy" => Ok(Self::Deploy(Deploy::decode(decoder)?)),
            other => Err(nota_codec::Error::UnknownKindForVerb {
                verb: "LojixRequest",
                got: other.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    arguments: Vec<OsString>,
}

impl CommandLine {
    pub fn from_env() -> Self {
        Self::from_arguments(std::env::args_os().skip(1))
    }

    pub fn from_arguments<I, S>(arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        Self {
            arguments: arguments.into_iter().map(Into::into).collect(),
        }
    }

    pub fn decode_request(&self) -> Result<LojixRequest> {
        match self.arguments.first() {
            Some(first) if starts_inline_record(first) => {
                LojixRequest::from_nota(&self.inline_nota_text()?)
            }
            Some(first) => {
                self.require_single_path_argument()?;
                RequestFile::from_path(PathBuf::from(first)).decode()
            }
            None => RequestFile::from_default_locations()?.decode(),
        }
    }

    fn inline_nota_text(&self) -> Result<String> {
        let mut parts = Vec::new();
        for argument in &self.arguments {
            let Some(text) = argument.to_str() else {
                return Err(Error::InvalidName {
                    kind: "inline Nota argument (must be UTF-8)",
                    got: format!("{argument:?}"),
                });
            };
            parts.push(text.to_string());
        }
        Ok(parts.join(" "))
    }

    fn require_single_path_argument(&self) -> Result<()> {
        if let Some(argument) = self.arguments.get(1) {
            return Err(Error::UnexpectedArgument {
                got: argument.to_string_lossy().to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestFile {
    path: PathBuf,
}

impl RequestFile {
    pub fn from_path(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn from_default_locations() -> Result<Self> {
        let mut searched = Vec::new();
        if let Some(path) = std::env::var_os("LOJIX_CONFIG").map(PathBuf::from) {
            if path.exists() {
                return Ok(Self { path });
            }
            searched.push(path);
        }
        if let Some(path) = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .map(|path| path.join("lojix/config.nota"))
        {
            if path.exists() {
                return Ok(Self { path });
            }
            searched.push(path);
        }
        if let Some(path) = std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(".config/lojix/config.nota"))
        {
            if path.exists() {
                return Ok(Self { path });
            }
            searched.push(path);
        }
        Err(Error::NoRequestConfig { searched })
    }

    pub fn decode(&self) -> Result<LojixRequest> {
        let text = std::fs::read_to_string(&self.path)?;
        LojixRequest::from_nota(&text)
    }
}

fn starts_inline_record(argument: &OsString) -> bool {
    argument.to_string_lossy().starts_with('(')
}
