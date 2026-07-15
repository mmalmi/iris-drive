use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("entry not found")]
    NotFound,
    #[error("not a directory")]
    NotDir,
    #[error("is a directory")]
    IsDir,
    #[error("entry already exists")]
    AlreadyExists,
    #[error("directory not empty")]
    NotEmpty,
    #[error("invalid entry name")]
    InvalidName,
    #[error("invalid root: {0}")]
    InvalidRoot(String),
    #[error("backend error: {0}")]
    Backend(String),
}
