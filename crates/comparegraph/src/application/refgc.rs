//! CG-REFGC: refs move, so the runs built from them go stale, and nothing may answer silently from old data.
//! - `status_of`: `current` (the ref still names the commit the run read), `moved` (it names another), `gone` (it names nothing).
//! - `stale_limitation`: what a query verb that answered from a ref's run adds to its envelope when the run is not current.
//! - `gc_refs`: removes ref runs (their graph rows) and their extracted trees when the ref is gone, a newer run of the same ref
//!   supersedes them (keep the latest N), or they are older than M days. A dry run unless `--apply --by <who>`; every apply is one
//!   action-log line. It never touches a head run or the clone, and refuses outright while a live claim cites a run it would remove.
use super::{gitref, governance, AppError, Result};
use crate::domain::relevance::parse_ts_secs;
use crate::store::{now_iso, Store};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

/// A claim never cites a ref run: claims record a repository's head, a ref run is only for `delta` (and `decide --ref` records the verdict).
pub fn claim_on_ref_refused(git_ref: &str) -> AppError {
    AppError::structured(
        "invalid_arguments",
        "claims record a repository's head; a ref run (build --ref) is only for delta".to_string(),
        Some("drop --ref (the claim is made on the head run), or record the branch verdict with `decide --ref <ref> --commit <sha>`".into()),
        Some(json!({ "ref": git_ref })),
    )
}

/// `current`, `moved` or `gone`.
pub fn status_of(clone: &Path, git_ref: &str, commit: &str) -> &'static str {
    match gitref::resolve(clone, git_ref) {
        Ok(c) if c == commit => "current",
        Ok(_) => "moved",
        Err(_) => "gone",
    }
}

fn short(c: &str) -> String {
    c.chars().take(7).collect()
}

/// The `stale` limitation for a query that answered from the latest run built from `git_ref`, if that run is not current.
pub fn stale_limitation(store: &Store, repo: &str, git_ref: &str) -> Option<Value> {
    let r = store.repo_by_name(repo).ok()??;
    let rows = store.ref_run_rows(r.id?).ok()?;
    let row = rows.iter().find(|x| x.git_ref == git_ref || (git_ref.len() >= 7 && x.commit.starts_with(git_ref)))?;
    let now = gitref::resolve(Path::new(&r.path), &row.git_ref);
    let (state, detail) = match now {
        Ok(c) if c == row.commit => return None,
        Ok(c) => ("moved", format!("stale: moved - the answer is from run {} built at {}, but '{}' now names {}; rebuild with `build {repo} --ref {}`", row.id, short(&row.commit), row.git_ref, short(&c), row.git_ref)),
        Err(_) => ("gone", format!("stale: gone - the answer is from run {} built at {}, but '{}' no longer names a commit; `gc --refs` removes the run", row.id, short(&row.commit), row.git_ref)),
    };
    Some(json!({ "code": "stale", "detail": detail, "data": { "state": state, "ref": row.git_ref, "run": row.id, "built_at": row.commit } }))
}

/// Delete one ref run's graph rows. The delete triggers allow exactly this (`run.ref IS NOT NULL`, see schema.sql); none is dropped.
fn delete_runs(store: &Store, ids: &[i64]) -> Result<()> {
    let c = store.conn();
    for id in ids {
        c.execute("DELETE FROM community_member WHERE community_id IN (SELECT id FROM community WHERE run_id=?1)", [id])?;
        c.execute("DELETE FROM community WHERE run_id=?1", [id])?;
        c.execute("DELETE FROM edge WHERE run_id=?1", [id])?;
        c.execute("DELETE FROM node_fts WHERE rowid IN (SELECT id FROM node WHERE run_id=?1)", [id])?;
        c.execute("DELETE FROM embedding WHERE owner_kind='node' AND owner_id IN (SELECT id FROM node WHERE run_id=?1)", [id])?;
        c.execute("DELETE FROM node WHERE run_id=?1", [id])?;
        c.execute("DELETE FROM run WHERE id=?1", [id])?;
    }
    Ok(())
}

