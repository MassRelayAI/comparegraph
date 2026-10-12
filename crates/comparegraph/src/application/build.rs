use super::{AppError, Result};
use crate::domain::{Edge, Limitation, NodeId, Run, RunId, RunStatus};
use crate::providers::{by_name, BuildOptions};
use crate::store::{now_iso, Store};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

#[derive(Debug, Clone, serde::Serialize)]
pub struct BuildReport {
    pub run: RunId,
    pub nodes: usize,
    pub edges: usize,
    pub files_parsed: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub limitations: usize,
    pub communities: usize,
    /// carried to the envelope's evidence.limitations by the CLI
    pub limitation_items: Vec<Limitation>,
    pub store_hash: String,
    pub seconds: f64,
}

/// The workspace db as the provider's parse cache (its own connection; written only after the parse loop).
struct StoreCache(Store);
impl crate::providers::ParseCache for StoreCache {
    fn get(&self, h: &str, l: &str, v: &str) -> Option<Vec<u8>> {
        self.0.parse_cache_get(h, l, v)
    }
    fn put_many(&self, entries: &[(String, String, String, Vec<u8>)]) {
        self.0.parse_cache_put_many(entries);
    }
}

pub fn build_repo(
    root: &Path,
    repo_name: &str,
    provider_name: &str,
    opts: &BuildOptions,
) -> Result<BuildReport> {
    let store = Store::open(root)?;
    let repo = store.repo_by_name(repo_name)?.ok_or_else(|| {
        AppError::NotFound(format!("no repo '{repo_name}'; run `comparegraph add` first"))
    })?;
    let provider = by_name(provider_name)
        .ok_or_else(|| AppError::NotFound(format!("no provider '{provider_name}'")))?;
    let status = provider.detect();
    if !status.installed {
        return Err(AppError::Provider(
            crate::providers::ProviderError::Unavailable(provider_name.into(), status.detail),
        ));
    }
    let repo_id = repo.id.unwrap();
    let t = Instant::now();
    let run = store.insert_run(&Run {
        id: None,
        repo: repo_id,
        provider: provider.name().into(),
        provider_version: provider.version(),
        started_at: now_iso(),
        seconds: 0.0,
        status: RunStatus::Running,
        limitations: vec![],
        store_hash: None,
    })?;
    let mut opts = opts.clone();
    if opts.full {
        opts.cache = None;
    } else if opts.cache.is_none() {
        opts.cache = Some(std::rc::Rc::new(StoreCache(Store::open(root)?)));
    }
    let out = match provider.build(Path::new(&repo.path), &opts) {
        Ok(o) => o,
        Err(e) => {
            store.finish_run(
                run,
                RunStatus::Failed,
                t.elapsed().as_secs_f64(),
                &[Limitation::new("provider_error", e.to_string())],
                None,
            )?;
            return Err(e.into());
        }
    };
    if out.files_parsed == 0 {
        let msg = format!(
            "0 files parsed under {} ({} files seen, {} limitations)",
            repo.path,
            out.files_seen,
            out.limitations.len()
        );
        store.finish_run(
            run,
            RunStatus::Failed,
            t.elapsed().as_secs_f64(),
            &[Limitation::new("empty_build", msg.clone())],
            None,
        )?;
        return Err(AppError::Empty(msg));
    }
    let stored = (|| -> Result<(usize, usize, usize, String, f64)> {
        let mut nodes = out.nodes;
        store.insert_nodes(repo_id, run, &mut nodes)?;
        let ids: HashMap<&str, NodeId> = nodes
            .iter()
            .map(|n| (n.qualified_name.as_str(), n.id.unwrap()))
            .collect();
        let edges: Vec<Edge> = out
            .edges_by_qn
            .iter()
            .filter_map(|(k, s, t2, c)| {
                Some(Edge {
                    kind: *k,
                    source: *ids.get(s.as_str())?,
                    target: *ids.get(t2.as_str())?,
                    confidence: *c,
                })
            })
            .collect();
        store.insert_edges(repo_id, run, &edges)?;
        let comms = super::communities::compute(&nodes, &edges);
        store.insert_communities(repo_id, run, &comms)?;
        let hash = store.store_hash(run)?;
        let secs = t.elapsed().as_secs_f64();
        store.finish_run(run, RunStatus::Done, secs, &out.limitations, Some(&hash))?;
        Ok((nodes.len(), edges.len(), comms.len(), hash, secs))
    })();
    let (n_nodes, n_edges, n_comms, hash, secs) = match stored {
        Ok(v) => v,
        Err(e) => {
            let _ = store.finish_run(
                run,
                RunStatus::Failed,
                t.elapsed().as_secs_f64(),
                &[Limitation::new("store_error", e.to_string())],
                None,
            );
            return Err(e);
        }
    };
    Ok(BuildReport {
        run,
        nodes: n_nodes,
        edges: n_edges,
        files_parsed: out.files_parsed,
        cache_hits: out.cache_hits,
        cache_misses: out.cache_misses,
        limitations: out.limitations.len(),
        communities: n_comms,
        limitation_items: out.limitations,
        store_hash: hash,
        seconds: secs,
    })
}
