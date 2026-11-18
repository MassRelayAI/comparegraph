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
    /// a branch verdict (scope `ref`): the ref and commit by value, an optional bundle path (a string, never read) and the delta numbers
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<serde_json::Value>,
}

/// The branch a decision is about (`decide --ref <ref> --commit <sha>`): a verdict on a branch, kept by value so it outlives
/// `gc --refs` of the ref's data. The bundle is only a string the caller recorded; nothing reads it.
#[derive(Debug, Clone, Default)]
pub struct RefVerdict {
    pub git_ref: String,
    pub commit: String,
    pub bundle: Option<String>,
    /// store the numbers of the last `delta` computed for this commit
    pub summary_from_delta: bool,
}

fn invalid<T>(m: impl Into<String>) -> Result<T> {
    Err(AppError::InvalidArguments(m.into()))
}

#[allow(clippy::too_many_arguments)]
pub fn decide(
    root: &Path,
    repo: &str,
    disposition: &str,
    why: &str,
    by: &str,
    feature: Option<&str>,
    family: Option<&str>,
    rv: Option<&RefVerdict>,
) -> Result<DecisionOut> {
    check_input(disposition, why, by)?;
    let store = Store::open(root)?;
    decide_on(&store, repo, disposition, why, by, feature, family, rv)
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
#[allow(clippy::too_many_arguments)]
pub fn decide_on(
    store: &Store,
    repo: &str,
    disposition: &str,
    why: &str,
    by: &str,
    feature: Option<&str>,
    family: Option<&str>,
    rv: Option<&RefVerdict>,
) -> Result<DecisionOut> {
    check_input(disposition, why, by)?;
    if let Some(rv) = rv {
        if feature.is_some() || family.is_some() {
            return invalid("a branch verdict (--ref) is about the branch, not a feature");
        }
        let c = &rv.commit;
        if rv.git_ref.trim().is_empty() || rv.git_ref.starts_with('-') {
            return invalid("--ref must name the branch");
        }
        if c.len() < 7 || c.len() > 40 || !c.bytes().all(|b| b.is_ascii_hexdigit()) {
            return invalid("--commit <sha> (7 to 40 hex digits) is required with --ref: the verdict is about that commit");
        }
    }
    let r = store
        .repo_by_name(repo)?
        .ok_or_else(|| AppError::NotFound(format!("no repo named {repo}")))?;
    super::archive::refuse_if_archived(store, repo)?;
    let rid = r.id.unwrap().0;
    let fid = match feature {
        Some(f) => Some(lookup_feature(store, f, family)?.0),
        None => None,
    };
    let scope = if rv.is_some() { "ref" } else if fid.is_some() { "feature" } else { "repo" };
    let (rref, rcommit, rbundle): (Option<&str>, Option<&str>, Option<&str>) = match rv {
        Some(v) => (Some(v.git_ref.as_str()), Some(v.commit.as_str()), v.bundle.as_deref()),
        None => (None, None, None),
    };
    let summary: Option<serde_json::Value> = match rv {
        Some(v) if v.summary_from_delta => Some(super::delta::last_delta(store, repo, &v.commit).ok_or_else(|| {
            AppError::NotFound(format!("no delta has been computed for {} ({}): run `delta {repo} --base <ref> --head {}` first", v.git_ref, v.commit, v.git_ref))
        })?),
        _ => None,
    };
    let summary_json: Option<String> = summary.as_ref().map(|s| s.to_string());
    let now = now_iso();
    let tx = store.conn();
    tx.execute_batch("SAVEPOINT cg_decide")?;
    let written: Result<(Option<i64>, i64)> = (|| {
        let supersedes: Option<i64> = tx.query_row(
            "SELECT MAX(id) FROM decision WHERE repo_id = ?1 AND scope = ?2 AND feature_id IS ?3 AND ref IS ?4",
            params![rid, scope, fid, rref],
            |x| x.get(0),
        )?;
        tx.execute(
            "INSERT INTO decision(repo_id, scope, feature_id, disposition, why, decided_by, decided_at, supersedes_id, ref, ref_commit, bundle, summary_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![rid, scope, fid, disposition, why, by, now, supersedes, rref, rcommit, rbundle, summary_json],
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
        git_ref: rref.map(String::from),
        ref_commit: rcommit.map(String::from),
        bundle: rbundle.map(String::from),
        summary,
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
    #[serde(default, rename = "ref")]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub bundle: Option<String>,
    #[serde(default)]
    pub summary_from_delta: bool,
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
                l.git_ref.as_ref().map(|r| RefVerdict { git_ref: r.clone(), commit: l.commit.clone().unwrap_or_default(), bundle: l.bundle.clone(), summary_from_delta: l.summary_from_delta }).as_ref(),
            )
        },
        |_| false,
        on_row,
    )
}

/// Latest per (repo, scope, feature) by default; every row when `history`.
pub fn decisions(root: &Path, repo: Option<&str>, history: bool) -> Result<Vec<DecisionOut>> {
    decisions_with(root, repo, history, None, false)
}

/// `decisions` with the branch verdicts (scope `ref`) included and, with `git_ref`, only those of one branch. The repo and feature
/// adoptions that matrix and export read never include them.
pub fn decisions_with(root: &Path, repo: Option<&str>, history: bool, git_ref: Option<&str>, with_refs: bool) -> Result<Vec<DecisionOut>> {
    let store = Store::open_existing(root)?;
    let mut st = store.conn().prepare(
        "SELECT d.id, r.name, d.scope, f.slug, d.disposition, d.why, d.decided_by, d.decided_at, d.supersedes_id, d.ref, d.ref_commit, d.bundle, d.summary_json \
         FROM decision d JOIN repo r ON r.id = d.repo_id LEFT JOIN feature f ON f.id = d.feature_id \
         WHERE (?1 IS NULL OR r.name = ?1) AND (?3 IS NULL OR d.ref = ?3) AND (?4 = 1 OR d.scope != 'ref') \
         AND (?2 = 1 OR d.id = (SELECT MAX(id) FROM decision x WHERE x.repo_id = d.repo_id AND x.scope = d.scope AND x.feature_id IS d.feature_id AND x.ref IS d.ref)) \
         ORDER BY r.name, d.scope, f.slug, d.id",
    )?;
    let out = st
        .query_map(params![repo, history as i64, git_ref, with_refs as i64], |r| {
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
                git_ref: r.get(9)?,
                ref_commit: r.get(10)?,
                bundle: r.get(11)?,
                summary: r.get::<_, Option<String>>(12)?.and_then(|s| serde_json::from_str(&s).ok()),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(out)
}
