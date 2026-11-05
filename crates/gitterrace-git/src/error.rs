/// Errors from reading a repository.
#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("not a git repository: {0}")]
    NotARepo(String),
    #[error("revision not found: {0}")]
    RevNotFound(String),
    #[error("path not found: {0}")]
    PathNotFound(String),
    #[error("git: {0}")]
    Git(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn git<E: std::fmt::Display>(e: E) -> Error {
    Error::Git(e.to_string())
}
