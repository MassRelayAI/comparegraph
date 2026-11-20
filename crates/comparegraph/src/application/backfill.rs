//! `backfill-head`: pin a finished run that recorded no git head (it predates the `head` column) to a commit, by proof.
//!
//! The candidate is the repo's recorded head, resolved to a full commit in the clone. Every location of every claim that
//! cites the run is hashed at that commit; only when ALL of them equal the recorded content hash is `run.head` set (and
//! `run.head_source = 'inferred-by-hash'`, plus an action-log entry). One mismatch, no locations to prove, no recorded
//! head, or a commit missing from the clone leaves the run NULL and names it as unverifiable. Never a guess (CG-1, D451).
//! Dry run by default; `--apply` needs `--by`.
use super::governance::{self, Action};
use super::{AppError, Result};
use crate::store::Store;
use serde_json::{json, Value};
use std::path::Path;

pub fn backfill_head(root: &Path, repo: Option<&str>, apply: bool, by: &str) -> Result<Value> {
    if apply && by.trim().is_empty() {
        return Err(AppError::InvalidArguments("--by is required with --apply".into()));
    }
    let store = if apply { Store::open_guarded(root)? } else { Store::open_read_only(root)? };
    let archived = super::archive::archived_names(&store)?;
    let repos = store.repos()?;
    if let Some(n) = repo {
        if !repos.iter().any(|r| r.name == n) {
            return Err(AppError::NotFound(format!("no repo named {n}")));
        }
    }
    let mut would_pin: Vec<(i64, String, String, String, usize)> = vec![]; // run, repo, short, full, locations
    let mut unverifiable: Vec<Value> = vec![];
    let mut examined = 0usize;
    for (run, repo_id) in store.runs_without_head()? {
        let Some(r) = repos.iter().find(|r| r.id == Some(repo_id)) else { continue };
        if archived.contains(&r.name) || repo.is_some_and(|n| n != r.name) {
            continue;
        }
        examined += 1;
        let locs = store.locations_of_run(run)?;
        let why = |reason: &str, mismatched: usize| {
            json!({ "run": run, "repo": r.name, "reason": reason, "locations": locs.len(), "mismatched": mismatched })
        };
        if locs.is_empty() {
            unverifiable.push(why("no_locations", 0));
            continue;
        }
        let clone = Path::new(&r.path);
        let Some(recorded) = r.head.as_deref().filter(|h| !h.trim().is_empty()) else {
            unverifiable.push(why("repo_has_no_recorded_head", 0));
            continue;
        };
        if !clone.is_dir() {
            unverifiable.push(why("clone_missing", 0));
            continue;
        }
        let Some(full) = super::git_commit_id(clone, recorded) else {
            unverifiable.push(why("commit_unavailable", 0));
            continue;
        };
        let Some(mut blobs) = super::BlobReader::open(clone) else {
            unverifiable.push(why("git_unavailable", 0));
            continue;
        };
        let mut mismatched = 0usize;
        for (path, s, e, hash) in &locs {
            let ok = blobs
                .read(&full, path)
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .and_then(|t| super::lines::span_hash(&t, *s as usize, *e as usize))
                .is_some_and(|h| &h == hash);
            if !ok {
                mismatched += 1;
            }
        }
        if mismatched > 0 {
            unverifiable.push(why("hash_mismatch", mismatched));
            continue;
        }
        let short: String = full.chars().take(7).collect();
        would_pin.push((run, r.name.clone(), short, full, locs.len()));
    }
    let item = |p: &(i64, String, String, String, usize)| json!({ "run": p.0, "repo": p.1, "commit": p.3, "head": p.2, "locations_proven": p.4 });
    let mut v = json!({
        "dry_run": !apply,
        "runs_examined": examined,
        "unverifiable": unverifiable,
    });
    if !apply {
        v["would_pin"] = json!(would_pin.iter().map(item).collect::<Vec<_>>());
        return Ok(v);
    }
    if would_pin.is_empty() {
        v["pinned"] = json!([]);
        return Ok(v);
    }
    let bytes = store.file_bytes();
    let entry = store.with_write_lock(|s| -> Result<Value> {
        for p in &would_pin {
            s.set_run_head_inferred(p.0, &p.2)?;
        }
        governance::append(s, root, &Action {
            verb: "backfill-head".into(),
            by: by.into(),
            inputs: json!({ "runs": would_pin.iter().map(item).collect::<Vec<_>>(), "unverifiable": unverifiable.len() }),
            bytes_before: Some(bytes),
            ..Default::default()
        })
    })?;
    v["pinned"] = json!(would_pin.iter().map(item).collect::<Vec<_>>());
    v["action_seq"] = entry["seq"].clone();
    Ok(v)
}
