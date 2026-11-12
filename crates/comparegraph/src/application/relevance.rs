//! `relevance` (lane CG-STORE B1): per repo, the factor counts over the store and the files a policy names, and - when
//! the user supplies a `relevance-policy/1` file - a score and a class. Reads only (the store is opened read-only).
//! Deterministic: as-of defaults to the newest timestamp in the store, not the wall clock; every input file's sha256
//! and a digest of the store are in `data.inputs`.
use super::{AppError, Result};
use crate::domain::actionlog::sha256_hex;
use crate::domain::relevance::{self as rel, norm_name, parse_date_days, Factors, Policy, UiRef};
use crate::store::Store;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

pub const MAX_LIMIT: usize = 500;

/// A policy as loaded: the parsed form, where it was found, the sha256 of its bytes.
#[derive(Debug, Clone)]
pub struct LoadedPolicy {
    pub policy: Policy,
    pub path: PathBuf,
    pub sha256: String,
}

fn invalid_policy(msg: String) -> AppError {
    AppError::structured("invalid_policy", msg, Some("fix the policy file; unknown keys are refused, never ignored".into()), None)
}

/// The policy at `path`, else `<root>/.comparegraph/policy.json` when present, else None.
pub fn load_policy(root: &Path, path: Option<&Path>) -> Result<Option<LoadedPolicy>> {
    let p = match path {
        Some(p) => p.to_path_buf(),
        None => {
            let d = root.join(".comparegraph").join("policy.json");
            if !d.is_file() {
                return Ok(None);
            }
            d
        }
    };
    let bytes = std::fs::read(&p).map_err(|e| AppError::Io(p.clone(), e))?;
    let text = String::from_utf8(bytes.clone()).map_err(|_| invalid_policy(format!("{}: not UTF-8", p.display())))?;
    let policy = Policy::parse(&text).map_err(|e| invalid_policy(format!("{}: {e}", p.display())))?;
    Ok(Some(LoadedPolicy { policy, path: p, sha256: sha256_hex(&bytes) }))
}

/// RFC 4180 CSV: quoted fields with `""` escapes and embedded newlines; CRLF or LF.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = vec![];
    let mut row = vec![];
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// The input files a policy names, read once: their text and sha256, by the path as written in the policy.
struct Inputs {
    base: PathBuf,
    files: BTreeMap<String, String>,
}
impl Inputs {
    fn read(&mut self, rel_path: &str) -> Result<String> {
        let p = self.base.join(rel_path);
        let bytes = std::fs::read(&p).map_err(|e| invalid_policy(format!("the policy names {rel_path}, which cannot be read at {}: {e}", p.display())))?;
        self.files.insert(rel_path.to_string(), sha256_hex(&bytes));
        String::from_utf8(bytes).map_err(|_| invalid_policy(format!("{rel_path}: not UTF-8")))
    }
    fn csv(&mut self, rel_path: &str) -> Result<(Vec<String>, Vec<Vec<String>>)> {
        let mut rows = parse_csv(&self.read(rel_path)?);
        if rows.is_empty() {
            return Ok((vec![], vec![]));
        }
        let header = rows.remove(0).into_iter().map(|h| h.trim().to_string()).collect();
        Ok((header, rows))
    }
}
fn col(header: &[String], name: &str, file: &str) -> Result<usize> {
    header.iter().position(|h| h == name).ok_or_else(|| invalid_policy(format!("{file} has no column {name:?} (columns: {})", header.join(", "))))
}

/// One repo, rated.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Rated {
    pub repo: String,
    pub role: String,
    pub class: String,
    pub rule: String,
    pub pinned: bool,
    pub score: Option<f64>,
    pub factors: Factors,
    /// nodes of the latest done run (a cost shown beside the score, never part of it)
    pub size_nodes: i64,
    pub latest_run_status: Option<String>,
    pub no_graph: bool,
    pub licence: Option<String>,
    pub lane: String,
    pub added_at: String,
    pub archived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terms: Option<rel::Terms>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bands: Option<Vec<Value>>,
}

