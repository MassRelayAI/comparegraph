//! `decide` and `decisions`: adopt/reject/park, recorded from a person's instruction. Append-only; the latest of the same
//! scope (repo, or repo+feature) wins and the new row points at it.
use super::claim::{lookup_feature, run_batch, BatchRow, BatchSummary};
use super::{AppError, Result};
use crate::store::{now_iso, Store};
use rusqlite::params;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize, Clone)]
pub struct DecisionOut {
    pub id: i64,
    pub repo: String,
    pub scope: String,
    pub feature: Option<String>,
    pub disposition: String,
    pub why: String,
    pub decided_by: String,
    pub decided_at: String,
    pub supersedes_id: Option<i64>,
}

fn invalid<T>(m: impl Into<String>) -> Result<T> {
    Err(AppError::InvalidArguments(m.into()))
}

pub fn decide(
    root: &Path,
    repo: &str,
    disposition: &str,
    why: &str,
    by: &str,
    feature: Option<&str>,
    family: Option<&str>,
) -> Result<DecisionOut> {
    check_input(disposition, why, by)?;
    let store = Store::open(root)?;
    decide_on(&store, repo, disposition, why, by, feature, family)
}

fn check_input(disposition: &str, why: &str, by: &str) -> Result<()> {
    if !["adopt", "reject", "park"].contains(&disposition) {
        return invalid(format!(
            "unknown disposition {disposition} (adopt|reject|park)"
        ));
    }
    if why.trim().is_empty() {
        return invalid("--why is required and must not be empty");
    }
    if by.trim().is_empty() {
        return invalid("--by is required and must not be empty");
    }
    Ok(())
}

/// The body of `decide` on an already open store; inside a batch transaction the write is a savepoint.
pub fn decide_on(
    store: &Store,
    repo: &str,
    disposition: &str,
    why: &str,
    by: &str,
    feature: Option<&str>,
    family: Option<&str>,
) -> Result<DecisionOut> {
    check_input(disposition, why, by)?;
    let r = store
        .repo_by_name(repo)?
        .ok_or_else(|| AppError::NotFound(format!("no repo named {repo}")))?;
    let rid = r.id.unwrap().0;
    let fid = match feature {
        Some(f) => Some(lookup_feature(store, f, family)?.0),
        None => None,
    };
    let scope = if fid.is_some() { "feature" } else { "repo" };
    let now = now_iso();
    let tx = store.conn();
    tx.execute_batch("SAVEPOINT cg_decide")?;
    let written: Result<(Option<i64>, i64)> = (|| {
        let supersedes: Option<i64> = tx.query_row(
            "SELECT MAX(id) FROM decision WHERE repo_id = ?1 AND scope = ?2 AND feature_id IS ?3",
            params![rid, scope, fid],
            |x| x.get(0),
        )?;
        tx.execute(
            "INSERT INTO decision(repo_id, scope, feature_id, disposition, why, decided_by, decided_at, supersedes_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![rid, scope, fid, disposition, why, by, now, supersedes],
        )?;
        let id = tx.last_insert_rowid();
        Ok((supersedes, id))
    })();
    let (supersedes, id) = match written {
        Ok(x) => {
            tx.execute_batch("RELEASE cg_decide")?;
            x
        }
        Err(e) => {
            let _ = tx.execute_batch("ROLLBACK TO cg_decide; RELEASE cg_decide");
            return Err(e);
        }
    };
    Ok(DecisionOut {
        id,
        repo: repo.into(),
        scope: scope.into(),
        feature: feature.map(String::from),
        disposition: disposition.into(),
        why: why.into(),
        decided_by: by.into(),
        decided_at: now,
        supersedes_id: supersedes,
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecideLine {
    pub repo: String,
    pub disposition: String,
    pub why: String,
    pub by: String,
    #[serde(default)]
    pub feature: Option<String>,
    #[serde(default)]
    pub family: Option<String>,
}

/// `decide` for a JSONL stream: one store open, one transaction, one `BatchRow` per non-blank line, each handed to
/// `on_row` as it is decided.
pub fn decide_batch(
    root: &Path,
    lines: impl std::io::BufRead,
    on_row: impl FnMut(BatchRow<DecisionOut>),
) -> Result<BatchSummary> {
    run_batch(
        root,
        lines,
        |store, l: DecideLine| {
            decide_on(
                store,
                &l.repo,
                &l.disposition,
                &l.why,
                &l.by,
                l.feature.as_deref(),
                l.family.as_deref(),
            )
        },
        |_| false,
        on_row,
    )
}

/// Latest per (repo, scope, feature) by default; every row when `history`.
pub fn decisions(root: &Path, repo: Option<&str>, history: bool) -> Result<Vec<DecisionOut>> {
    let store = Store::open_existing(root)?;
    let mut st = store.conn().prepare(
        "SELECT d.id, r.name, d.scope, f.slug, d.disposition, d.why, d.decided_by, d.decided_at, d.supersedes_id \
         FROM decision d JOIN repo r ON r.id = d.repo_id LEFT JOIN feature f ON f.id = d.feature_id \
         WHERE (?1 IS NULL OR r.name = ?1) \
         AND (?2 = 1 OR d.id = (SELECT MAX(id) FROM decision x WHERE x.repo_id = d.repo_id AND x.scope = d.scope AND x.feature_id IS d.feature_id)) \
         ORDER BY r.name, d.scope, f.slug, d.id",
    )?;
    let out = st
        .query_map(params![repo, history as i64], |r| {
            Ok(DecisionOut {
                id: r.get(0)?,
                repo: r.get(1)?,
                scope: r.get(2)?,
                feature: r.get(3)?,
                disposition: r.get(4)?,
                why: r.get(5)?,
                decided_by: r.get(6)?,
                decided_at: r.get(7)?,
                supersedes_id: r.get(8)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(out)
}
