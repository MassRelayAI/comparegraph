use super::Result;
use crate::store::{Store, SCHEMA_VERSION};
use rusqlite::params;
use std::path::Path;

#[derive(Debug, Default, serde::Serialize)]
pub struct CheckReport {
    pub repos: usize,
    pub runs_done: usize,
    pub stale_locations: usize,
    pub drifted_runs: usize,
    /// Latest claims whose confidence shows as unknown because a cited location is stale (informational).
    pub downgraded_claims: usize,
    pub schema_ok: bool,
    pub problems: Vec<String>,
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
        let mut st = store.conn().prepare("SELECT l.id, l.path, l.line_start, l.line_end, l.content_hash FROM location l JOIN claim c ON c.id = l.claim_id WHERE c.repo_id = ?1")?;
        let rows: Vec<(i64, String, u32, u32, String)> = st
            .query_map(params![r.id.unwrap().0], |x| {
                Ok((x.get(0)?, x.get(1)?, x.get(2)?, x.get(3)?, x.get(4)?))
            })?
            .collect::<std::result::Result<_, _>>()?;
        for (id, path, ls, le, h) in rows {
            let full = Path::new(&r.path).join(&path);
            let stale = match std::fs::read_to_string(&full) {
                Ok(src) => {
                    super::lines::span_hash(&src, ls as usize, le as usize).as_deref()
                        != Some(h.as_str())
                }
                Err(_) => true,
            };
            if stale {
                rep.stale_locations += 1;
                store
                    .conn()
                    .execute("UPDATE location SET stale = 1 WHERE id = ?1", params![id])?; // the one permitted flag flip (spec §7.4)
                rep.problems.push(format!(
                    "{}: location {} {}:{}-{} is stale",
                    r.name, id, path, ls, le
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