pub struct Rating {
    pub items: Vec<Rated>,
    pub as_of: String,
    pub inputs: Value,
    pub limitations: Vec<Value>,
}

/// Rate every repo of an open store (read-only is enough). `family` restricts every factor to that family.
pub fn rate(store: &Store, root: &Path, policy: Option<&LoadedPolicy>, family: Option<&str>, as_of: Option<&str>) -> Result<Rating> {
    if let Some(f) = family {
        if store.family_by_name(f)?.is_none() {
            return Err(AppError::NotFound(format!("no family named {f}")));
        }
    }
    let as_of = match as_of {
        Some(a) => {
            parse_date_days(a).ok_or_else(|| AppError::InvalidArguments(format!("--as-of {a:?} is not a YYYY-MM-DD date")))?;
            a.to_string()
        }
        None => store.newest_timestamp()?.unwrap_or_else(|| "1970-01-01".into()),
    };
    let as_of_days = parse_date_days(&as_of).unwrap_or(0);
    let archived = super::archive::archived_names(store)?;
    let mut limitations: Vec<Value> = vec![];
    // live claims per repo
    let mut evidence: HashMap<String, u64> = HashMap::new();
    let mut other: HashMap<String, u64> = HashMap::new();
    let mut last: HashMap<String, String> = HashMap::new();
    let mut claimed_fams: HashMap<String, Vec<String>> = HashMap::new();
    for (repo, fam, verdict, created) in store.live_claims()? {
        if family.is_some_and(|f| f != fam) {
            continue;
        }
        if verdict == "has" || verdict == "partial" {
            *evidence.entry(repo.clone()).or_default() += 1;
        } else {
            *other.entry(repo.clone()).or_default() += 1;
        }
        let e = last.entry(repo.clone()).or_default();
        if created > *e {
            *e = created;
        }
        claimed_fams.entry(repo).or_default().push(fam);
    }
    // bands: the latest stored row per (repo, family)
    let mut bands: HashMap<String, Vec<Value>> = HashMap::new();
    let mut best: HashMap<String, String> = HashMap::new();
    let rank = |b: &str| match b {
        "related" => 2,
        "adjacent" => 1,
        _ => 0,
    };
    for r in super::map::latest_relatedness_in(store, None, family)? {
        bands.entry(r.repo.clone()).or_default().push(json!({"family": r.family, "band": r.band, "score": r.score}));
        if claimed_fams.get(&r.repo).is_some_and(|f| f.contains(&r.family)) {
            let cur = best.get(&r.repo).map(|b| rank(b)).unwrap_or(-1);
            if rank(&r.band) > cur {
                best.insert(r.repo.clone(), r.band.clone());
            }
        }
    }
    // the policy's files
    let mut pull: HashMap<String, u64> = HashMap::new();
    let mut cited: HashMap<String, String> = HashMap::new();
    let mut ui: HashMap<String, UiRef> = HashMap::new();
    let mut inputs = Inputs { base: PathBuf::new(), files: BTreeMap::new() };
    if let Some(lp) = policy {
        inputs.base = lp.path.parent().map(Path::to_path_buf).unwrap_or_default();
        let p = &lp.policy;
        // aliases: a name in another file -> the registered repo name
        let mut alias: HashMap<String, String> = HashMap::new();
        if let Some(a) = &p.aliases {
            let (h, rows) = inputs.csv(&a.file)?;
            let (fi, ti) = (col(&h, &a.from, &a.file)?, col(&h, &a.to, &a.file)?);
            for r in rows {
                if let (Some(f), Some(t)) = (r.get(fi), r.get(ti)) {
                    if !f.is_empty() && !t.is_empty() {
                        alias.insert(norm_name(f), norm_name(t));
                    }
                }
            }
        }
        let resolve = |n: &str| -> String {
            let k = norm_name(n);
            alias.get(&k).cloned().unwrap_or(k)
        };
        if let Some(pl) = &p.pull {
            let (h, rows) = inputs.csv(&pl.file)?;
            let ri = col(&h, &pl.repo_column, &pl.file)?;
            let only = match &pl.only {
                Some(o) => Some((col(&h, &o.column, &pl.file)?, o.values.clone())),
                None => None,
            };
            for r in rows {
                if let Some((ci, vals)) = &only {
                    if !r.get(*ci).is_some_and(|v| vals.contains(v)) {
                        continue;
                    }
                }
                if let Some(n) = r.get(ri).filter(|n| !n.is_empty()) {
                    *pull.entry(resolve(n)).or_default() += 1;
                }
            }
        } else {
            limitations.push(json!({"code": "factor_not_configured", "detail": "the policy names no pull file: pull_rows is 0 for every repo"}));
        }
        if let Some(c) = &p.cited {
            let (h, rows) = inputs.csv(&c.file)?;
            let (ri, di) = (col(&h, &c.repo_column, &c.file)?, col(&h, &c.date_column, &c.file)?);
            for r in rows {
                if let (Some(n), Some(d)) = (r.get(ri), r.get(di)) {
                    if parse_date_days(d).is_some() {
                        let e = cited.entry(resolve(n)).or_default();
                        if *d > *e {
                            *e = d.clone();
                        }
                    }
                }
            }
        }
        if let Some(u) = &p.ui {
            let text = inputs.read(&u.file)?;
            let v: Value = serde_json::from_str(&text).map_err(|e| invalid_policy(format!("{}: {e}", u.file)))?;
            if v["schema"] != "ui-registry/1" {
                return Err(invalid_policy(format!("{} is not a ui-registry/1 file", u.file)));
            }
            if let Some(m) = v["products"].as_object() {
                for (k, prod) in m {
                    let r = if prod["manifest"] == "proven" { UiRef::Proven } else { UiRef::Listed };
                    ui.insert(resolve(k), r);
                }
            }
        } else {
            limitations.push(json!({"code": "factor_not_configured", "detail": "the policy names no ui registry: ui is `no` for every repo"}));
        }
        for pin in &p.pins {
            if parse_date_days(&pin.until).is_some_and(|u| as_of_days > u) {
                limitations.push(json!({"code": "pin_expired", "detail": format!("pin of {} expired on {} (as-of {as_of})", pin.repo, pin.until)}));
            }
        }
    } else {
        limitations.push(json!({"code": "unrated", "detail": "no policy file (--policy or .comparegraph/policy.json): factors only, every class `unrated`; the engine ships no weights"}));
    }
    let mut items = vec![];
    for r in store.repos()? {
        let id = r.id.unwrap();
        let latest = store.latest_run(id)?;
        let done = store.latest_done_run(id)?;
        let size = match &done {
            Some(d) => store.node_count(d.id.unwrap())?,
            None => 0,
        };
        let key = norm_name(&r.name);
        let last_cited = [last.get(&r.name).cloned(), cited.get(&key).cloned()].into_iter().flatten().max();
        let f = Factors {
            evidence_claims: evidence.get(&r.name).copied().unwrap_or(0),
            other_claims: other.get(&r.name).copied().unwrap_or(0),
            pull_rows: pull.get(&key).copied().unwrap_or(0),
            days_since_cited: last_cited.as_deref().and_then(parse_date_days).map(|d| as_of_days - d),
            best_band: best.get(&r.name).cloned(),
            ui: ui.get(&key).copied().unwrap_or(UiRef::No),
            days_since_added: parse_date_days(&r.added_at).map(|d| as_of_days - d),
        };
        let (class, rule, pinned, score, terms) = match policy {
            Some(lp) => {
                let t = rel::score(&f, &lp.policy);
                let c = rel::classify(&r.name, r.role == "root", &f, &t, &lp.policy, as_of_days);
                (c.class, c.rule, c.pinned, Some(t.total), Some(t))
            }
            None if r.role == "root" => ("ROOT".into(), "repo.role = root".into(), false, None, None),
            None => ("unrated".into(), "no policy".into(), false, None, None),
        };
        items.push(Rated {
            repo: r.name.clone(),
            role: r.role.clone(),
            class,
            rule,
            pinned,
            score,
            factors: f,
            size_nodes: size,
            latest_run_status: latest.map(|l| l.status.as_str().to_string()),
            no_graph: done.is_none(),
            licence: r.licence.clone(),
            lane: r.lane.clone(),
            added_at: r.added_at.clone(),
            archived: archived.contains(&r.name),
            terms,
            bands: bands.remove(&r.name),
        });
    }
    let order = |c: &str| rel::CLASS_ORDER.iter().position(|x| *x == c).unwrap_or(rel::CLASS_ORDER.len());
    items.sort_by(|a, b| {
        order(&a.class)
            .cmp(&order(&b.class))
            .then(b.score.unwrap_or(0.0).partial_cmp(&a.score.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.repo.cmp(&b.repo))
    });
    let mut fams: BTreeMap<String, String> = BTreeMap::new();
    for f in store.families()? {
        if family.is_none_or(|x| x == f.name) {
            fams.insert(f.name, f.definition_hash);
        }
    }
    let inputs = json!({
        "engine": super::governance::engine(),
        "policy": policy.map(|p| p.path.display().to_string()),
        "policy_sha256": policy.map(|p| p.sha256.clone()),
        "files": inputs.files,
        "families": fams,
        "store_digest": super::backup::store_digest(store)?,
        "as_of": as_of,
    });
    let _ = root;
    Ok(Rating { items, as_of, inputs, limitations })
}

pub struct Opts<'a> {
    pub policy: Option<&'a Path>,
    pub family: Option<&'a str>,
    pub explain: Option<&'a str>,
    pub class: Option<&'a str>,
    pub as_of: Option<&'a str>,
    pub limit: usize,
    pub cursor: Option<&'a str>,
}

