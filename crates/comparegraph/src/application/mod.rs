pub mod absent;
pub mod add;
pub mod agreement;
pub mod build;
pub mod catalog;
pub mod check;
pub mod claim;
pub mod communities;
pub mod conform;
pub mod decide;
pub mod export;
pub mod graph;
pub mod family;
pub mod lines;
pub mod map;
pub mod matrix;
pub mod propose;
pub mod search;
pub mod setup;
pub mod show;
pub mod vault;
use std::path::PathBuf;
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Store(#[from] crate::store::StoreError),
    #[error(transparent)]
    Provider(#[from] crate::providers::ProviderError),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Empty(String),
    #[error("{0}")]
    BelowFloor(String),
    #[error("{0}")]
    CheckFailed(String),
    #[error("{0}")]
    InvalidArguments(String),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
}
impl AppError {
    /// Stable error codes; they are the catalog's error codes.
    pub fn code(&self) -> &'static str {
        match self {
            AppError::Store(crate::store::StoreError::NotFound(_)) => "not_found",
            AppError::Store(_) => "store_error",
            AppError::InvalidArguments(_) => "invalid_arguments",
            AppError::Provider(_) => "provider_unavailable",
            AppError::NotFound(_) => "not_found",
            AppError::Empty(_) => "empty",
            AppError::BelowFloor(_) => "below_floor",
            AppError::CheckFailed(_) => "check_failed",
            AppError::Io(..) => "io_error",
        }
    }
}
pub type Result<T> = std::result::Result<T, AppError>;
pub(crate) fn git(clone: &std::path::Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(clone)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::Store(e.into())
    }
}
