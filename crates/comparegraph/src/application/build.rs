use super::{AppError, Result};
use crate::domain::{Edge, Limitation, NodeId, Run, RunId, RunStatus};
use crate::providers::{by_name_in, BuildOptions};
use crate::store::{now_iso, Store};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BuildReport {
    pub run: RunId,
    /// the clone path this build read
    #[serde(default)]
    pub path: String,
    /// the git head (short) of that path when the build ran
    #[serde(default)]
    pub head: Option<String>,
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
    /// true when nothing changed since the latest done run and that run was returned (no new rows)
    #[serde(default)]
    pub reused: bool,
    /// incremental build: edges copied from the previous run instead of resolved again
    #[serde(default)]
    pub edges_copied: usize,
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
    let provider = by_name_in(root, provider_name)
        .ok_or_else(|| AppError::NotFound(format!("no provider '{provider_name}'")))?;
    let status = provider.detect();
    if !status.installed {
        return Err(AppError::Provider(
            crate::providers::ProviderError::Unavailable(provider_name.into(), status.detail),
        ));
    }
    if provider_name != "reference" {
        if let Some(ext) = crate::providers::external::ExternalProvider::discover(root, provider_name) {
            // the languages the walker finds in the clone, computed once; the conform gate judges only those
            let langs: std::collections::BTreeSet<String> = crate::providers::reference::walker::walk(Path::new(&repo.path), opts)
                .files
                .iter()
                .map(|f| crate::providers::external::fixture_name(&f.language).to_string())
                .collect();
            ext.conformance_gate(&langs)
                .map_err(|e| AppError::Provider(crate::providers::ProviderError::Unavailable(provider_name.into(), e)))?;
        }
    }
    let repo_id = repo.id.unwrap();
    let t = Instant::now();
    if !opts.full && provider.name() == "reference" {
        if let Some(r) = try_reuse(&store, repo_id, &repo.path, &provider.version(), opts, t)? {
            return Ok(r);
        }
    }
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
    let mut prev_comms: Option<PrevComms> = None;
    if opts.full {
        opts.cache = None;
        opts.prev = None;
    } else if provider.name() == "reference" && opts.prev.is_none() {
        let tp = Instant::now();
        let loaded = load_prev(&store, repo_id, &provider.version())?;
        crate::providers::reference::profile_line("load_prev", tp, loaded.as_ref().map_or(0, |l| l.0.edges.len()));
        if let Some((g, c)) = loaded {
            opts.prev = Some(std::rc::Rc::new(g));
            prev_comms = Some(c);
        }
    }
    if !opts.full && opts.cache.is_none() {
        opts.cache = Some(std::rc::Rc::new(StoreCache(Store::open(root)?)));
    }
    if opts.progress {
        eprintln!("[progress] reading {} head={}", repo.path, super::git(Path::new(&repo.path), &["rev-parse", "--short=7", "HEAD"]).unwrap_or_else(|| "none".into()));
    }
    let out = match provider.build(Path::new(&repo.path), &opts) {
        Ok(o) => o,
        Err(e @ crate::providers::ProviderError::Unavailable(..)) => {
            store.finish_run(run, RunStatus::Failed, t.elapsed().as_secs_f64(), &[Limitation::new("provider_unavailable", e.to_string())], None)?;
            return Err(e.into());
        }
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
    let mut out = out;
    out.limitations = collapse_limitations(std::mem::take(&mut out.limitations));
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
        let tp = std::time::Instant::now();
        store.insert_nodes(repo_id, run, &mut nodes)?;
        crate::providers::reference::profile_line("store.insert_nodes", tp, nodes.len());
        let tp = std::time::Instant::now();
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
        crate::providers::reference::profile_line("store.map_edges", tp, edges.len());
        let tp = std::time::Instant::now();
        store.insert_edges(repo_id, run, &edges)?;
        crate::providers::reference::profile_line("store.insert_edges", tp, edges.len());
        let tp = std::time::Instant::now();
        let comms = match (&out.touched_paths, &prev_comms) {
            (Some(touched), Some(pc)) => {
                super::communities::compute_with_prev(&nodes, &edges, touched, &pc.by_files)
            }
            _ => super::communities::compute(&nodes, &edges),
        };
        crate::providers::reference::profile_line("communities", tp, comms.len());
        let tp = std::time::Instant::now();
        store.insert_communities(repo_id, run, &comms)?;
        crate::providers::reference::profile_line("store.insert_comms", tp, comms.len());
        let tp = std::time::Instant::now();
        let hash = store.store_hash(run)?;
        crate::providers::reference::profile_line("store.store_hash", tp, 0);
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
        path: repo.path.clone(),
        head: super::git(Path::new(&repo.path), &["rev-parse", "--short=7", "HEAD"]),
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
        reused: false,
        edges_copied: out.edges_copied,
    })
}

/// The previous community rows, keyed by the sorted paths of their files.
struct PrevComms {
    by_files: HashMap<Vec<String>, (Vec<String>, Option<f64>)>,
}

