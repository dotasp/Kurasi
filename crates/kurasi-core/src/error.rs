//! Errors that carry an HTTP status and one plain sentence for the person.

use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 400: the request is malformed.
    #[error("{0}")]
    BadRequest(String),
    /// 404: the item was not found.
    #[error("{0}")]
    NotFound(String),
    /// 409: the item changed in the meantime, or a resource is held.
    #[error("{0}")]
    Conflict(String),
    /// 409: the file is open in another program.
    #[error("The file is open in another program. Close it and try again.")]
    FileBusy,
    /// 423: the project is read-only.
    #[error("This project is read-only, so its files stay unchanged. Turn off Read-only in Settings to write to them.")]
    ReadOnly,
    /// 501: the PDF backend in use cannot do this.
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn bad(msg: impl Into<String>) -> Error {
        Error::BadRequest(msg.into())
    }
    pub fn not_found(msg: impl Into<String>) -> Error {
        Error::NotFound(msg.into())
    }
    pub fn conflict(msg: impl Into<String>) -> Error {
        Error::Conflict(msg.into())
    }
    pub fn unsupported(msg: impl Into<String>) -> Error {
        Error::Unsupported(msg.into())
    }

    /// The HTTP status the API answers with.
    pub fn status(&self) -> u16 {
        match self {
            Error::BadRequest(_) | Error::Json(_) => 400,
            Error::NotFound(_) => 404,
            Error::Conflict(_) | Error::FileBusy => 409,
            Error::ReadOnly => 423,
            Error::Unsupported(_) => 501,
            Error::Io(_) | Error::Sql(_) | Error::Other(_) => 500,
        }
    }

    /// One plain sentence for the person.
    pub fn message(&self) -> String {
        match self {
            Error::Io(e) => format!("A file operation failed: {}.", e),
            Error::Sql(e) => format!("The database reported an error: {}.", e),
            other => other.to_string(),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
