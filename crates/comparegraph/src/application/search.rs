//! Search: FTS5 over the symbols of each repo's latest done run. Tokens are quoted, so query syntax never leaks.
use super::{AppError, Result};
use crate::domain::NodeKind;
use crate::store::Store;
use rusqlite::params;
use serde::Serialize;
use std::path::Path;

pub struct SearchQuery {
    pub text: String,
    pub repo: Option<String>,
    pub kind: Option<NodeKind>,
    pub limit: usize,
    pub offset: usize,
}
#[derive(Debug, Serialize)]
pub struct Hit {
    pub repo: String,
    pub qualified_name: String,
    pub kind: String,
    pub path: String,
    pub line_start: i64,
    pub line_end: i64,
    pub community: Option<String>,
    pub score: f64,
    pub label: &'static str,
}
#[derive(Debug, Serialize)]
pub struct SearchPage {
    pub items: Vec<Hit>,
    pub total: usize,
    pub next: Option<String>,
}

fn quoted_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\"*"))
        .collect()
}

/// Each alphanumeric/underscore run becomes `"tok"*`; joined by spaces (AND).
pub fn fts_query(text: &str) -> Option<String> {
    let toks = quoted_tokens(text);
    if toks.is_empty() {
        None
    } else {
        Some(toks.join(" "))
    }
}

/// The same quoted tokens joined with `OR`: a symbol matching any term qualifies.
pub fn fts_query_or(text: &str) -> Option<String> {
    let toks = quoted_tokens(text);
    if toks.is_empty() {
        None
    } else {
        Some(toks.join(" OR "))
    }
}

/// Top `limit` symbols of one repo's latest done run matching ANY of `terms`, best bm25 first.
pub fn search_any_in_repo(
    store: &Store,
    repo: &str,
    terms: &[String],
    limit: usize,
) -> Result<Vec<Hit>> {
    let Some(m) = fts_query_or(&terms.join(" ")) else {
        return Ok(vec![]);
    };
    let sql = format!(
        "SELECT r.name, n.qualified_name, n.kind, n.path, n.line_start, n.line_end, \
         (SELECT cm.label FROM community_member x JOIN community cm ON cm.id = x.community_id \
           WHERE x.node_id = n.id AND cm.run_id = n.run_id LIMIT 1), -bm25(node_fts) AS score \
         {WHERE} ORDER BY score DESC, n.qualified_name ASC, n.id ASC LIMIT ?4"
    );
    let mut st = store.conn().prepare(&sql)?;
    let none: Option<&str> = None;
    let hits = st
        .query_map(params![m, repo, none, limit as i64], |r| {
            Ok(Hit {
                repo: r.get(0)?,
                qualified_name: r.get(1)?,
                kind: r.get(2)?,
                path: r.get(3)?,
                line_start: r.get(4)?,
                line_end: r.get(5)?,
                community: r.get(6)?,
                score: r.get(7)?,
                label: "candidate",
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(hits)
}

const WHERE: &str = "FROM node_fts JOIN node n ON n.id = node_fts.rowid JOIN repo r ON r.id = n.repo_id \
 WHERE node_fts MATCH ?1 AND n.run_id = (SELECT MAX(id) FROM run WHERE repo_id = n.repo_id AND status = 'done') \
 AND (?2 IS NULL OR r.name = ?2) AND (?3 IS NULL OR n.kind = ?3)";

pub fn search(root: &Path, q: &SearchQuery) -> Result<SearchPage> {
    let m = fts_query(&q.text).ok_or_else(|| AppError::Empty("empty query".into()))?;
    let store = Store::open_existing(root)?;
    let c = store.conn();
    let kind = q.kind.map(|k| k.as_str());
    let total: i64 = c.query_row(
        &format!("SELECT COUNT(*) {WHERE}"),
        params![m, q.repo, kind],
        |r| r.get(0),
    )?;
    let sql = format!(
        "SELECT r.name, n.qualified_name, n.kind, n.path, n.line_start, n.line_end, \
         (SELECT cm.label FROM community_member x JOIN community cm ON cm.id = x.community_id \
           WHERE x.node_id = n.id AND cm.run_id = n.run_id LIMIT 1), -bm25(node_fts) AS score \
         {WHERE} ORDER BY score DESC, n.qualified_name ASC, n.id ASC LIMIT ?4 OFFSET ?5"
    );
    let mut st = c.prepare(&sql)?;
    let items = st
        .query_map(
            params![m, q.repo, kind, q.limit as i64, q.offset as i64],
            |r| {
                Ok(Hit {
                    repo: r.get(0)?,
                    qualified_name: r.get(1)?,
                    kind: r.get(2)?,
                    path: r.get(3)?,
                    line_start: r.get(4)?,
                    line_end: r.get(5)?,
                    community: r.get(6)?,
                    score: r.get(7)?,
                    label: "candidate",
                })
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let total = total as usize;
    let end = q.offset + items.len();
    Ok(SearchPage {
        next: (end < total).then(|| end.to_string()),
        items,
        total,
    })
}
