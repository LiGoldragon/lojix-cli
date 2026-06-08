use std::ffi::OsString;
use std::path::PathBuf;

use horizon_lib::name::{ClusterName, NodeName, UserName};
use nota_next::{
    Block, Delimiter, NotaBlock, NotaBodyEncode, NotaDecode, NotaDecodeError, NotaEncode,
    NotaSource,
};

use crate::build::{BuildPlan, HomeBuildPlan, HomeMode, SystemAction};
use crate::check::CheckHostKeyMaterial;
use crate::cluster::{FlakeRef, ProposalSource};
use crate::deploy::DeployRequest;
use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, NotaDecode, NotaEncode)]
pub struct FullOs {
    pub cluster: ClusterName,
    pub node: NodeName,
    pub source: ProposalSource,
    pub criomos: FlakeRef,
    pub action: SystemAction,
    pub builder: Option<NodeName>,
    pub substituters: Option<Vec<NodeName>>,
}

#[derive(Debug, Clone, PartialEq, Eq, NotaDecode, NotaEncode)]
pub struct OsOnly {
    pub cluster: ClusterName,
    pub node: NodeName,
    pub source: ProposalSource,
    pub criomos: FlakeRef,
    pub action: SystemAction,
    pub builder: Option<NodeName>,
    pub substituters: Option<Vec<NodeName>>,
}

#[derive(Debug, Clone, PartialEq, Eq, NotaDecode, NotaEncode)]
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
        Ok(NotaSource::new(text).parse()?)
    }

    fn from_tagged_children(children: &[Block]) -> std::result::Result<Self, NotaDecodeError> {
        let Some(tag_block) = children.first() else {
            return Err(NotaDecodeError::ExpectedRootCount {
                type_name: "LojixRequest",
                expected: 1,
                found: 0,
            });
        };
        let tag = tag_block
            .demote_to_string()
            .ok_or(NotaDecodeError::ExpectedAtom {
                type_name: "LojixRequest variant",
            })?;
        let payload = &children[1..];
        match tag {
            "FullOs" => Ok(Self::FullOs(FullOs::from_body_objects(payload)?)),
            "OsOnly" => Ok(Self::OsOnly(OsOnly::from_body_objects(payload)?)),
            "HomeOnly" => Ok(Self::HomeOnly(HomeOnly::from_body_objects(payload)?)),
            "CheckHostKeyMaterial" => Ok(Self::CheckHostKeyMaterial(
                CheckHostKeyMaterial::from_body_objects(payload)?,
            )),
            other => Err(NotaDecodeError::UnknownVariant {
                enum_name: "LojixRequest",
                variant: other.to_string(),
            }),
        }
    }

    fn tagged_payload_to_nota(tag: &'static str, payload: &impl NotaBodyEncode) -> String {
        let mut fields = Vec::new();
        fields.push(tag.to_owned());
        fields.extend(payload.to_nota_body().fields().iter().cloned());
        Delimiter::Parenthesis.wrap(fields)
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
    fn to_nota(&self) -> String {
        match self {
            Self::FullOs(request) => Self::tagged_payload_to_nota("FullOs", request),
            Self::OsOnly(request) => Self::tagged_payload_to_nota("OsOnly", request),
            Self::HomeOnly(request) => Self::tagged_payload_to_nota("HomeOnly", request),
            Self::CheckHostKeyMaterial(request) => {
                Self::tagged_payload_to_nota("CheckHostKeyMaterial", request)
            }
        }
    }
}

impl NotaDecode for LojixRequest {
    fn from_nota_block(block: &Block) -> std::result::Result<Self, NotaDecodeError> {
        let children =
            NotaBlock::new(block).expect_delimited(Delimiter::Parenthesis, "LojixRequest")?;
        Self::from_tagged_children(children)
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
