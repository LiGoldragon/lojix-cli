use std::path::{Path, PathBuf};

use horizon_lib::ClusterProposal;
use nota_codec::{NotaDecode, NotaEncode};

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalSource(PathBuf);

impl ProposalSource {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn load(&self) -> Result<ClusterProposal> {
        let bytes = std::fs::read_to_string(&self.0)?;
        let mut decoder = nota_codec::Decoder::nota(&bytes);
        Ok(<ClusterProposal as nota_codec::NotaDecode>::decode(
            &mut decoder,
        )?)
    }
}

impl NotaEncode for ProposalSource {
    fn encode(&self, encoder: &mut nota_codec::Encoder) -> nota_codec::Result<()> {
        self.0.display().to_string().encode(encoder)
    }
}

impl NotaDecode for ProposalSource {
    fn decode(decoder: &mut nota_codec::Decoder<'_>) -> nota_codec::Result<Self> {
        Ok(Self(PathBuf::from(String::decode(decoder)?)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, nota_codec::NotaTransparent)]
pub struct FlakeRef(String);

impl FlakeRef {
    pub fn new(uri: impl Into<String>) -> Self {
        Self(uri.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct OverrideUri(String);

impl OverrideUri {
    pub fn from_local_path(path: &Path) -> Self {
        Self(format!("path:{}", path.display()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct NarHashSri(String);

impl NarHashSri {
    pub fn try_new(text: impl Into<String>) -> Result<Self> {
        let text = text.into();
        if text.starts_with("sha256-") {
            Ok(Self(text))
        } else {
            Err(Error::InvalidName {
                kind: "NarHashSri (must start with sha256-)",
                got: text,
            })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A `/nix/store/...` path. Constructed from the stdout of `nix
/// build --print-out-paths`; rejected if the prefix doesn't match
/// (catches accidental whitespace, multi-line output, etc.).
#[derive(Debug, Clone)]
pub struct StorePath(String);

impl StorePath {
    pub fn try_new(text: impl Into<String>) -> Result<Self> {
        let text = text.into();
        let trimmed = text.trim();
        if trimmed.starts_with("/nix/store/") && !trimmed.contains('\n') {
            Ok(Self(trimmed.to_string()))
        } else {
            Err(Error::InvalidName {
                kind: "StorePath (must start with /nix/store/ and be one line)",
                got: text,
            })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivationPath(String);

impl DerivationPath {
    pub fn try_new(text: impl Into<String>) -> Result<Self> {
        let text = text.into();
        let trimmed = text.trim();
        if trimmed.starts_with("/nix/store/")
            && trimmed.ends_with(".drv")
            && !trimmed.contains('\n')
        {
            Ok(Self(trimmed.to_string()))
        } else {
            Err(Error::InvalidName {
                kind: "DerivationPath (must be one /nix/store/*.drv path)",
                got: text,
            })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
