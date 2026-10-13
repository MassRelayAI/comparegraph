pub mod external;
pub mod reference;

/// The `provider_ref` of a FILE node whose module-level code runs on its own (a Python script with an `if __name__ == "__main__":`
/// guard). The provider writes it; `application::graph` reads it to let a module-level call from that file rescue a callee in
/// dead-code and entry-points. Any other file node has no `provider_ref`.
pub const ENTRY_FILE_REF: &str = "entry-file";
use crate::domain::{EdgeKind, Limitation, Node};
use std::path::Path;

/// A per-file parse cache the application layer supplies; the provider never knows what stores it.
pub trait ParseCache {
    fn get(&self, file_hash: &str, language: &str, provider_version: &str) -> Option<Vec<u8>>;
    fn put_many(&self, entries: &[(String, String, String, Vec<u8>)]);
}

#[derive(Clone)]
pub struct BuildOptions {
    pub max_file_bytes: u64,
    pub max_files: usize,
    /// `build --full`: bypass the parse cache
    pub full: bool,
    pub cache: Option<std::rc::Rc<dyn ParseCache>>,
    /// The latest done run of the same provider version, for an incremental build (None: resolve everything)
    pub prev: Option<std::rc::Rc<PrevGraph>>,
}

/// What an incremental build knows of the previous run, by qualified name (row ids are per run and never carried).
#[derive(Debug, Default)]
pub struct PrevGraph {
    /// file or test node path → its content hash
    pub file_hashes: std::collections::HashMap<String, String>,
    /// path → the symbols the previous run held there, in node order: (name, kind, signature)
    pub symbols_by_file: std::collections::HashMap<String, Vec<(String, String, Option<String>)>>,
    /// qualified name → path, for every node
    pub path_of_qn: std::collections::HashMap<String, String>,
    pub edges: Vec<(EdgeKind, String, String, f64)>,
    /// the `go.mod` fingerprint the previous run recorded (`reference::incr::go_fingerprint`); empty when it had none
    pub go_fp: String,
}
impl std::fmt::Debug for BuildOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuildOptions")
            .field("max_file_bytes", &self.max_file_bytes)
            .field("max_files", &self.max_files)
            .field("full", &self.full)
            .field("cache", &self.cache.is_some())
            .field("prev", &self.prev.is_some())
            .finish()
    }
}
impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            max_file_bytes: 1_048_576,
            max_files: 50_000,
            full: false,
            cache: None,
            prev: None,
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
    pub cache_hits: usize,
    pub cache_misses: usize,
    /// incremental build: the files whose edges were recomputed (None: every edge was); communities touching one are recomputed
    pub touched_paths: Option<std::collections::HashSet<String>>,
    /// incremental build: edges copied from the previous run
    pub edges_copied: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("provider {0} is not available: {1}")]
    Unavailable(String, String),
    #[error("resolution panicked while processing {0}: {1}")]
    Panicked(String, String),
}

pub trait GraphProvider {
    fn name(&self) -> &str;
    fn version(&self) -> String;
    fn detect(&self) -> ProviderStatus;
    fn capabilities(&self) -> Capabilities;
    fn build(&self, repo_root: &Path, opts: &BuildOptions) -> Result<BuildOutput, ProviderError>;
}

/// In-process providers first; then external discovery relative to the current directory's `comparegraph.toml`.
pub fn by_name(name: &str) -> Option<Box<dyn GraphProvider>> {
    by_name_in(Path::new("."), name)
}

/// `workspace` is where `comparegraph.toml` lives (the CLI's `--root`).
pub fn by_name_in(workspace: &Path, name: &str) -> Option<Box<dyn GraphProvider>> {
    match name {
        "reference" => Some(Box::new(reference::ReferenceProvider)),
        _ => external::ExternalProvider::discover(workspace, name).map(|p| Box::new(p) as Box<dyn GraphProvider>),
    }
}
