use horizon_lib::name::{ClusterName, NodeName, UserName};
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

    #[error("local hostname command failed (exit {status}): {stderr}")]
    LocalHostnameFailed { status: i32, stderr: String },

    #[error("invalid system activation for {action:?}: {reason}")]
    InvalidSystemActivation {
        action: crate::build::SystemAction,
        reason: &'static str,
    },

    #[error("invalid system profile link: {got:?}")]
    InvalidSystemProfileLink { got: String },

    #[error("builder node {0} not found in horizon ex_nodes")]
    UnknownBuilder(NodeName),

    #[error(
        "builder {0} is not a valid remote Nix builder in this horizon (isRemoteNixBuilder=false)"
    )]
    InvalidBuilder(NodeName),

    #[error("substituter node {0} not found in horizon")]
    UnknownSubstituter(NodeName),

    #[error("substituter node {0} is not a Nix cache in this horizon")]
    InvalidSubstituter(NodeName),

    #[error("user {user} not present in projected horizon users for {cluster}/{node}")]
    UnknownHomeUser {
        user: UserName,
        cluster: ClusterName,
        node: NodeName,
    },

    #[error("home activation requested for user {requested}, but current user is {actual}")]
    LocalHomeUserMismatch { requested: UserName, actual: String },

    #[error("home activation requested for node {requested}, but current node is {actual}")]
    LocalHomeNodeMismatch { requested: NodeName, actual: String },

    #[error("invalid {kind}: {got:?}")]
    InvalidName { kind: &'static str, got: String },

    #[error("unexpected command-line argument: {got:?}")]
    UnexpectedArgument { got: String },

    #[error("no lojix request supplied and no default config file exists; searched {searched:?}")]
    NoRequestConfig { searched: Vec<PathBuf> },

    #[error("HOME env var not set")]
    NoHome,

    #[error("USER/LOGNAME env var not set")]
    NoUser,

    #[error("CheckHostKeyMaterial: {0}")]
    CheckHostKeyMaterial(String),
}

pub type Result<T> = std::result::Result<T, Error>;