/// The latest done run of the same provider version as the base of an incremental build.
fn load_prev(
    store: &Store,
    repo_id: crate::domain::RepoId,
    version: &str,
) -> Result<Option<(crate::providers::PrevGraph, PrevComms)>> {
    use crate::domain::{EdgeKind, NodeKind};
    let Some(last) = store.latest_done_run_of(repo_id, "reference", version)? else {
        return Ok(None);
    };
    let Some(last_id) = last.id else {
        return Ok(None);
    };
    let nodes = store.nodes_of_run(last_id)?;
    let mut g = crate::providers::PrevGraph {
        go_fp: go_fp_of(&last),
        ..Default::default()
    };
    let mut qn_of: HashMap<NodeId, &str> = HashMap::new();
    let mut path_of_id: HashMap<NodeId, &str> = HashMap::new();
    for n in &nodes {
        let id = n.id.unwrap();
        qn_of.insert(id, n.qualified_name.as_str());
        path_of_id.insert(id, n.path.as_str());
        g.path_of_qn.insert(n.qualified_name.clone(), n.path.clone());
        if matches!(n.kind, NodeKind::File | NodeKind::Test) {
            g.file_hashes.insert(n.path.clone(), n.content_hash.clone());
        } else {
            g.symbols_by_file.entry(n.path.clone()).or_default().push((
                n.name.clone(),
                n.kind.as_str().to_string(),
                n.provider_ref.clone(),
            ));
        }
    }
    for e in store.edges_of_run(last_id)? {
        if matches!(e.kind, EdgeKind::Contains) {
            continue;
        }
        if let (Some(s), Some(t)) = (qn_of.get(&e.source), qn_of.get(&e.target)) {
            g.edges.push((e.kind, s.to_string(), t.to_string(), e.confidence));
        }
    }
    let mut by_files = HashMap::new();
    for c in store.communities_of_run(last_id)? {
        let mut paths: Vec<String> = c
            .members
            .iter()
            .filter_map(|m| path_of_id.get(m))
            .map(|p| p.to_string())
            .collect();
        paths.sort();
        paths.dedup();
        by_files.insert(paths, (c.keywords, c.cohesion));
    }
    Ok(Some((g, PrevComms { by_files })))
}

/// The no-op rule: the (path, content hash) set the walker would parse equals the latest done run's file nodes, and the provider
/// version is the same. Nothing is parsed; a file the build would skip (unreadable, not UTF-8, minified) is skipped here too.
fn try_reuse(
    store: &Store,
    repo_id: crate::domain::RepoId,
    repo_path: &str,
    version: &str,
    opts: &BuildOptions,
    t: Instant,
) -> Result<Option<BuildReport>> {
    use crate::domain::{content_hash, NodeKind};
    use crate::providers::reference::walker;
    let Some(last) = store.latest_done_run_of(repo_id, "reference", version)? else {
        return Ok(None);
    };
    let Some(last_id) = last.id else {
        return Ok(None);
    };
    let nodes = store.nodes_of_run(last_id)?;
    let mut recorded: HashMap<&str, &str> = HashMap::new();
    for n in nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::File | NodeKind::Test))
    {
        recorded.insert(n.path.as_str(), n.content_hash.as_str());
    }
    let root = Path::new(repo_path);
    let walked = walker::walk(root, opts);
    // go.mod files are read by Go import resolution but are not parsed files: a changed one makes the recorded graph stale
    if crate::providers::reference::incr::go_fingerprint(root, &walked.go_mods) != go_fp_of(&last) {
        return Ok(None);
    }
    let mut seen = 0usize;
    for f in &walked.files {
        let Ok(bytes) = std::fs::read(root.join(&f.rel_path)) else {
            continue;
        };
        let Ok(source) = String::from_utf8(bytes) else {
            continue;
        };
        if source.lines().any(|l| l.len() > walker::MINIFIED_LINE_CHARS) {
            continue;
        }
        let lines: Vec<&str> = source.lines().collect();
        match recorded.get(f.rel_path.as_str()) {
            Some(h) if *h == content_hash(&lines) => seen += 1,
            _ => return Ok(None),
        }
    }
    if seen != recorded.len() {
        return Ok(None);
    }
    let edges = store.edges_of_run(last_id)?.len();
    let communities = store.communities_of_run(last_id)?.len();
    Ok(Some(BuildReport {
        run: last_id,
        path: repo_path.to_string(),
        head: super::git(root, &["rev-parse", "--short=7", "HEAD"]),
        nodes: nodes.len(),
        edges,
        files_parsed: seen,
        cache_hits: 0,
        cache_misses: 0,
        limitations: last.limitations.len(),
        communities,
        limitation_items: last.limitations.clone(),
        store_hash: last.store_hash.clone().unwrap_or_default(),
        seconds: t.elapsed().as_secs_f64(),
        reused: true,
        edges_copied: 0,
    }))
}

