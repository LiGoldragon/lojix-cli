use horizon_lib::name::NodeName;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("nota: {0}")]
    Nota(#[from] nota_codec::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("horizon: {0}")]
    Horizon(#[from] horizon_lib::Error),

    #[error("nix invocation failed (exit {status}): {stderr}")]
    NixFailed { status: i32, stderr: String },

    #[error("rsync failed (exit {status}): {stderr}")]
    RsyncFailed { status: i32, stderr: String },

    #[error("ssh failed (exit {status}): {stderr}")]
    SshFailed { status: i32, stderr: String },

    #[error("builder node {0} not found in horizon ex_nodes")]
    UnknownBuilder(NodeName),

    #[error("builder {0} is not a valid builder in this horizon (is_builder=false or offline)")]
    InvalidBuilder(NodeName),

    #[error("invalid {kind}: {got:?}")]
    InvalidName { kind: &'static str, got: String },

    #[error("unexpected command-line argument: {got:?}")]
    UnexpectedArgument { got: String },

    #[error("no lojix request supplied and no default config file exists; searched {searched:?}")]
    NoRequestConfig { searched: Vec<PathBuf> },

    #[error("HOME env var not set")]
    NoHome,

    #[error("ractor: {0}")]
    Ractor(String),
}

pub type Result<T> = std::result::Result<T, Error>;
