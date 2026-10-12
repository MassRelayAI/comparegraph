//! Show: one symbol with its neighbours, licence lane and (on request) its current lines. Lines are read from the clone, never stored.
use super::{AppError, Result};
use crate::domain::Node;
use crate::store::Store;
use rusqlite::params;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct Shown {
    pub repo: String,
    pub licence: Option<String>,
    pub lane: String,
    pub node: Node,
    pub community: Option<String>,
    pub callers: Vec<String>,
    pub callees: Vec<String>,
    pub stale: bool,
    pub confidence: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<String>>,
}

const MAX_NEIGHBOURS: i64 = 50;

pub fn show(root: &Path, repo: &str, qn: &str, full: bool) -> Result<Shown> {
    let store = Store::open_existing(root)?;
    let r = store
        .repo_by_name(repo)?
        .ok_or_else(|| AppError::NotFound(format!("no repo named {repo}")))?;
    let run = store
        .latest_done_run(r.id.unwrap())?
        .ok_or_else(|| AppError::NotFound(format!("{repo} has no finished run")))?;
    let run_id = run.id.unwrap();
    let node = store
        .nodes_of_run(run_id)?
        .into_iter()
        .find(|n| n.qualified_name == qn)
        .ok_or_else(|| AppError::NotFound(format!("no symbol {qn} in {repo}")))?;
    let nid = node.id.unwrap().0;
    let c = store.conn();
    let community: Option<String> = c
        .query_row(
            "SELECT cm.label FROM community_member x JOIN community cm ON cm.id = x.community_id \
             WHERE x.node_id = ?1 AND cm.run_id = ?2 LIMIT 1",
            params![nid, run_id.0],
            |x| x.get(0),
        )
        .ok();
    let neighbours = |sql: &str| -> Result<Vec<String>> {
        let mut st = c.prepare(sql)?;
        let v = st
            .query_map(params![nid, run_id.0, MAX_NEIGHBOURS], |x| x.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(v)
    };
    let callers = neighbours(
        "SELECT DISTINCT n.qualified_name FROM edge e JOIN node n ON n.id = e.source_node_id \
         WHERE e.kind = 'calls' AND e.target_node_id = ?1 AND e.run_id = ?2 ORDER BY 1 LIMIT ?3",
    )?;
    let callees = neighbours(
        "SELECT DISTINCT n.qualified_name FROM edge e JOIN node n ON n.id = e.target_node_id \
         WHERE e.kind = 'calls' AND e.source_node_id = ?1 AND e.run_id = ?2 ORDER BY 1 LIMIT ?3",
    )?;
    // same rule as check.rs: out of range, missing file, or a different hash of the node's lines is stale
    let (ls, le) = (node.line_start as usize, node.line_end as usize);
    let src = std::fs::read_to_string(Path::new(&r.path).join(&node.path)).ok();
    let all: Vec<&str> = src
        .as_deref()
        .map(|s| s.lines().collect())
        .unwrap_or_default();
    let in_range = src.is_some() && ls >= 1 && ls <= le && le <= all.len();
    let stale = !in_range
        || src
            .as_deref()
            .and_then(|t| super::lines::span_hash(t, ls, le))
            .as_deref()
            != Some(node.content_hash.as_str());
    // current lines (clamped to the file) when readable, even if stale
    let lines = match (&src, full) {
        (Some(_), true) => {
            let from = ls.saturating_sub(1).min(all.len());
            let to = le.max(from).min(all.len());
            Some(all[from..to].iter().map(|s| s.to_string()).collect())
        }
        _ => None,
    };
    Ok(Shown {
        repo: r.name,
        licence: r.licence,
        lane: r.lane,
        node,
        community,
        callers,
        callees,
        stale,
        confidence: if stale { "unknown" } else { "observed" },
        lines,
    })
}