/// `build --providers a,b,...`: every provider builds the same clone in the order given; the first run is the baseline and each
/// later run is scored against it (`agreement`). The agreement rows below 0.8 are added to that later run's limitations.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MultiBuildReport {
    pub runs: Vec<BuildReport>,
    pub providers: Vec<String>,
    pub agreement: Vec<super::agreement::AgreementRow>,
    /// the last run's limitations, carried to the envelope
    pub limitation_items: Vec<Limitation>,
}

/// A limitation code that repeats more than this many times in one build is stored once, with its count.
const COLLAPSE_ABOVE: usize = 20;
/// Codes whose detail is parsed elsewhere (the Go module fingerprint, the incremental plan) or is a single verdict; never collapsed.
const NEVER_COLLAPSED: &[&str] = &["go_modules", "incremental_fallback", "provider_unavailable", "provider_error", "store_error"];

/// One place where a build's limitations are finalised (before `finish_run` and before the envelope): a code repeated more than
/// 20 times becomes ONE limitation, `"<n> occurrences; first: <d1>, … <d5>, …"`, at the position of its first occurrence.
pub fn collapse_limitations(items: Vec<Limitation>) -> Vec<Limitation> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for l in &items {
        *counts.entry(l.code.as_str()).or_default() += 1;
    }
    let repeated: std::collections::HashSet<String> = counts
        .into_iter()
        .filter(|(c, n)| *n > COLLAPSE_ABOVE && !NEVER_COLLAPSED.contains(c) && !c.starts_with("agreement"))
        .map(|(c, _)| c.to_string())
        .collect();
    if repeated.is_empty() {
        return items;
    }
    let mut seen: std::collections::HashMap<String, (usize, Vec<String>)> = std::collections::HashMap::new();
    for l in &items {
        if repeated.contains(&l.code) {
            let e = seen.entry(l.code.clone()).or_default();
            e.0 += 1;
            if e.1.len() < 5 {
                e.1.push(l.detail.clone());
            }
        }
    }
    let mut done: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for l in items {
        if !repeated.contains(&l.code) {
            out.push(l);
        } else if done.insert(l.code.clone()) {
            let (n, first) = &seen[&l.code];
            out.push(Limitation::new(&l.code, format!("{n} occurrences; first: {}, …", first.join(", "))));
        }
    }
    out
}

pub fn build_many(root: &Path, repo_name: &str, providers: &[String], opts: &BuildOptions) -> Result<MultiBuildReport> {
    if providers.is_empty() {
        return Err(AppError::InvalidArguments("--providers needs at least one provider name".into()));
    }
    // validate every name before anything is built, so an unknown name leaves the store untouched
    for p in providers {
        if by_name_in(root, p).is_none() {
            return Err(AppError::NotFound(format!("no provider '{p}'")));
        }
    }
    let mut runs: Vec<BuildReport> = Vec::new();
    let mut agreement = Vec::new();
    for p in providers {
        let mut rep = build_repo(root, repo_name, p, opts)?;
        if let Some(base) = runs.first() {
            let store = Store::open(root)?;
            let rows = super::agreement::compute(&store, base.run, rep.run, &providers[0], p)?;
            let lims = super::agreement::limitations(&rows);
            if !lims.is_empty() {
                rep.limitation_items.extend(lims);
                rep.limitations = rep.limitation_items.len();
                store.set_run_limitations(rep.run, &rep.limitation_items)?;
            }
            agreement.extend(rows);
        }
        runs.push(rep);
    }
    let limitation_items = runs.last().map(|r| r.limitation_items.clone()).unwrap_or_default();
    Ok(MultiBuildReport { runs, providers: providers.to_vec(), agreement, limitation_items })
}

/// The `go.mod` fingerprint a run recorded (empty when it had none).
fn go_fp_of(run: &Run) -> String {
    run.limitations
        .iter()
        .find(|l| l.code == crate::providers::reference::incr::GO_MODULES)
        .map(|l| l.detail.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod collapse_tests {
    use super::*;
    #[test]
    fn only_repeated_unprotected_codes_collapse() {
        let mut v = vec![Limitation::new("go_modules", "fp"), Limitation::new("skipped_dir", "node_modules")];
        v.extend((0..21).map(|i| Limitation::new("clamped_line_end", format!("f{i}.ts:x 2 → 1"))));
        v.extend((0..30).map(|i| Limitation::new("go_modules", format!("m{i}"))));
        v.extend((0..19).map(|i| Limitation::new("skipped_dir", format!("d{i}"))));
        let c = collapse_limitations(v);
        let cl: Vec<_> = c.iter().filter(|l| l.code == "clamped_line_end").collect();
        assert_eq!(cl.len(), 1);
        assert_eq!(cl[0].detail, "21 occurrences; first: f0.ts:x 2 → 1, f1.ts:x 2 → 1, f2.ts:x 2 → 1, f3.ts:x 2 → 1, f4.ts:x 2 → 1, …");
        assert_eq!(c.iter().filter(|l| l.code == "go_modules").count(), 31);
        assert_eq!(c.iter().filter(|l| l.code == "skipped_dir").count(), 20);
        assert_eq!(c[0].detail, "fp");
    }
}
