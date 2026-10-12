//! `matrix <family>`: active features x repos. A cell is the latest claim's verdict; a claim with a stale location
//! displays `unknown*` and counts as unknown. Prevalence counts comparators only.
use super::claim::claims;
use super::decide::decisions;
use super::{AppError, Result};
use crate::store::Store;
use rusqlite::params;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct Column {
    pub repo: String,
    pub role: String,
    pub relatedness: Option<String>,
    pub decision: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct Row {
    pub group: String,
    pub id: String,
    pub tier: String,
    pub weight: i64,
    pub cells: Vec<String>,
    pub prevalence: Option<f64>,
    pub gap: Option<bool>,
    pub differentiator: Option<bool>,
}
#[derive(Debug, Serialize)]
pub struct Matrix {
    pub family: String,
    pub comparators: usize,
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
}

fn present(c: &str) -> bool {
    c == "has" || c == "partial"
}

pub fn matrix(root: &Path, family: &str) -> Result<Matrix> {
    let store = Store::open_existing(root)?;
    let c = store.conn();
    let fam_id: i64 = c
        .query_row(
            "SELECT id FROM family WHERE name = ?1",
            params![family],
            |r| r.get(0),
        )
        .map_err(|_| AppError::NotFound(format!("no family {family}")))?;
    let mut repos: Vec<(i64, String, String)> = c
        .prepare("SELECT id, name, role FROM repo")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    repos.sort_by(|a, b| ((a.2 != "root"), &a.1).cmp(&((b.2 != "root"), &b.1)));
    let has_root = repos.iter().any(|r| r.2 == "root");
    let comparators = repos.iter().filter(|r| r.2 != "root").count();

    let mut cl: HashMap<(String, String), String> = HashMap::new();
    for x in claims(root, None, Some(family), None, false)? {
        let stale = x.locations.iter().any(|l| l.stale);
        cl.insert(
            (x.repo, x.feature),
            if stale { "unknown*".into() } else { x.verdict },
        );
    }
    let mut dec: HashMap<String, String> = HashMap::new();
    for d in decisions(root, None, false)? {
        if d.scope == "repo" {
            dec.insert(d.repo, d.disposition);
        }
    }
    let mut columns = vec![];
    for (_, name, role) in &repos {
        let band: Option<String> = super::map::latest_relatedness(root, Some(name), Some(family))?
            .into_iter()
            .next()
            .map(|r| r.band);
        columns.push(Column {
            repo: name.clone(),
            role: role.clone(),
            relatedness: band,
            decision: dec.get(name).cloned(),
        });
    }
    let mut st = c.prepare("SELECT slug, tier, weight, grp FROM feature WHERE family_id = ?1 AND tier != 'retired' ORDER BY grp, slug")?;
    let feats: Vec<(String, String, i64, String)> = st
        .query_map(params![fam_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let mut rows = vec![];
    for (slug, tier, weight, group) in feats {
        let cells: Vec<String> = repos
            .iter()
            .map(|r| {
                cl.get(&(r.1.clone(), slug.clone()))
                    .cloned()
                    .unwrap_or_else(|| "-".into())
            })
            .collect();
        let comp: Vec<&String> = repos
            .iter()
            .zip(&cells)
            .filter(|(r, _)| r.2 != "root")
            .map(|(_, c)| c)
            .collect();
        let n_present = comp.iter().filter(|c| present(c)).count();
        let prevalence = (comparators > 0).then(|| n_present as f64 / comparators as f64);
        let (mut gap, mut diff) = (None, None);
        if has_root {
            let root_cell = repos
                .iter()
                .zip(&cells)
                .find(|(r, _)| r.2 == "root")
                .map(|(_, c)| c.as_str())
                .unwrap();
            gap = Some(n_present > 0 && !present(root_cell));
            diff = Some(present(root_cell) && n_present == 0);
        }
        rows.push(Row {
            group,
            id: slug,
            tier,
            weight,
            cells,
            prevalence,
            gap,
            differentiator: diff,
        });
    }
    Ok(Matrix {
        family: family.into(),
        comparators,
        columns,
        rows,
    })
}
