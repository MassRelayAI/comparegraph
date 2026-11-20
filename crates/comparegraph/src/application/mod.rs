pub mod absent;
pub mod add;
pub mod agreement;
pub mod archive;
pub mod backup;
pub mod build;
pub mod backfill;
pub mod catalog;
pub mod check;
pub mod citation;
pub mod claim;
pub mod communities;
pub mod conform;
pub mod decide;
pub mod delta;
pub mod export;
pub mod gitref;
pub mod governance;
pub mod graph;
pub mod health;
pub mod family;
pub mod lines;
pub mod map;
pub mod matrix;
pub mod propose;
pub mod reach;
pub mod refgc;
pub mod relevance;
pub mod retain;
pub mod search;
pub mod setup;
pub mod show;
pub mod termscore;
pub mod output_schema;
pub mod ui;
pub mod ui_graph;
pub mod ui_limits;
pub mod ui_net;
pub mod ui_owners;
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
    /// A `claim none` the engine will not record: the message for people, `error.data` (the `NoneRefusal`) for callers.
    #[error("{}", .0.message)]
    NoneRefused(Box<absent::NoneRefusal>),
    /// An error that carries its recovery: a machine-readable `data` (selector, candidates, drift list ...) and a `hint` naming the next step.
    #[error("{}", .0.message)]
    Structured(Box<StructuredError>),
}
/// The payload of [`AppError::Structured`] (boxed: the enum stays small).
#[derive(Debug)]
pub struct StructuredError {
    pub code: &'static str,
    pub message: String,
    pub hint: Option<String>,
    pub data: Option<serde_json::Value>,
}
impl AppError {
    pub fn structured(code: &'static str, message: String, hint: Option<String>, data: Option<serde_json::Value>) -> Self {
        AppError::Structured(Box::new(StructuredError { code, message, hint, data }))
    }
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
            AppError::NoneRefused(_) => "none_refused",
            AppError::Structured(s) => s.code,
        }
    }
    /// The recovery step for a caller (envelope `error.hint`); only errors that know their recovery have one.
    pub fn hint(&self) -> Option<String> {
        match self {
            AppError::Structured(s) => s.hint.clone(),
            _ => None,
        }
    }
    /// A `not_found` that keeps what was asked for and what exists: `data = {selector, candidates}`.
    pub fn not_found_with(message: String, selector: &str, candidates: Vec<String>, hint: &str) -> Self {
        AppError::structured("not_found", message, Some(hint.into()), Some(serde_json::json!({ "selector": selector, "candidates": candidates })))
    }
    /// A `check_failed` with a recovery hint.
    pub fn check_failed_hint(message: String, hint: &str) -> Self {
        AppError::structured("check_failed", message, Some(hint.into()), None)
    }
    /// Structured detail an envelope carries as `error.data` (only a `none_refused` has one).
    pub fn data(&self) -> Option<serde_json::Value> {
        match self {
            AppError::NoneRefused(r) => serde_json::to_value(r).ok(),
            AppError::Structured(s) => s.data.clone(),
            _ => None,
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
/// The full commit id a (possibly short) rev names in the clone's objects; None when the object is not there.
pub(crate) fn git_commit_id(clone: &std::path::Path, rev: &str) -> Option<String> {
    git(clone, &["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")])
}
/// One `git cat-file --batch` process over a clone: many committed files read without one process per file. A path is
/// relative to the clone, which may be a subfolder of a larger git repo, hence `<commit>:./<path>`.
pub(crate) struct BlobReader {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: std::io::BufReader<std::process::ChildStdout>,
}
impl BlobReader {
    pub(crate) fn open(clone: &std::path::Path) -> Option<BlobReader> {
        let mut child = std::process::Command::new("git")
            .arg("-C")
            .arg(clone)
            .args(["cat-file", "--batch"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        let stdout = std::io::BufReader::new(child.stdout.take()?);
        Some(BlobReader { child, stdin, stdout })
    }
    /// The committed bytes of `path` at `commit`; None when either is missing or the object is not a file.
    pub(crate) fn read(&mut self, commit: &str, path: &str) -> Option<Vec<u8>> {
        use std::io::{BufRead, Read, Write};
        if path.contains(['\n', '\r']) {
            return None;
        }
        writeln!(self.stdin, "{commit}:./{path}").ok()?;
        self.stdin.flush().ok()?;
        let mut head = String::new();
        self.stdout.read_line(&mut head).ok()?;
        let mut it = head.split_whitespace();
        let (_, kind, size) = (it.next()?, it.next()?, it.next()?);
        if kind != "blob" {
            // `missing` and `ambiguous` have no body; a tree or commit has one that must be drained
            if matches!(kind, "tree" | "commit" | "tag") {
                let n: u64 = size.parse().ok()?;
                std::io::copy(&mut (&mut self.stdout).take(n + 1), &mut std::io::sink()).ok()?;
            }
            return None;
        }
        let n: usize = size.parse().ok()?;
        let mut buf = vec![0u8; n + 1];
        self.stdout.read_exact(&mut buf).ok()?;
        buf.pop();
        Some(buf)
    }
}
impl Drop for BlobReader {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::Store(e.into())
    }
}
