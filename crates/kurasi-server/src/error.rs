//! Errors of the server crate. Route handlers answer with the status of the core error
//! (`kurasi_core::Error::status`); everything else is a startup or transport failure.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Core(#[from] kurasi_core::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("MCP transport error: {0}")]
    Mcp(String),
    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn other(msg: impl Into<String>) -> Error {
        Error::Other(msg.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
