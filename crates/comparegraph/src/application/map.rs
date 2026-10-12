//! `map` and `relate`: per-feature candidates from the graph (FTS OR over the feature terms, plus communities whose
//! label or keywords carry a term), and a relatedness band per (clone, family). Candidates are never verdicts.
use super::communities::tokens;
use super::search::{search_any_in_repo, Hit};
use super::{AppError, Result};
use crate::store::{FeatureRow, Store};
use rusqlite::params;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct CommunityCandidate {
    pub label: String,
    pub matched: Vec<String>,
    pub candidate: &'static str,
}
#[derive(Debug, Serialize)]
pub struct FeatureMap {
    pub id: String,
    pub title: String,
    pub terms: Vec<String>,
    pub symbols: Vec<Hit>,
    pub communities: Vec<CommunityCandidate>,
}
#[derive(Debug, Serialize)]
pub struct Mapped {
    pub repo: String,
    pub family: String,
    pub features: Vec<FeatureMap>,
}
#[derive(Debug, Serialize, Clone)]
pub struct Related {
    pub repo: String,
    pub family: String,
    pub run_id: i64,
    pub score: f64,
    pub band: String,
    pub basis: Vec<String>,
}

/// Tokens of the group, the slug and the title; deduped and sorted. The definition is not used.
pub fn feature_terms(f: &FeatureRow) -> Vec<String> {
    let slug = f.slug.rsplit('.').next().unwrap_or(&f.slug);
    let set: BTreeSet<String> = tokens(&f.grp)
        .into_iter()
        .chain(tokens(slug))
        .chain(tokens(&f.title))
        .collect();
    set.into_iter().collect()
}

pub fn band_of(score: f64) -> &'static str {
    if score >= 0.5 {
        "related"
    } else if score >= 0.2 {
        "adjacent"
    } else {
        "unrelated"
    }
}

fn latest_done(store: &Store, repo: &str) -> Result<(i64, i64)> {
    let r = store
        .repo_by_name(repo)?
        .ok_or_else(|| AppError::NotFound(format!("no repo named {repo}")))?;
    let rid = r.id.unwrap();
    let run = store
        .latest_done_run(rid)?
        .ok_or_else(|| AppError::NotFound(format!("{repo} has no finished run; build it first")))?;
    Ok((rid.0, run.id.unwrap().0))
}

fn map_feature(
    store: &Store,
    repo: &str,
    run: i64,
    f: &FeatureRow,
    limit: usize,
) -> Result<FeatureMap> {
    let terms = feature_terms(f);
    let symbols = search_any_in_repo(store, repo, &terms, limit)?;
    let mut communities: Vec<CommunityCandidate> = store
        .communities_of_run(crate::domain::RunId(run))?
        .into_iter()
        .filter_map(|c| {
            let have: BTreeSet<String> = tokens(&c.label)
                .into_iter()
                .chain(c.keywords.iter().map(|k| k.to_lowercase()))
                .collect();
            let matched: Vec<String> = terms
                .iter()
                .filter(|t| have.contains(*t))
                .cloned()
                .collect();
            (!matched.is_empty()).then_some(CommunityCandidate {
                label: c.label,
                matched,
                candidate: "candidate",
            })
        })
        .collect();
    communities.sort_by(|a, b| {
        b.matched
            .len()
            .cmp(&a.matched.len())
            .then(a.label.cmp(&b.label))
    });
    communities.truncate(3);
    Ok(FeatureMap {
        id: f.key(),
        title: f.title.clone(),
        terms,
        symbols,
        communities,
    })
}

fn active(store: &Store, family: &str) -> Result<Vec<FeatureRow>> {
    Ok(store
        .features_of(family)?
        .into_iter()
        .filter(|f| f.tier != "retired")
        .collect())
}

pub fn map(
    root: &Path,
    repo: &str,
    family: &str,
    feature: Option<&str>,
    limit: usize,
) -> Result<Mapped> {
    let store = Store::open_existing(root)?;
    store
        .family_by_name(family)?
        .ok_or_else(|| AppError::NotFound(format!("no family named {family}")))?;
    let (_, run) = latest_done(&store, repo)?;
    let mut feats = active(&store, family)?;
    if let Some(id) = feature {
        feats.retain(|f| f.key() == id);
        if feats.is_empty() {
            return Err(AppError::NotFound(format!(
                "no active feature {id} in {family}"
            )));
        }
    }
    let features = feats
        .iter()
        .map(|f| map_feature(&store, repo, run, f, limit))
        .collect::<Result<Vec<_>>>()?;
    Ok(Mapped {
        repo: repo.into(),
        family: family.into(),
        features,
    })
}