pub fn gc_refs(root: &Path, keep: usize, older_than_days: Option<u64>, apply: bool, by: &str) -> Result<Value> {
    if keep == 0 {
        return Err(AppError::InvalidArguments("--keep must be at least 1".into()));
    }
    if older_than_days == Some(0) {
        return Err(AppError::InvalidArguments("--older-than-days must be at least 1".into()));
    }
    if apply && by.trim().is_empty() {
        return Err(AppError::InvalidArguments("--by is required with --apply".into()));
    }
    let store = if apply { Store::open_guarded(root)? } else { Store::open_read_only(root)? };
    let now = parse_ts_secs(&now_iso()).unwrap_or(0);
    let mut cands: Vec<Value> = vec![];
    let mut cited: Vec<Value> = vec![];
    let mut kept = 0usize;
    let mut ids: Vec<i64> = vec![];
    let mut trees: BTreeMap<(String, String), bool> = BTreeMap::new(); // (repo, commit) -> still used by a run that stays
    for repo in store.repos()? {
        let rid = repo.id.expect("stored repo has id");
        let rows = store.ref_run_rows(rid)?;
        let mut per_ref: BTreeMap<String, usize> = BTreeMap::new();
        for row in &rows {
            // rows come newest first within a ref
            let rank = per_ref.entry(row.git_ref.clone()).or_default();
            *rank += 1;
            let mut why = vec![];
            if status_of(Path::new(&repo.path), &row.git_ref, &row.commit) == "gone" {
                why.push("gone".to_string());
            }
            if *rank > keep {
                why.push(format!("superseded (keeping the latest {keep})"));
            }
            if let (Some(days), Some(t)) = (older_than_days, parse_ts_secs(&row.started_at)) {
                if now - t > days as i64 * 86_400 {
                    why.push(format!("older than {days} days"));
                }
            }
            if why.is_empty() {
                kept += 1;
                trees.insert((repo.name.clone(), row.commit.clone()), true);
                continue;
            }
            let citations: i64 = store.conn().query_row("SELECT COUNT(*) FROM claim WHERE run_id=?1", [row.id], |r| r.get(0))?;
            let rel: i64 = store.conn().query_row("SELECT COUNT(*) FROM relatedness WHERE run_id=?1", [row.id], |r| r.get(0))?;
            let item = json!({ "run": row.id, "repo": repo.name, "ref": row.git_ref, "ref_commit": row.commit, "started_at": row.started_at, "why": why });
            if citations + rel > 0 {
                cited.push(json!({ "run": row.id, "repo": repo.name, "ref": row.git_ref, "claims": citations, "relatedness_rows": rel }));
            }
            trees.entry((repo.name.clone(), row.commit.clone())).or_insert(false);
            ids.push(row.id);
            cands.push(item);
        }
    }
    let mut base = json!({
        "keep": keep, "older_than_days": older_than_days, "candidates": cands, "kept": kept,
        "cited": cited,
    });
    if !apply {
        base["dry_run"] = json!(true);
        return Ok(base);
    }
    if !cited.is_empty() {
        return Err(AppError::structured(
            "check_failed",
            format!("{} run(s) this would remove are cited by claims; nothing was removed: {}", cited.len(), cited.iter().map(|c| c["run"].to_string()).collect::<Vec<_>>().join(", ")),
            Some("supersede or retire those claims (or build the ref again and re-claim), then run gc --refs again".into()),
            Some(json!({ "cited": cited })),
        ));
    }
    let builds: Vec<String> = store.other_holders().into_iter().filter(|h| h.starts_with("build")).collect();
    if !builds.is_empty() {
        return Err(AppError::structured("store_busy", format!("a build is running ({}); gc --refs removes nothing while one is", builds.join(", ")), Some("retry when it has finished".into()), None));
    }
    let bytes = store.file_bytes();
    let entry = store.with_write_lock(|s| -> Result<Value> {
        delete_runs(s, &ids)?;
        governance::append(
            s,
            root,
            &governance::Action {
                verb: "gc-refs".into(),
                by: by.into(),
                inputs: json!({ "keep": keep, "older_than_days": older_than_days, "runs": base["candidates"] }),
                bytes_before: Some(bytes),
                ..Default::default()
            },
        )
    })?;
    let mut removed_trees = vec![];
    for ((repo, commit), used) in &trees {
        if *used {
            continue;
        }
        let dir = gitref::cache_dir(root, repo, commit);
        let marker = dir.with_file_name(format!("{commit}.complete"));
        if dir.is_dir() && std::fs::remove_dir_all(&dir).is_ok() {
            removed_trees.push(dir.display().to_string());
        }
        let _ = std::fs::remove_file(marker);
    }
    base["dry_run"] = json!(false);
    base["removed_runs"] = json!(ids);
    base["removed_trees"] = json!(removed_trees);
    base["action_seq"] = entry["seq"].clone();
    Ok(base)
}