/// `relevance`: the paged rating, or one repo with every term (`--explain`).
pub fn relevance(root: &Path, o: &Opts) -> Result<Value> {
    if o.limit == 0 || o.limit > MAX_LIMIT {
        return Err(AppError::InvalidArguments(format!("--limit must be 1..={MAX_LIMIT}")));
    }
    let offset = match o.cursor {
        Some(c) => c.parse::<usize>().map_err(|_| AppError::InvalidArguments(format!("bad cursor {c}")))?,
        None => 0,
    };
    let policy = load_policy(root, o.policy)?;
    let store = Store::open_read_only(root)?;
    let mut r = rate(&store, root, policy.as_ref(), o.family, o.as_of)?;
    if let Some(name) = o.explain {
        let Some(item) = r.items.into_iter().find(|i| i.repo == name) else {
            return Err(AppError::NotFound(format!("no repo named {name}")));
        };
        let weights = policy.as_ref().map(|p| serde_json::to_value(&p.policy.weights).unwrap());
        return Ok(json!({
            "repo": item, "weights": weights, "policy_classes": policy.as_ref().map(|p| serde_json::to_value(&p.policy.classes).unwrap()),
            "as_of": r.as_of, "inputs": r.inputs, "evidence_confidence": "derived", "limitation_items": r.limitations,
        }));
    }
    if let Some(c) = o.class {
        r.items.retain(|i| i.class == c);
    }
    for i in r.items.iter_mut() {
        i.terms = None;
        i.bands = None;
    }
    let total = r.items.len();
    let page: Vec<&Rated> = r.items.iter().skip(offset).take(o.limit).collect();
    let next = (offset + page.len() < total).then(|| (offset + page.len()).to_string());
    if next.is_some() {
        r.limitations.push(json!({"code": "truncated", "detail": format!("{} of {total} repos on this page; pass the cursor for the next", page.len())}));
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for i in &r.items {
        *counts.entry(i.class.clone()).or_default() += 1;
    }
    Ok(json!({
        "items": page, "classes": counts, "as_of": r.as_of, "inputs": r.inputs,
        "evidence_confidence": "derived", "limitation_items": r.limitations,
        "page": {"next": next, "total": total},
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv_handles_quotes_commas_and_newlines() {
        let rows = parse_csv("a,b\r\n\"x, y\",\"he said \"\"hi\"\"\"\n\"multi\nline\",z\n");
        assert_eq!(rows, vec![vec!["a", "b"], vec!["x, y", "he said \"hi\""], vec!["multi\nline", "z"]]);
    }
}