/// sha256 of the family's active features as sorted `<id>\t<title>` lines: what a relatedness row was scored against.
pub(crate) fn pack_hash(store: &Store, family: &str) -> Result<String> {
    let mut lines: Vec<String> = active(store, family)?
        .iter()
        .map(|f| format!("{}\t{}", f.key(), f.title))
        .collect();
    lines.sort();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    Ok(crate::domain::content_hash(&refs))
}

fn relate_pair(store: &Store, repo: &str, family: &str) -> Result<Related> {
    let (rid, run) = latest_done(store, repo)?;
    let fam = store
        .family_by_name(family)?
        .ok_or_else(|| AppError::NotFound(format!("no family named {family}")))?;
    let feats = active(store, family)?;
    let ph = pack_hash(store, family)?;
    let mut basis = vec![];
    for f in &feats {
        let m = map_feature(store, repo, run, f, 1)?;
        if !m.symbols.is_empty() || !m.communities.is_empty() {
            basis.push(f.key());
        }
    }
    basis.sort();
    let score = if feats.is_empty() {
        0.0
    } else {
        basis.len() as f64 / feats.len() as f64
    };
    store.conn().execute(
        "INSERT OR IGNORE INTO relatedness(repo_id, family_id, run_id, score, band, basis_json, pack_hash) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![rid, fam.id, run, score, band_of(score), serde_json::to_string(&basis).unwrap(), ph],
    )?;
    let (score, band, bj): (f64, String, String) = store.conn().query_row(
        "SELECT score, band, basis_json FROM relatedness WHERE repo_id=?1 AND family_id=?2 AND run_id=?3 AND pack_hash=?4",
        params![rid, fam.id, run, ph],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    Ok(Related {
        repo: repo.into(),
        family: family.into(),
        run_id: run,
        score,
        band,
        basis: serde_json::from_str(&bj).unwrap_or_default(),
    })
}

/// Score every (repo, family) pair (or the given ones). Unbuilt repos are skipped in the all-repos case,
/// refused (`not_found`) when named. Re-running on the same run returns the stored row.
pub fn relate(root: &Path, repo: Option<&str>, family: Option<&str>) -> Result<Vec<Related>> {
    let store = Store::open(root)?;
    let repos: Vec<String> = match repo {
        Some(r) => vec![r.to_string()],
        None => store
            .repos()?
            .into_iter()
            .filter(|r| {
                r.id.is_some_and(|id| store.latest_done_run(id).ok().flatten().is_some())
            })
            .map(|r| r.name)
            .collect(),
    };
    let fams: Vec<String> = match family {
        Some(f) => vec![f.to_string()],
        None => store.families()?.into_iter().map(|f| f.name).collect(),
    };
    let mut out = vec![];
    for r in &repos {
        for f in &fams {
            out.push(relate_pair(&store, r, f)?);
        }
    }
    Ok(out)
}

/// The latest stored row per (repo, family), computing nothing: the row scored against the family's current pack when
/// there is one, else the newest. Pairs never scored are absent.
pub fn latest_relatedness(
    root: &Path,
    repo: Option<&str>,
    family: Option<&str>,
) -> Result<Vec<Related>> {
    let store = Store::open_existing(root)?;
    let mut st = store.conn().prepare(
        "SELECT r.name, fa.name, x.run_id, x.score, x.band, x.basis_json, x.pack_hash FROM relatedness x \
         JOIN repo r ON r.id = x.repo_id JOIN family fa ON fa.id = x.family_id \
         WHERE (?1 IS NULL OR r.name = ?1) AND (?2 IS NULL OR fa.name = ?2) \
         ORDER BY r.name, fa.name, x.run_id DESC, x.rowid DESC",
    )?;
    let rows = st
        .query_map(params![repo, family], |r| {
            let bj: String = r.get(5)?;
            Ok((
                r.get::<_, String>(6)?,
                Related {
                    repo: r.get(0)?,
                    family: r.get(1)?,
                    run_id: r.get(2)?,
                    score: r.get(3)?,
                    band: r.get(4)?,
                    basis: serde_json::from_str(&bj).unwrap_or_default(),
                },
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut hashes: std::collections::HashMap<String, String> = Default::default();
    let mut out: Vec<Related> = vec![];
    let mut i = 0;
    while i < rows.len() {
        let mut j = i;
        while j < rows.len()
            && rows[j].1.repo == rows[i].1.repo
            && rows[j].1.family == rows[i].1.family
        {
            j += 1;
        }
        let fam = rows[i].1.family.clone();
        if !hashes.contains_key(&fam) {
            hashes.insert(fam.clone(), pack_hash(&store, &fam)?);
        }
        let cur = &hashes[&fam];
        let pick = rows[i..j]
            .iter()
            .find(|(h, _)| h == cur)
            .unwrap_or(&rows[i]);
        out.push(pick.1.clone());
        i = j;
    }
    Ok(out)
}
