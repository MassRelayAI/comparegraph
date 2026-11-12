//! `check`: schema, the search index, unbuilt claim runs and citation locations, plus the action-log chain.
//!
//! Read-only by default (lane CG-STORE A3): the store is opened SQLITE_OPEN_READ_ONLY, so neither the MCP tool nor the
//! CLI can write a flag, migrate the store or checkpoint it. A repo whose clone is missing, or whose HEAD is not the
//! head its latest run read, is `clone_unavailable`: its locations are not read and nothing is written for it (an
//! environment failure is never recorded as content drift). `--mark` (CLI only) writes `stale = 1` on the locations
//! found stale; `--mark --recompute` also clears `stale` where the cited lines hash as recorded again (the reset path
//! the flag never had). `--deep` adds the per-run `store_hash` recomputation (all nodes and edges re-read).
use super::{AppError, Result};
use crate::store::{Store, SCHEMA_VERSION};
use rusqlite::params;
use std::path::Path;

#[derive(Debug, Default, Clone)]
pub struct CheckOptions {
    /// write `stale = 1` for the locations found stale (CLI only)
    pub mark: bool,
    /// with `mark`: also clear `stale` on locations whose cited lines hash as recorded
    pub recompute: bool,
    /// recompute every latest run's store_hash
    pub deep: bool,
    /// who asked (`--by`), recorded in the action log when `mark` writes
    pub by: Option<String>,
}

