use super::Result;
use crate::store::{Store, SCHEMA_VERSION};
use rusqlite::params;
use std::path::Path;

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
    pub problems: Vec<String>,
    /// informational lines, never problems
    pub notes: Vec<String>,
}
impl CheckReport {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

pub fn check(root: &Path) -> Result<CheckReport> {
    let store = Store::open_existing(root)?;
    let mut rep = CheckReport {
        schema_ok: store.schema_version()? == SCHEMA_VERSION,
        ..Default::default()
    };
    if !rep.schema_ok {
        rep.problems
            .push(format!("schema version is not {SCHEMA_VERSION}"));
    }
    let repos = store.repos()?;
    rep.repos = repos.len();
    for r in &repos {
        let mut st = store.conn().prepare(
            "SELECT c.run_id, COUNT(*) FROM claim c LEFT JOIN run u ON u.id = c.run_id \
             WHERE c.repo_id = ?1 AND (u.id IS NULL OR u.status != 'done') GROUP BY c.run_id ORDER BY c.run_id",
        )?;
        let bad: Vec<(i64, usize)> = st
            .query_map(params![r.id.unwrap().0], |x| Ok((x.get(0)?, x.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        for (run_id, n) in bad {
            rep.unbuilt_claim_runs += n;
            rep.problems.push(format!(
                "{}: {} claims cite run {} which was never built",
                r.name, n, run_id
            ));
        }
        let Some(run) = store.latest_done_run(r.id.unwrap())? else {
            continue;
        };
        rep.runs_done += store.conn().query_row(
            "SELECT COUNT(*) FROM run WHERE repo_id=?1 AND status='done'",
            params![r.id.unwrap().0],
            |x| x.get::<_, usize>(0),
        )?;
        // determinism: the recorded hash must equal a recomputation over the stored rows
        let recomputed = store.store_hash(run.id.unwrap())?;
        if run.store_hash.as_deref() != Some(recomputed.as_str()) {
            rep.drifted_runs += 1;
            rep.problems.push(format!(
                "{}: run {} store_hash drifted",
                r.name,
                run.id.unwrap()
            ));
        }
        // search index: every node of the run has an FTS row
        let nodes: i64 = store.conn().query_row(
            "SELECT COUNT(*) FROM node WHERE run_id=?1",
            params![run.id.unwrap().0],
            |x| x.get(0),
        )?;
        let fts: i64 = store.conn().query_row(
            "SELECT COUNT(*) FROM node_fts WHERE rowid IN (SELECT id FROM node WHERE run_id=?1)",
            params![run.id.unwrap().0],
            |x| x.get(0),
        )?;
        if nodes != fts {
            rep.problems.push(format!(
                "{}: search index incomplete: rebuild {}",
                r.name, r.name
            ));
        }
        // locations: path exists, lines within file, content hash matches
        let mut st = store.conn().prepare("SELECT l.id, l.path, l.line_start, l.line_end, l.content_hash, c.id = (SELECT MAX(id) FROM claim x WHERE x.repo_id = c.repo_id AND x.feature_id = c.feature_id) FROM location l JOIN claim c ON c.id = l.claim_id WHERE c.repo_id = ?1")?;
        let rows: Vec<(i64, String, u32, u32, String, bool)> = st
            .query_map(params![r.id.unwrap().0], |x| {
                Ok((x.get(0)?, x.get(1)?, x.get(2)?, x.get(3)?, x.get(4)?, x.get(5)?))
            })?
            .collect::<std::result::Result<_, _>>()?;
        for (id, path, ls, le, h, live) in rows {
            let full = Path::new(&r.path).join(&path);
            let stale = match std::fs::read_to_string(&full) {
                Ok(src) => {
                    super::lines::span_hash(&src, ls as usize, le as usize).as_deref()
                        != Some(h.as_str())
                }
                Err(_) => true,
            };
            if stale {
                store
                    .conn()
                    .execute("UPDATE location SET stale = 1 WHERE id = ?1", params![id])?; // the one permitted flag flip (spec §7.4)
                if live {
                    rep.stale_locations += 1;
                    rep.problems.push(format!(
                        "{}: location {} {}:{}-{} is stale",
                        r.name, id, path, ls, le
                    ));
                } else {
                    rep.stale_locations_superseded += 1;
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
                rep.notes.push(format!(
                    "family {}: the pack snapshot was edited after import",
                    f.name
                ));
            }
        }
    }
    rep.downgraded_claims = store.conn().query_row(
        "SELECT COUNT(*) FROM claim c WHERE c.id = (SELECT MAX(id) FROM claim x WHERE x.repo_id = c.repo_id AND x.feature_id = c.feature_id) \
         AND EXISTS (SELECT 1 FROM location l WHERE l.claim_id = c.id AND l.stale = 1)",
        [],
        |x| x.get(0),
    )?;
    Ok(rep)
}
