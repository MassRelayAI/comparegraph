//! The action log beside the store (`.comparegraph/actions.jsonl`, `action/1`): every store-safety write verb appends
//! one hash-chained line, and the head (`<seq>:<hash>`) is kept in `meta.actions_head`, so a file cut short disagrees
//! with the store. `governance verify` re-walks it. A file and not a table: no `schema.sql` change (docs/decisions.md).
//!
//! Limit, stated: whoever can write both the file and the store can rewrite both; that is the trust boundary of the
//! store itself.
use super::{AppError, Result};
use crate::domain::actionlog::{self, sha256_hex, Walk};
use crate::store::{now_iso, Store};
use serde_json::{json, Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const HEAD_KEY: &str = "actions_head";
pub const ARCHIVED_PREFIX: &str = "archived:";

pub fn log_path(root: &Path) -> PathBuf {
    root.join(".comparegraph").join("actions.jsonl")
}
pub fn archive_dir(root: &Path) -> PathBuf {
    root.join(".comparegraph").join("archive")
}
pub fn manifest_path(root: &Path, repo: &str) -> PathBuf {
    archive_dir(root).join(format!("{repo}.json"))
}

/// One action, before it is sealed into the chain.
#[derive(Debug, Clone, Default)]
pub struct Action {
    pub verb: String,
    pub repo: Option<String>,
    pub by: String,
    pub why: Option<String>,
    pub forced: Option<String>,
    pub inputs: Value,
    pub bytes_before: Option<u64>,
    pub bytes_after: Option<u64>,
    pub detail: Value,
}

fn read_log(root: &Path) -> Result<String> {
    let p = log_path(root);
    match std::fs::read_to_string(&p) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(AppError::Io(p, e)),
    }
}

fn chain_broken(msg: String) -> AppError {
    AppError::structured(
        "chain_broken",
        msg,
        Some("run `comparegraph governance verify`; nothing was written".into()),
        None,
    )
}

/// The engine version stamped into every entry (the reference provider's version: crate version + parser hash).
pub fn engine() -> String {
    use crate::providers::GraphProvider;
    crate::providers::reference::ReferenceProvider.version()
}

/// Append one sealed entry. The caller holds the store's write lock (`Store::with_write_lock`), which serialises
/// writers across processes; the file line is fsynced before `meta.actions_head` moves. A broken chain or a head that
/// disagrees with the file refuses (`chain_broken`) and writes nothing.
pub fn append(store: &Store, root: &Path, a: &Action) -> Result<Value> {
    let text = read_log(root)?;
    let w = actionlog::walk(&text);
    if let Some(b) = &w.broken {
        return Err(chain_broken(format!("the action log is broken at line {}: {}", b.line, b.reason)));
    }
    let meta_head = store.meta_get(HEAD_KEY)?;
    if meta_head != w.head_text() {
        return Err(chain_broken(format!(
            "the action log head {:?} disagrees with the store's {:?} (a line was removed or the store was replaced)",
            w.head_text(),
            meta_head
        )));
    }
    let (seq, prev) = w.head.clone().map_or((1, String::new()), |(s, h)| (s + 1, h));
    let mut m = Map::new();
    m.insert("at".into(), json!(now_iso()));
    m.insert("verb".into(), json!(a.verb));
    if let Some(r) = &a.repo {
        m.insert("repo".into(), json!(r));
    }
    m.insert("by".into(), json!(a.by));
    if let Some(w) = &a.why {
        m.insert("why".into(), json!(w));
    }
    m.insert("forced".into(), a.forced.as_ref().map_or(Value::Null, |f| json!(f)));
    let mut inputs = if a.inputs.is_object() { a.inputs.clone() } else { json!({}) };
    inputs["engine"] = json!(engine());
    m.insert("inputs".into(), inputs);
    if let Some(b) = a.bytes_before {
        m.insert("bytes_before".into(), json!(b));
    }
    if let Some(b) = a.bytes_after {
        m.insert("bytes_after".into(), json!(b));
    }
    if !a.detail.is_null() {
        m.insert("detail".into(), a.detail.clone());
    }
    let entry = actionlog::seal(m, seq, &prev);
    let p = log_path(root);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| AppError::Io(d.to_path_buf(), e))?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&p).map_err(|e| AppError::Io(p.clone(), e))?;
    f.write_all(format!("{entry}\n").as_bytes()).and_then(|_| f.sync_all()).map_err(|e| AppError::Io(p.clone(), e))?;
    store.meta_set(HEAD_KEY, &format!("{seq}:{}", entry["hash"].as_str().unwrap_or("")))?;
    Ok(entry)
}