#[derive(Debug, Default, serde::Serialize)]
pub struct CheckReport {
    pub repos: usize,
    pub runs_done: usize,
    /// Stale locations of live claims (the latest claim per repo and feature).
    pub stale_locations: usize,
    /// Stale locations that belong to superseded claims: append-only history, informational.
    pub stale_locations_superseded: usize,
    pub drifted_runs: usize,
    /// Claims whose cited run has no row or is not a finished (`done`) build.
    pub unbuilt_claim_runs: usize,
    /// Latest claims whose confidence shows as unknown because a cited location is stale (informational).
    pub downgraded_claims: usize,
    /// Families whose pack snapshot file is absent from the workspace.
    pub family_snapshots_missing: usize,
    pub schema_ok: bool,
    /// repos whose clone could not be read as the run read it; their locations were not checked and nothing was written
    pub clone_unavailable: Vec<CloneUnavailable>,
    /// archived repos (`archive`): never read by check
    pub archived_repos: usize,
    /// `--deep` was given: the store_hash of every latest run was recomputed
    pub deep: bool,
    /// what `--mark` wrote (null without it): flags set, flags cleared
    pub marked: Option<Marked>,
    /// the action log: entries and whether the chain verifies
    pub actions: usize,
    pub chain_ok: bool,
    pub problems: Vec<String>,
    /// informational lines, never problems
    pub notes: Vec<String>,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct CloneUnavailable {
    pub repo: String,
    pub path: String,
    /// `missing` (no directory) or `head_moved` (HEAD is not the head the run read)
    pub reason: String,
    pub expected_head: Option<String>,
    pub found_head: Option<String>,
    pub locations_not_checked: usize,
}
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Marked {
    pub set_stale: usize,
    pub cleared: usize,
}
impl CheckReport {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
    /// True when every problem is a clone that could not be read (the envelope then says `clone_unavailable`).
    pub fn only_unavailable(&self) -> bool {
        !self.problems.is_empty() && self.problems.iter().all(|p| p.contains("clone unavailable"))
    }
}

/// The read-only check (MCP `comparegraph_check`, `comparegraph check`).
pub fn check(root: &Path) -> Result<CheckReport> {
    check_with(root, &CheckOptions::default())
}

/// Is the clone there as the run read it? None when it is; the reason otherwise.
fn availability(path: &str, expected: Option<&str>) -> Option<(String, Option<String>)> {
    let p = Path::new(path);
    if !p.is_dir() {
        return Some(("missing".into(), None));
    }
    let expected = expected?;
    let found = super::git(p, &["rev-parse", "--short=7", "HEAD"]);
    match &found {
        Some(f) if f.starts_with(expected) || expected.starts_with(f.as_str()) => None,
        _ => Some(("head_moved".into(), found)),
    }
}

pub fn check_with(root: &Path, o: &CheckOptions) -> Result<CheckReport> {
    if o.recompute && !o.mark {
        return Err(AppError::InvalidArguments("--recompute writes flags: give it with --mark".into()));
    }
    if o.mark && o.by.as_deref().is_none_or(|b| b.trim().is_empty()) {
        return Err(AppError::InvalidArguments("--mark writes: give --by <who> (recorded in the action log)".into()));
    }
    let store = if o.mark { Store::open_guarded(root)? } else { Store::open_read_only(root)? };
    let mut rep = CheckReport {
        schema_ok: store.schema_version()? == SCHEMA_VERSION,
        deep: o.deep,
        ..Default::default()
    };
    if !rep.schema_ok {
        rep.problems.push(format!("schema version is not {SCHEMA_VERSION}"));
    }
    if store.predates_this_binary()? {
        rep.notes.push("the store predates columns of this binary and was read without migrating it".into());
    }
    let archived = super::archive::archived_names(&store)?;
    let repos = store.repos()?;
    rep.repos = repos.len();
    let mut to_set: Vec<i64> = Vec::new();
    let mut to_clear: Vec<i64> = Vec::new();
    for r in &repos {
        if archived.contains(&r.name) {
            rep.archived_repos += 1;
            continue;
        }
        let mut st = store.conn().prepare(
            "SELECT c.run_id, COUNT(*) FROM claim c LEFT JOIN run u ON u.id = c.run_id \
             WHERE c.repo_id = ?1 AND (u.id IS NULL OR u.status != 'done') GROUP BY c.run_id ORDER BY c.run_id",
        )?;
        let bad: Vec<(i64, usize)> = st
            .query_map(params![r.id.unwrap().0], |x| Ok((x.get(0)?, x.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let done = store.latest_done_run(r.id.unwrap())?;
        for (run_id, n) in bad {
            // a run back from a claims-only backup was built once; once the repo is rebuilt its claims are checkable
            let restored = store.run(crate::domain::RunId(run_id))?.is_some_and(|u| u.status == crate::domain::RunStatus::Restored);
            if restored && done.is_some() {
                rep.notes.push(format!("{}: {} claims cite run {} restored without its graph; the repo has been rebuilt", r.name, n, run_id));
                continue;
            }
            rep.unbuilt_claim_runs += n;
            let why = if restored { "restored without its graph (build the repo)" } else { "which was never built" };
            rep.problems.push(format!("{}: {} claims cite run {} {why}", r.name, n, run_id));
        }
        let Some(run) = done else {
            continue;
        };
        rep.runs_done += store.conn().query_row(
            "SELECT COUNT(*) FROM run WHERE repo_id=?1 AND status='done'",
            params![r.id.unwrap().0],
            |x| x.get::<_, usize>(0),
        )?;
        if o.deep {
            // determinism: the recorded hash must equal a recomputation over the stored rows
            let recomputed = store.store_hash(run.id.unwrap())?;
            if run.store_hash.as_deref() != Some(recomputed.as_str()) {
                rep.drifted_runs += 1;
                rep.problems.push(format!("{}: run {} store_hash drifted", r.name, run.id.unwrap()));
            }
        }
        // search index: every node of the run has an FTS row
        let nodes: i64 = store.conn().query_row("SELECT COUNT(*) FROM node WHERE run_id=?1", params![run.id.unwrap().0], |x| x.get(0))?;
        let fts: i64 = store.conn().query_row(
            "SELECT COUNT(*) FROM node_fts WHERE rowid IN (SELECT id FROM node WHERE run_id=?1)",
            params![run.id.unwrap().0],
            |x| x.get(0),
        )?;
        if nodes != fts {
            rep.problems.push(format!("{}: search index incomplete: rebuild {}", r.name, r.name));
        }
        // locations: only when the clone is there as the run read it
        let rows = store.location_check_rows(r.id.unwrap())?;
        if rows.is_empty() {
            continue;
        }
        let expected = store.run_head_opt(run.id.unwrap())?.or_else(|| r.head.clone());
        if let Some((reason, found)) = availability(&r.path, expected.as_deref()) {
            rep.problems.push(format!(
                "{}: clone unavailable ({reason}) at {}; its {} location(s) were not checked and nothing was written",
                r.name,
                r.path,
                rows.len()
            ));
            rep.clone_unavailable.push(CloneUnavailable {
                repo: r.name.clone(),
                path: r.path.clone(),
                reason,
                expected_head: expected,
                found_head: found,
                locations_not_checked: rows.len(),
            });
            continue;
        }
        for l in rows {
            let full = Path::new(&r.path).join(&l.path);
            // the clone is present at its head: an unreadable cited file is real drift (deleted or renamed)
            let stale = match std::fs::read_to_string(&full) {
                Ok(src) => super::lines::span_hash(&src, l.line_start as usize, l.line_end as usize).as_deref() != Some(l.content_hash.as_str()),
                Err(_) => true,
            };
            if stale {
                if !l.stale {
                    to_set.push(l.id);
                }
                if l.live {
                    rep.stale_locations += 1;
                    rep.problems.push(format!("{}: location {} {}:{}-{} is stale", r.name, l.id, l.path, l.line_start, l.line_end));
                } else {
                    rep.stale_locations_superseded += 1;
                }
            } else if l.stale {
                to_clear.push(l.id);
                if !o.recompute {
                    rep.notes.push(format!(
                        "{}: location {} is flagged stale but its lines hash as recorded now (`check --mark --recompute` clears it)",
                        r.name, l.id
                    ));
                }
            }
        }
    }
    if rep.stale_locations_superseded > 0 {
        rep.notes.push(format!(
            "{} stale location(s) belong to superseded claims (history, not problems)",
            rep.stale_locations_superseded
        ));
    }
    for f in store.families()? {
        if !super::family::snapshot_present(root, &f.definition_path) {
            rep.family_snapshots_missing += 1;
            rep.problems.push(format!(
                "family {}: the pack snapshot {} is missing; re-run family add <dir>",
                f.name, f.definition_path
            ));
        } else if !f.definition_hash.is_empty() {
            let edited = std::fs::read(root.join(&f.definition_path))
                .map(|b| crate::providers::reference::hex_sha256(&b) != f.definition_hash)
                .unwrap_or(false);
            if edited {
                rep.notes.push(format!("family {}: the pack snapshot was edited after import", f.name));
            }
        }
    }
    // the action log: a broken chain is a problem like any other
    let g = super::governance::verify_with(&store, root)?;
    rep.actions = g.actions;
    rep.chain_ok = g.ok();
    for p in &g.problems {
        rep.problems.push(format!("action log: {p}"));
    }
    if o.mark {
        let clear: Vec<i64> = if o.recompute { to_clear } else { vec![] };
        let marked = Marked { set_stale: to_set.len(), cleared: clear.len() };
        if marked.set_stale + marked.cleared > 0 {
            let bytes = store.file_bytes();
            store.with_write_lock(|s| -> Result<()> {
                for id in &to_set {
                    s.set_location_stale(*id, true)?;
                }
                for id in &clear {
                    s.set_location_stale(*id, false)?;
                }
                super::governance::append(s, root, &super::governance::Action {
                    verb: if o.recompute { "check-recompute".into() } else { "check-mark".into() },
                    by: o.by.clone().unwrap_or_default(),
                    inputs: serde_json::json!({ "set_stale": to_set, "cleared": clear }),
                    bytes_before: Some(bytes),
                    ..Default::default()
                })?;
                Ok(())
            })?;
            rep.actions += 1;
        }
        rep.marked = Some(marked);
    }
    rep.downgraded_claims = store.conn().query_row(
        "SELECT COUNT(*) FROM claim c WHERE c.id = (SELECT MAX(id) FROM claim x WHERE x.repo_id = c.repo_id AND x.feature_id = c.feature_id) \
         AND EXISTS (SELECT 1 FROM location l WHERE l.claim_id = c.id AND l.stale = 1)",
        [],
        |x| x.get(0),
    )?;
    Ok(rep)
}
