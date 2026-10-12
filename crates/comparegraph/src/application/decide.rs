//! `decide` and `decisions`: adopt/reject/park, recorded from a person's instruction. Append-only; the latest of the same
//! scope (repo, or repo+feature) wins and the new row points at it.
use super::claim::lookup_feature;
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
    let store = Store::open(root)?;
    let r = store
        .repo_by_name(repo)?
        .ok_or_else(|| AppError::NotFound(format!("no repo named {repo}")))?;
    let rid = r.id.unwrap().0;
    let fid = match feature {
        Some(f) => Some(lookup_feature(&store, f, family)?.0),
        None => None,
    };
    let scope = if fid.is_some() { "feature" } else { "repo" };
    let now = now_iso();
    let tx = store.conn().unchecked_transaction()?;
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
    tx.commit()?;
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
