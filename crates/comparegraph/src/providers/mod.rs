pub mod reference;
use crate::domain::{EdgeKind, Limitation, Node};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub max_file_bytes: u64,
    pub max_files: usize,
}
impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            max_file_bytes: 1_048_576,
            max_files: 50_000,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderStatus {
    pub installed: bool,
    pub version: String,
    pub detail: String,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct Capabilities {
    pub languages: Vec<String>,
    pub edge_kinds: Vec<EdgeKind>,
    pub communities: bool,
    pub embeddings: bool,
}
#[derive(Debug, Default)]
pub struct BuildOutput {
    pub nodes: Vec<Node>,
    pub edges_by_qn: Vec<(EdgeKind, String, String, f64)>,
    pub limitations: Vec<Limitation>,
    pub files_seen: usize,
    pub files_parsed: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("provider {0} is not available: {1}")]
    Unavailable(String, String),
}

pub trait GraphProvider {
    fn name(&self) -> &str;
    fn version(&self) -> String;
    fn detect(&self) -> ProviderStatus;
    fn capabilities(&self) -> Capabilities;
    fn build(&self, repo_root: &Path, opts: &BuildOptions) -> Result<BuildOutput, ProviderError>;
}

pub fn by_name(name: &str) -> Option<Box<dyn GraphProvider>> {
    match name {
        "reference" => Some(Box::new(reference::ReferenceProvider)),
        _ => None,
    }
}
