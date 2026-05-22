use std::ffi::OsString;
use std::path::PathBuf;

use horizon_lib::name::{ClusterName, NodeName, UserName};
use nota_codec::{Decoder, Encoder, NotaDecode, NotaEncode, NotaRecord};

use crate::build::{BuildPlan, HomeBuildPlan, HomeMode, SystemAction};
use crate::check::CheckHostKeyMaterial;
use crate::cluster::{FlakeRef, ProposalSource};
use crate::deploy::DeployRequest;
use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, NotaRecord)]
pub struct FullOs {
    pub cluster: ClusterName,
    pub node: NodeName,
    pub source: ProposalSource,
    pub criomos: FlakeRef,
    pub action: SystemAction,
    pub builder: Option<NodeName>,
    pub substituters: Option<Vec<NodeName>>,
}

#[derive(Debug, Clone, PartialEq, Eq, NotaRecord)]
pub struct OsOnly {
    pub cluster: ClusterName,
    pub node: NodeName,
    pub source: ProposalSource,
    pub criomos: FlakeRef,
    pub action: SystemAction,
    pub builder: Option<NodeName>,
    pub substituters: Option<Vec<NodeName>>,
}

#[derive(Debug, Clone, PartialEq, Eq, NotaRecord)]
pub struct HomeOnly {
    pub cluster: ClusterName,
    pub node: NodeName,
    pub user: UserName,
    pub source: ProposalSource,
    pub home: FlakeRef,
    pub mode: HomeMode,
    pub builder: Option<NodeName>,
    pub substituters: Option<Vec<NodeName>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LojixRequest {
    FullOs(FullOs),
    OsOnly(OsOnly),
    HomeOnly(HomeOnly),
    CheckHostKeyMaterial(CheckHostKeyMaterial),
}

impl LojixRequest {
    pub fn from_nota(text: &str) -> Result<Self> {
        let mut decoder = Decoder::new(text);
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
}

impl FullOs {
    pub fn into_deploy_request(self) -> DeployRequest {
        DeployRequest {
            cluster: self.cluster,
            node: self.node,
            builder: self.builder,
            substituters: self.substituters.unwrap_or_default(),
            plan: BuildPlan::full_os(self.action),
            source: self.source,
            flake: self.criomos,
        }
    }
}

impl OsOnly {
    pub fn into_deploy_request(self) -> DeployRequest {
        DeployRequest {
            cluster: self.cluster,
            node: self.node,
            builder: self.builder,
            substituters: self.substituters.unwrap_or_default(),
            plan: BuildPlan::os_only(self.action),
            source: self.source,
            flake: self.criomos,
        }
    }
}

impl HomeOnly {
    pub fn into_deploy_request(self) -> DeployRequest {
        DeployRequest {
            cluster: self.cluster,
            node: self.node,
            builder: self.builder,
            substituters: self.substituters.unwrap_or_default(),
            plan: BuildPlan::home_only(HomeBuildPlan {
                user: self.user,
                mode: self.mode,
            }),
            source: self.source,
            flake: self.home,
        }
    }
}

impl NotaEncode for LojixRequest {
    fn encode(&self, encoder: &mut Encoder) -> nota_codec::Result<()> {
        match self {
            Self::FullOs(request) => request.encode(encoder),
            Self::OsOnly(request) => request.encode(encoder),
            Self::HomeOnly(request) => request.encode(encoder),
            Self::CheckHostKeyMaterial(request) => request.encode(encoder),
        }
    }
}

impl NotaDecode for LojixRequest {
    fn decode(decoder: &mut Decoder<'_>) -> nota_codec::Result<Self> {
        let head = decoder.peek_record_head()?;
        match head.as_str() {
            "FullOs" => Ok(Self::FullOs(FullOs::decode(decoder)?)),
            "OsOnly" => Ok(Self::OsOnly(OsOnly::decode(decoder)?)),
            "HomeOnly" => Ok(Self::HomeOnly(HomeOnly::decode(decoder)?)),
            "CheckHostKeyMaterial" => Ok(Self::CheckHostKeyMaterial(CheckHostKeyMaterial::decode(
                decoder,
            )?)),
            other => Err(nota_codec::Error::UnknownVariant {
                enum_name: "LojixRequest",
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
            Some(first) if CommandLineArgument::new(first).starts_inline_record() => {
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

struct CommandLineArgument<'argument> {
    argument: &'argument OsString,
}

impl<'argument> CommandLineArgument<'argument> {
    fn new(argument: &'argument OsString) -> Self {
        Self { argument }
    }

    fn starts_inline_record(&self) -> bool {
        self.argument.to_string_lossy().starts_with('(')
    }
}
