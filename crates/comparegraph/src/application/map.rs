//! `map` and `relate`: per-feature candidates from the graph (idf-weighted ranking of the nodes, `termscore.rs`, plus
//! communities whose label or keywords carry a term), and a relatedness band per (clone, family). Candidates are never verdicts.
use super::search::Hit;
use super::termscore::{feature_tokens, tok_match, Ranked, RunIndex};
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
/// A ranked symbol: the hit, and the informative terms its name carries.
#[derive(Debug, Serialize)]
pub struct SymbolCandidate {
    #[serde(flatten)]
    pub hit: Hit,
    pub matched: Vec<String>,
    /// graph position, as the ranking used it (`termscore::Reach`): `entry` (a root: an `entry-points` symbol), `reached`
    /// (called from a root, `entry_hops` away), `unreached` (called, but only from code no root reaches: ranked down),
    /// `uncalled` (no root reaches it and nothing outside tests calls it: left alone), `not_applicable` (not a
    /// function/method/route, or the run has no root)
    pub reach: &'static str,
    pub entry_hops: Option<u32>,
    /// other candidates in the same community whose names carry a term (reported, not scored: `termscore.rs`)
    pub community_peers: usize,
    /// set on a re-export shim (see `Shims`): the qualified name of the module it re-exports. The shim's ranking slot
    /// went to that target, listed above it; the shim itself takes no slot of `--limit`. Null otherwise.
    pub shim_of: Option<String>,
    /// the name carries one of the feature's own terms; what `relate` counts (see `termscore::Term::names_feature`)
    #[serde(skip)]
    pub named: bool,
}
#[derive(Debug, Serialize)]
pub struct FeatureMap {
    pub id: String,
    pub title: String,
    pub terms: Vec<String>,
    pub symbols: Vec<SymbolCandidate>,
    pub communities: Vec<CommunityCandidate>,
}
#[derive(Debug, Serialize)]
pub struct Mapped {
    pub repo: String,
    pub family: String,
    /// roots of the reach walk (`entry-points` symbols and entry files); 0 means the reach signal was off for every candidate
    pub entry_points: usize,
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

/// Tokens of the group, the slug, the title and the pack author's synonyms, without pack bookkeeping (`(pair row)`, and
/// `specific` inside a parenthesised note: `termscore::feature_tokens`); deduped and sorted. The definition is not used.
pub fn feature_terms(f: &FeatureRow) -> Vec<String> {
    let slug = f.slug.rsplit('.').next().unwrap_or(&f.slug);
    let set: BTreeSet<String> = feature_tokens(&f.grp)
        .into_iter()
        .chain(feature_tokens(slug))
        .chain(feature_tokens(&f.title))
        .chain(feature_tokens(&f.synonyms))
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

/// Re-export shims of a run, from data the store already holds: a `file`/`module` node that contains no symbol (no
/// `contains` edge out), calls nothing at module level (no `calls` edge out) and imports at least one file of the clone
/// (`imports` edges, which the reference provider writes for Python `from x import y` and Rust `mod`/`use`). Its body can
/// only be imports and plain assignments (`__all__ = [...]`): a compatibility module. The target is its one imported file,
/// or, when it imports several, the one whose stem is the shim's own (`agents/remote_agent.py` re-exporting
/// `a2a/remote_agent.py` among others); several or none: not a shim. Chains are followed at most `MAX_CHAIN` shims deep;
/// a cycle never reaches a non-shim within that bound, so it marks nothing. Keyed by index into `RunIndex::nodes`; a BTreeMap, so iteration is sorted.
#[derive(Debug, Default)]
pub(crate) struct Shims {
    target: std::collections::BTreeMap<usize, usize>,
}
const MAX_CHAIN: usize = 3;

/// File stem a shim and its target share: the base name without extension, or the directory for a package file.
fn stem(path: &str) -> &str {
    let mut parts = path.rsplit('/');
    let base = parts.next().unwrap_or(path);
    let s = base.split('.').next().unwrap_or(base);
    if matches!(s, "__init__" | "index" | "mod" | "lib") {
        parts.next().unwrap_or(s)
    } else {
        s
    }
}

impl Shims {
    pub(crate) fn load(store: &Store, idx: &RunIndex) -> Result<Self> {
        let at: std::collections::HashMap<i64, usize> = idx.nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
        let mut st = store.conn().prepare(
            "SELECT DISTINCT n.id, e.target_node_id FROM node n \
             JOIN edge e ON e.run_id = n.run_id AND e.source_node_id = n.id AND e.kind = 'imports' \
             WHERE n.run_id = ?1 AND n.kind IN ('file', 'module') AND e.target_node_id <> n.id \
             AND NOT EXISTS (SELECT 1 FROM edge x WHERE x.run_id = ?1 AND x.source_node_id = n.id AND x.kind IN ('contains', 'calls')) \
             ORDER BY n.id, e.target_node_id",
        )?;
        let mut imports: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
        for r in st.query_map(params![idx.run], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
            let (s, t) = r?;
            if let (Some(&s), Some(&t)) = (at.get(&s), at.get(&t)) {
                imports.entry(s).or_default().push(t);
            }
        }
        Ok(Self::from_imports(idx, &imports))
    }

    fn from_imports(idx: &RunIndex, imports: &std::collections::BTreeMap<usize, Vec<usize>>) -> Self {
        let direct: std::collections::BTreeMap<usize, usize> = imports
            .iter()
            .filter_map(|(&s, ts)| {
                let pick: Vec<usize> = if ts.len() == 1 {
                    ts.clone()
                } else {
                    let own = stem(&idx.nodes[s].path);
                    ts.iter().copied().filter(|&t| stem(&idx.nodes[t].path) == own).collect()
                };
                (pick.len() == 1).then(|| (s, pick[0]))
            })
            .collect();
        let target = direct
            .keys()
            .filter_map(|&s| {
                let mut t = direct[&s];
                for _ in 0..MAX_CHAIN {
                    match direct.get(&t) {
                        Some(&next) => t = next,
                        None => return Some((s, t)),
                    }
                }
                None
            })
            .collect();
        Self { target }
    }
}

fn candidate(repo: &str, idx: &RunIndex, r: &Ranked, shim_of: Option<String>) -> SymbolCandidate {
    let n = &idx.nodes[r.i];
    SymbolCandidate {
        hit: Hit {
            repo: repo.into(),
            qualified_name: n.qn.clone(),
            kind: n.kind.to_string(),
            path: n.path.to_string(),
            line_start: n.ls,
            line_end: n.le,
            community: idx.community_label(r.i),
            score: r.score,
            label: "candidate",
        },
        matched: r.matched.clone(),
        reach: r.reach.label(),
        entry_hops: r.reach.hops(),
        community_peers: r.peers,
        shim_of,
        named: r.named,
    }
}

fn map_feature(
    repo: &str,
    idx: &RunIndex,
    shims: &Shims,
    f: &FeatureRow,
    limit: usize,
) -> Result<FeatureMap> {
    let wt = idx.terms(f);
    let terms: Vec<String> = wt.iter().filter(|t| t.informative()).map(|t| t.text.clone()).collect();
    // with no shim this is `rank(.., limit)` exactly; with shims the whole ranked pool (rank sorts it whole either way) is
    // walked so a target ranked below its shim, or not ranked at all, can take the shim's slot
    let pool = idx.rank(f, &wt, if shims.target.is_empty() { limit } else { usize::MAX });
    let mut symbols: Vec<SymbolCandidate> = vec![];
    let mut shown: std::collections::HashSet<usize> = Default::default();
    let mut slots = 0;
    for r in &pool {
        if slots == limit {
            break;
        }
        if shown.contains(&r.i) {
            continue;
        }
        let Some(&t) = shims.target.get(&r.i) else {
            shown.insert(r.i);
            symbols.push(candidate(repo, idx, r, None));
            slots += 1;
            continue;
        };
        if shown.insert(t) {
            // the target, at the shim's slot and at least the shim's score; its own matched terms when it ranked
            let own = pool.iter().find(|p| p.i == t);
            let tr = Ranked {
                i: t,
                score: own.map_or(r.score, |o| o.score.max(r.score)),
                matched: own.map(|o| o.matched.clone()).unwrap_or_default(),
                named: own.is_some_and(|o| o.named),
                reach: idx.reach(t),
                peers: own.map_or(0, |o| o.peers),
            };
            symbols.push(candidate(repo, idx, &tr, None));
            slots += 1;
        }
        shown.insert(r.i);
        symbols.push(candidate(repo, idx, r, Some(idx.nodes[t].qn.clone())));
    }
    let mut communities: Vec<CommunityCandidate> = idx
        .communities()
        .filter_map(|(label, have)| {
            let matched: Vec<String> = terms.iter().filter(|t| have.iter().any(|k| tok_match(t, k))).cloned().collect();
            (!matched.is_empty()).then(|| CommunityCandidate {
                label: label.to_string(),
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
    let idx = RunIndex::load(&store, run)?;
    let shims = Shims::load(&store, &idx)?;
    let features = feats
        .iter()
        .map(|f| map_feature(repo, &idx, &shims, f, limit))
        .collect::<Result<Vec<_>>>()?;
    Ok(Mapped {
        repo: repo.into(),
        family: family.into(),
        entry_points: idx.entries,
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

fn relate_pair(store: &Store, repo: &str, family: &str, rid: i64, run: i64, idx: &RunIndex) -> Result<Related> {
    let fam = store
        .family_by_name(family)?
        .ok_or_else(|| AppError::NotFound(format!("no family named {family}")))?;
    let feats = active(store, family)?;
    let ph = pack_hash(store, family)?;
    let mut basis = vec![];
    for f in &feats {
        // relatedness is scored on the plain ranking: a shim's slot is a `map` display choice
        let m = map_feature(repo, idx, &Shims::default(), f, 1)?;
        if m.symbols.first().is_some_and(|s| s.named) {
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
        // the index depends on the run only: one per repo, loaded on the first family
        let mut run_idx: Option<(i64, i64, RunIndex)> = None;
        for f in &fams {
            if run_idx.is_none() {
                let (rid, run) = latest_done(&store, r)?;
                run_idx = Some((rid, run, RunIndex::load(&store, run)?));
            }
            let (rid, run, idx) = run_idx.as_ref().unwrap();
            out.push(relate_pair(&store, r, f, *rid, *run, idx)?);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn feature() -> FeatureRow {
        FeatureRow {
            id: 1,
            family: "f".into(),
            grp: "quality".into(),
            slug: "quality.widget".into(),
            title: "Widget".into(),
            definition: "".into(),
            counts_as_has: "".into(),
            tier: "core".into(),
            weight: 1,
            synonyms: "".into(),
        }
    }
    /// eleven unrelated rows plus `extra`
    fn run(extra: (&'static str, &'static str, &'static str)) -> RunIndex {
        let mut rows: Vec<(&str, &str, &str)> = vec![
            ("function", "alpha", "src/a.ts"),
            ("function", "beta", "src/b.ts"),
            ("function", "gamma", "src/c.ts"),
            ("function", "delta", "src/d.ts"),
            ("function", "epsilon", "src/e.ts"),
            ("function", "zeta", "src/f.ts"),
            ("function", "theta", "src/g.ts"),
            ("function", "iota", "src/h.ts"),
            ("function", "kappa", "src/i.ts"),
            ("function", "lambda", "src/j.ts"),
            ("function", "omicron", "src/k.ts"),
        ];
        rows.push(extra);
        assert_eq!(rows.len(), 12);
        RunIndex::from_rows(&rows)
    }

    fn shims_of(paths: &[&'static str], imports: &[(usize, &[usize])]) -> Vec<(usize, usize)> {
        let rows: Vec<(&str, &str, &str)> = paths.iter().map(|p| ("file", *p, *p)).collect();
        let idx = RunIndex::from_rows(&rows);
        let m = imports.iter().map(|(s, ts)| (*s, ts.to_vec())).collect();
        Shims::from_imports(&idx, &m).target.into_iter().collect()
    }

    #[test]
    fn a_shim_target_is_its_one_import_or_the_one_sharing_its_stem() {
        let p = ["agents/remote_agent.py", "a2a/remote_agent.py", "a2a/util.py", "a2a/other.py"];
        assert_eq!(shims_of(&p, &[(0, &[1])]), vec![(0, 1)]);
        assert_eq!(shims_of(&p, &[(0, &[1, 2])]), vec![(0, 1)], "stem tie-break");
        assert!(shims_of(&p, &[(0, &[2, 3])]).is_empty(), "several imports, none its own stem: an aggregator");
        let pkg = ["web/remote/__init__.py", "web/remote.py"];
        assert_eq!(shims_of(&pkg, &[(0, &[1])]), vec![(0, 1)]);
        assert_eq!(stem("web/remote/__init__.py"), "remote");
        assert_eq!(stem("src/inner/mod.rs"), "inner");
        assert_eq!(stem("a/b.test.ts"), "b");
    }

    #[test]
    fn a_shim_chain_resolves_to_the_real_module_and_a_cycle_to_nothing() {
        let p = ["a.py", "b.py", "c.py", "d.py", "e.py"];
        assert_eq!(shims_of(&p, &[(0, &[1]), (1, &[2])]), vec![(0, 2), (1, 2)]);
        assert!(shims_of(&p, &[(0, &[1]), (1, &[0])]).is_empty(), "cycle");
        // deeper than MAX_CHAIN: the head marks nothing, the rest resolve
        let deep = shims_of(&p, &[(0, &[1]), (1, &[2]), (2, &[3]), (3, &[4])]);
        assert_eq!(deep, vec![(1, 4), (2, 4), (3, 4)]);
    }

    #[test]
    fn a_path_only_match_does_not_count_toward_the_basis() {
        let idx = run(("function", "serve", "src/widget/serve.ts"));
        let m = map_feature("r", &idx, &Shims::default(), &feature(), 1).unwrap();
        assert_eq!(m.terms, vec!["widget".to_string()]);
        assert_eq!(m.symbols.len(), 1, "the path hit is still a candidate");
        assert!(m.symbols[0].matched.is_empty(), "no name carries the term");
        assert!(!m.symbols[0].named);
    }

    #[test]
    fn a_node_named_for_the_feature_counts() {
        let idx = run(("function", "widget_something", "src/other/serve.ts"));
        let m = map_feature("r", &idx, &Shims::default(), &feature(), 1).unwrap();
        assert_eq!(m.symbols[0].matched, vec!["widget".to_string()]);
        assert!(m.symbols[0].named);
    }

    #[test]
    fn a_definition_word_alone_does_not_name_the_feature() {
        let mut f = feature();
        f.definition = "renders gadgets".into();
        let idx = run(("function", "gadgets", "src/other/serve.ts"));
        let m = map_feature("r", &idx, &Shims::default(), &f, 1).unwrap();
        assert_eq!(m.symbols[0].matched, vec!["gadgets".to_string()]);
        assert!(!m.symbols[0].named, "a definition token has source weight 0.5");
    }

    #[test]
    fn feature_terms_drop_pack_bookkeeping() {
        let mut f = feature();
        f.title = "Provider retry (pair row)".into();
        f.synonyms = "backoff (rows), jitter (specifically, etc)".into();
        assert_eq!(feature_terms(&f), vec!["backoff", "jitter", "provider", "quality", "retry", "widget"]);
    }

    #[test]
    fn feature_terms_keep_row_outside_a_note() {
        let mut f = feature();
        f.grp = "grid".into();
        f.slug = "grid.rows".into();
        f.title = "Row virtualization".into();
        f.synonyms = String::new();
        assert_eq!(feature_terms(&f), vec!["grid", "row", "rows", "virtualization"]);
    }

    #[test]
    fn a_feature_whose_terms_are_all_common_maps_to_nothing() {
        let names: Vec<String> = (0..100).map(|i| if i < 20 { format!("graph_zq{i}") } else { format!("item_zq{i}") }).collect();
        let rows: Vec<(&str, &str, &str)> = names.iter().map(|s| ("function", s.as_str(), "a.ts")).collect();
        let idx = RunIndex::from_rows(&rows);
        let mut f = feature();
        f.grp = "zz".into();
        f.slug = "zz.zz".into();
        f.title = "graph".into();
        let m = map_feature("r", &idx, &Shims::default(), &f, 5).unwrap();
        assert!(m.symbols.is_empty() && m.terms.is_empty());
    }
}
