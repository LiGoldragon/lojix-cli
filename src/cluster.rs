use std::path::{Path, PathBuf};

use horizon_lib::ClusterProposal;
use nota_next::{Block, NotaDecode, NotaDecodeError, NotaEncode, NotaSource};

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
        Ok(NotaSource::new(&bytes).parse()?)
    }
}

impl NotaEncode for ProposalSource {
    fn to_nota(&self) -> String {
        self.0.display().to_string().to_nota()
    }
}

impl NotaDecode for ProposalSource {
    fn from_nota_block(block: &Block) -> std::result::Result<Self, NotaDecodeError> {
        let value = String::from_nota_block(block)?;
        if value.contains('"') {
            return Err(NotaDecodeError::InvalidValue {
                type_name: "ProposalSource",
                value,
                reason: "quotation marks are not NOTA string delimiters".to_string(),
            });
        }
        Ok(Self(PathBuf::from(value)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, nota_next::NotaEncode)]
pub struct FlakeRef(String);

impl FlakeRef {
    pub fn new(uri: impl Into<String>) -> Self {
        Self(uri.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn nix_string_literal(&self) -> String {
        format!("\"{}\"", self.0.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

impl NotaDecode for FlakeRef {
    fn from_nota_block(block: &Block) -> std::result::Result<Self, NotaDecodeError> {
        let value = String::from_nota_block(block)?;
        if value.contains('"') {
            return Err(NotaDecodeError::InvalidValue {
                type_name: "FlakeRef",
                value,
                reason: "quotation marks are not NOTA string delimiters".to_string(),
            });
        }
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlakeInputRef {
    url: String,
    nar_hash: NarHashSri,
}

impl FlakeInputRef {
    pub fn from_local_path(path: &Path, nar_hash: NarHashSri) -> Self {
        Self {
            url: format!("path:{}", path.display()),
            nar_hash,
        }
    }

    pub fn as_str(&self) -> &str {
        &self.url
    }

    pub fn flake_ref(&self) -> String {
        format!("{}?narHash={}", self.url, self.nar_hash.as_url_parameter())
    }

    pub fn nix_string_literal(&self) -> String {
        format!(
            "\"{}\"",
            self.flake_ref().replace('\\', "\\\\").replace('"', "\\\"")
        )
    }

    pub fn nar_hash(&self) -> &NarHashSri {
        &self.nar_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

    pub fn as_url_parameter(&self) -> String {
        self.0
            .replace('=', "%3D")
            .replace('+', "%2B")
            .replace('/', "%2F")
    }

    pub fn short_code(&self) -> String {
        self.0
            .trim_start_matches("sha256-")
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .take(12)
            .collect::<String>()
            .to_ascii_lowercase()
    }
}

/// A `/nix/store/...` path. Constructed from the stdout of `nix
/// build --print-out-paths`; rejected if the prefix doesn't match
/// (catches accidental whitespace, multi-line output, etc.).
#[derive(Debug, Clone, PartialEq, Eq)]
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