/// Append under the store's write lock (the common case: a verb whose own writes are already committed).
pub fn append_locked(store: &Store, root: &Path, a: &Action) -> Result<Value> {
    store.with_write_lock(|s| append(s, root, a))
}

/// Point `meta.actions_head` at the file's head (restore-store: a restored store carries the head of its backup time,
/// while the file kept growing). Refuses when the file chain itself is broken.
pub fn adopt_file_head(store: &Store, root: &Path) -> Result<Option<String>> {
    let w = actionlog::walk(&read_log(root)?);
    if let Some(b) = &w.broken {
        return Err(chain_broken(format!("the action log is broken at line {}: {}", b.line, b.reason)));
    }
    match w.head_text() {
        Some(h) => store.meta_set(HEAD_KEY, &h)?,
        None => store.meta_delete(HEAD_KEY)?,
    }
    Ok(w.head_text())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct VerifyReport {
    pub actions: usize,
    pub file: PathBuf,
    pub head: Option<String>,
    pub store_head: Option<String>,
    pub chain_ok: bool,
    pub broken_at: Option<actionlog::Break>,
    pub archived: usize,
    pub problems: Vec<String>,
}
impl VerifyReport {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// The walk plus the store-side checks: the head in `meta` equals the file's; every `archived:` flag has, as the last
/// archive/restore entry of its repo, an `archive` entry naming the same manifest sha256, and that manifest file still
/// hashes to it; no `restore` comes before its `archive`; no archive entry is left without its flag.
pub fn verify_with(store: &Store, root: &Path) -> Result<VerifyReport> {
    let text = read_log(root)?;
    let w: Walk = actionlog::walk(&text);
    let store_head = store.meta_get(HEAD_KEY)?;
    let mut problems = Vec::new();
    if let Some(b) = &w.broken {
        problems.push(format!("chain broken at line {} (seq {:?}): {}", b.line, b.seq, b.reason));
    } else if store_head != w.head_text() {
        problems.push(format!("head mismatch: the file ends at {:?}, the store records {:?}", w.head_text(), store_head));
    }
    // the last archive/restore entry per repo, in order
    let mut state: std::collections::BTreeMap<String, (String, Option<String>)> = Default::default();
    for e in &w.entries {
        let verb = e["verb"].as_str().unwrap_or("");
        let Some(repo) = e["repo"].as_str() else { continue };
        match verb {
            "archive" => {
                state.insert(repo.into(), ("archive".into(), e["inputs"]["manifest_sha256"].as_str().map(String::from)));
            }
            "restore" => {
                if state.get(repo).map(|s| s.0.as_str()) != Some("archive") {
                    problems.push(format!("seq {}: restore of {repo} without an archive before it", e["seq"]));
                }
                state.insert(repo.into(), ("restore".into(), None));
            }
            _ => {}
        }
    }
    let flags = store.meta_with_prefix(ARCHIVED_PREFIX)?;
    for (k, sha) in &flags {
        let repo = &k[ARCHIVED_PREFIX.len()..];
        match state.get(repo) {
            Some((v, Some(s))) if v == "archive" && s == sha => {}
            _ => problems.push(format!("{repo}: archived in the store with no matching archive entry in the log")),
        }
        let mp = manifest_path(root, repo);
        match std::fs::read(&mp) {
            Ok(b) if sha256_hex(&b) == *sha => {}
            Ok(_) => problems.push(format!("{repo}: manifest {} no longer hashes to the archived flag", mp.display())),
            Err(_) => problems.push(format!("{repo}: manifest {} is missing", mp.display())),
        }
    }
    for (repo, (v, _)) in &state {
        if v == "archive" && !flags.iter().any(|(k, _)| &k[ARCHIVED_PREFIX.len()..] == repo) {
            problems.push(format!("{repo}: the log says archived, the store has no flag"));
        }
    }
    Ok(VerifyReport {
        actions: w.entries.len(),
        file: log_path(root),
        head: w.head_text(),
        store_head,
        chain_ok: w.broken.is_none(),
        broken_at: w.broken.clone(),
        archived: flags.len(),
        problems,
    })
}

/// `governance verify`: reads only (the store is opened read-only).
pub fn verify(root: &Path) -> Result<VerifyReport> {
    let store = Store::open_read_only(root)?;
    verify_with(&store, root)
}
