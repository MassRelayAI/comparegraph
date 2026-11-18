//! CG-DELTA: what a branch holds that its base does not, and how much of that the base already has under another name or path.
//! Two runs built with `build --ref` are compared symbol by symbol; the engine reports facts and writes no verdict (restack, take
//! parts and retire are a reader's reading of the fields).
//!
//! Identity is `(kind, qualified_name)`. A symbol's body is its source lines normalised to a token stream (whitespace ignored, the
//! symbol's own name replaced by a placeholder), hashed with sha256 (already a dependency). From that:
//! - `added`: in head, not in base. `removed`: in base, not in head. `changed`: both, bodies differ.
//! - head-only = added + changed. Each is looked up on the base elsewhere: the same body hash under another name or path is
//!   `body_hash` at confidence 1.0; else a base symbol of the same name and kind whose token-bigram Jaccard is >= 0.9 is
//!   `name_and_tokens` at that similarity (floored to two decimals, never above 0.99). Nothing below 0.9 is reported.
//! - the families and features each head-only symbol ranks under come from `map`'s own ranking (`map::symbol_features`).
use super::{gitref, AppError, Result};
use crate::store::Store;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

/// How deep into each feature's ranking a symbol still counts as mapped to it.
pub const FEATURE_POOL: usize = 500;
/// The similarity floor of the `name_and_tokens` rule.
pub const SIMILARITY_FLOOR: f64 = 0.9;
const MAX_MATCHES: usize = 3;
const MAX_FEATURES_PER_ROW: usize = 10;

pub struct DeltaOpts<'a> {
    pub base: &'a str,
    pub head: &'a str,
    pub family: Option<&'a str>,
    pub max_nodes: usize,
}

#[derive(Debug, Clone)]
struct Sym {
    kind: String,
    name: String,
    qn: String,
    path: String,
    ls: u32,
    le: u32,
    hash: String,
}

struct Side {
    run: i64,
    commit: String,
    syms: BTreeMap<(String, String), Sym>,
    files: HashMap<String, Vec<String>>,
    unreadable: usize,
    duplicates: usize,
}

fn tokens(text: &str, name: &str) -> Vec<String> {
    let mut out = vec![];
    let mut word = String::new();
    let flush = |w: &mut String, out: &mut Vec<String>| {
        if !w.is_empty() {
            out.push(if w == name { "\u{1}NAME".to_string() } else { w.clone() });
            w.clear();
        }
    };
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            if !c.is_whitespace() {
                out.push(c.to_string());
            }
        }
    }
    flush(&mut word, &mut out);
    out
}

fn body_of(files: &HashMap<String, Vec<String>>, s: &Sym) -> Option<String> {
    let lines = files.get(&s.path)?;
    let (a, b) = (s.ls as usize, s.le as usize);
    if a == 0 || b < a || a > lines.len() {
        return None;
    }
    Some(lines[a - 1..b.min(lines.len())].join("\n"))
}

fn bigrams(toks: &[String]) -> HashSet<String> {
    if toks.len() < 2 {
        return toks.iter().cloned().collect();
    }
    toks.windows(2).map(|w| format!("{}\u{0}{}", w[0], w[1])).collect()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let i = a.intersection(b).count() as f64;
    i / (a.len() as f64 + b.len() as f64 - i)
}

type NodeRow = (String, String, String, String, i64, i64);

fn load_side(store: &Store, root: &Path, repo: &crate::store::RepoRow, git_ref: &str) -> Result<Side> {
    let clone = Path::new(&repo.path);
    let commit = gitref::resolve(clone, git_ref)?;
    let rid = repo.id.expect("stored repo has id");
    let run = store.done_run_at_commit(rid, &commit)?.ok_or_else(|| {
        AppError::structured(
            "not_found",
            format!("{} has no finished run at {git_ref} ({})", repo.name, &commit[..12]),
            Some(format!("run `comparegraph build {} --ref {git_ref}` first (the ref may have moved since it was built)", repo.name)),
            Some(json!({ "ref": git_ref, "commit": commit, "repo": repo.name })),
        )
    })?;
    let run_id = run.id.expect("stored run has id").0;
    let dir = gitref::materialise(clone, root, &repo.name, &commit)?;
    let mut rows: Vec<NodeRow> = vec![];
    {
        let mut st = store.conn().prepare(
            "SELECT kind, name, qualified_name, path, line_start, line_end FROM node WHERE run_id=?1 AND kind IN ('function','method','class','type','route') \
             ORDER BY qualified_name, kind, path, line_start",
        )?;
        for r in st.query_map([run_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))? {
            rows.push(r?);
        }
    }
    let mut side = Side { run: run_id, commit, syms: BTreeMap::new(), files: HashMap::new(), unreadable: 0, duplicates: 0 };
    for (kind, name, qn, path, ls, le) in rows {
        if !side.files.contains_key(&path) {
            let text = std::fs::read(dir.join(&path)).ok().and_then(|b| String::from_utf8(b).ok());
            side.files.insert(path.clone(), text.map(|t| t.lines().map(String::from).collect()).unwrap_or_default());
        }
        let mut s = Sym { kind: kind.clone(), name, qn: qn.clone(), path, ls: ls as u32, le: le as u32, hash: String::new() };
        let Some(body) = body_of(&side.files, &s) else {
            side.unreadable += 1;
            continue;
        };
        let mut h = Sha256::new();
        h.update(tokens(&body, &s.name).join("\u{0}").as_bytes());
        s.hash = hex::encode(h.finalize());
        if side.syms.insert((kind, qn), s).is_some() {
            side.duplicates += 1;
        }
    }
    Ok(side)
}

#[derive(Serialize)]
struct Match {
    qualified_name: String,
    path: String,
}
#[derive(Serialize)]
struct OnBase {
    confidence: f64,
    via: &'static str,
    matches: Vec<Match>,
}
#[derive(Serialize)]
struct Row {
    qualified_name: String,
    kind: String,
    path: String,
    line_start: u32,
    line_end: u32,
    body_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_body_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    already_on_base: Option<OnBase>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    features: Vec<String>,
}

fn row(s: &Sym) -> Row {
    Row {
        qualified_name: s.qn.clone(),
        kind: s.kind.clone(),
        path: s.path.clone(),
        line_start: s.ls,
        line_end: s.le,
        body_hash: s.hash.clone(),
        base_body_hash: None,
        already_on_base: None,
        features: vec![],
    }
}
fn sort_rows(v: &mut [Row]) {
    v.sort_by(|a, b| (&a.path, a.line_start, &a.qualified_name, &a.kind).cmp(&(&b.path, b.line_start, &b.qualified_name, &b.kind)));
}

fn git_lines(clone: &Path, args: &[&str]) -> Option<Vec<String>> {
    let out = std::process::Command::new("git").arg("-C").arg(clone).args(["-c", "core.quotepath=false"]).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).lines().map(String::from).collect())
}

pub fn delta(root: &Path, repo_name: &str, o: &DeltaOpts) -> Result<Value> {
    let store = Store::open_existing(root)?;
    let repo = store.repo_by_name(repo_name)?.ok_or_else(|| AppError::NotFound(format!("no repo named {repo_name}")))?;
    super::archive::refuse_if_archived(&store, repo_name)?;
    if !(1..=2000).contains(&o.max_nodes) {
        return Err(AppError::InvalidArguments("--max-nodes must be 1..=2000".into()));
    }
    let base = load_side(&store, root, &repo, o.base)?;
    let head = load_side(&store, root, &repo, o.head)?;

    let mut by_hash: HashMap<&str, Vec<&Sym>> = HashMap::new();
    let mut by_name: HashMap<(&str, &str), Vec<&Sym>> = HashMap::new();
    for s in base.syms.values() {
        by_hash.entry(s.hash.as_str()).or_default().push(s);
        by_name.entry((s.name.as_str(), s.kind.as_str())).or_default().push(s);
    }
    let on_base = |s: &Sym| -> Option<OnBase> {
        let mk = |v: &[&Sym]| -> Vec<Match> {
            let mut m: Vec<Match> = v.iter().map(|b| Match { qualified_name: b.qn.clone(), path: b.path.clone() }).collect();
            m.sort_by(|a, b| (&a.path, &a.qualified_name).cmp(&(&b.path, &b.qualified_name)));
            m.truncate(MAX_MATCHES);
            m
        };
        let exact: Vec<&Sym> = by_hash.get(s.hash.as_str()).map(|v| v.iter().copied().filter(|b| b.qn != s.qn || b.kind != s.kind).collect()).unwrap_or_default();
        if !exact.is_empty() {
            return Some(OnBase { confidence: 1.0, via: "body_hash", matches: mk(&exact) });
        }
        let cands: Vec<&Sym> = by_name.get(&(s.name.as_str(), s.kind.as_str())).map(|v| v.iter().copied().filter(|b| b.qn != s.qn).collect()).unwrap_or_default();
        if cands.is_empty() {
            return None;
        }
        let mine = bigrams(&tokens(&body_of(&head.files, s)?, &s.name));
        let mut best: Vec<(f64, &Sym)> = vec![];
        for c in cands {
            let theirs = bigrams(&tokens(&body_of(&base.files, c)?, &c.name));
            let sim = jaccard(&mine, &theirs);
            if sim >= SIMILARITY_FLOOR {
                best.push((sim, c));
            }
        }
        best.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then((&a.1.path, &a.1.qn).cmp(&(&b.1.path, &b.1.qn))));
        let top = best.first()?.0;
        let confidence = ((top * 100.0).floor() / 100.0).min(0.99);
        let at_top: Vec<&Sym> = best.iter().filter(|b| b.0 == top).map(|b| b.1).collect();
        Some(OnBase { confidence, via: "name_and_tokens", matches: mk(&at_top) })
    };

    let (mut added, mut removed, mut changed) = (vec![], vec![], vec![]);
    let mut unchanged = 0usize;
    for (k, h) in &head.syms {
        match base.syms.get(k) {
            None => {
                let mut r = row(h);
                r.already_on_base = on_base(h);
                added.push(r);
            }
            Some(b) if b.hash != h.hash => {
                let mut r = row(h);
                r.base_body_hash = Some(b.hash.clone());
                r.already_on_base = on_base(h);
                changed.push(r);
            }
            Some(_) => unchanged += 1,
        }
    }
    for (k, b) in &base.syms {
        if !head.syms.contains_key(k) {
            removed.push(row(b));
        }
    }

    // families and features of the head-only symbols (`map`'s ranking, deeper)
    let families: Vec<String> = match o.family {
        Some(f) => vec![f.to_string()],
        None => store.families()?.into_iter().map(|f| f.name).collect(),
    };
    let mut touched: BTreeMap<String, usize> = BTreeMap::new();
    if !added.is_empty() || !changed.is_empty() {
        let mut of: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for f in &families {
            for (qn, feats) in super::map::symbol_features(&store, head.run, f, FEATURE_POOL)? {
                of.entry(qn).or_default().extend(feats);
            }
        }
        for r in added.iter_mut().chain(changed.iter_mut()) {
            if let Some(f) = of.get(&r.qualified_name) {
                let mut f = f.clone();
                f.sort();
                f.dedup();
                for x in &f {
                    *touched.entry(x.clone()).or_default() += 1;
                }
                f.truncate(MAX_FEATURES_PER_ROW);
                r.features = f;
            }
        }
    }

    // the files the base moved under the branch since the merge base
    let clone = Path::new(&repo.path);
    let merge_base = git_lines(clone, &["merge-base", &base.commit, &head.commit]).and_then(|v| v.into_iter().next());
    let (mut deleted, mut rewritten) = (vec![], vec![]);
    let mut head_touched_n = 0usize;
    if let Some(mb) = &merge_base {
        let touched_head: BTreeSet<String> = git_lines(clone, &["diff", "--name-only", "--no-renames", mb, &head.commit]).unwrap_or_default().into_iter().collect();
        head_touched_n = touched_head.len();
        let head_dir = gitref::cache_dir(root, &repo.name, &head.commit);
        for l in git_lines(clone, &["diff", "--name-status", "--no-renames", mb, &base.commit]).unwrap_or_default() {
            let Some((st, path)) = l.split_once('\t') else { continue };
            let in_head = touched_head.contains(path);
            match st {
                "D" if head_dir.join(path).is_file() => deleted.push(json!({ "path": path, "head_modified": in_head })),
                "M" if in_head => rewritten.push(json!({ "path": path, "head_modified": true })),
                _ => {}
            }
        }
    }
    deleted.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    rewritten.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));

    let head_only = added.len() + changed.len();
    let already = added.iter().chain(changed.iter()).filter(|r| r.already_on_base.is_some()).count();
    let exact = added.iter().chain(changed.iter()).filter(|r| r.already_on_base.as_ref().is_some_and(|m| m.via == "body_hash")).count();
    let share = (head_only > 0).then(|| ((already as f64 / head_only as f64) * 10_000.0).round() / 10_000.0);
    let totals = (added.len(), removed.len(), changed.len(), deleted.len(), rewritten.len(), touched.len());
    let summary = json!({
        "base_symbols": base.syms.len(), "head_symbols": head.syms.len(),
        "added": totals.0, "removed": totals.1, "changed": totals.2, "unchanged": unchanged,
        "head_only": head_only, "head_only_already_on_base": already, "head_only_already_on_base_exact": exact,
        "share_already_on_base": share,
        "features_touched": totals.5,
        "files_base_deleted": totals.3, "files_base_rewritten": totals.4, "files_head_touched": head_touched_n,
    });
    let mut feats: Vec<(String, usize)> = touched.into_iter().collect();
    feats.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let cap = o.max_nodes;
    let mut limitations = vec![];
    let mut cut = |name: &str, n: usize| {
        if n > cap {
            limitations.push(json!({ "code": "truncated", "detail": format!("{name}: {n} rows, the first {cap} listed (sorted by path, line); raise --max-nodes, up to 2000") }));
        }
    };
    cut("added", added.len());
    cut("removed", removed.len());
    cut("changed", changed.len());
    cut("features_touched", feats.len());
    cut("base_files.deleted", deleted.len());
    cut("base_files.rewritten", rewritten.len());
    for (n, what) in [
        (base.unreadable + head.unreadable, "symbols whose source could not be read from the cached tree were left out"),
        (base.duplicates + head.duplicates, "symbols share a kind and qualified name with another; the first by path and line is kept"),
    ] {
        if n > 0 {
            limitations.push(json!({ "code": "partial", "detail": format!("{n} {what}") }));
        }
    }
    for v in [&mut added, &mut removed, &mut changed] {
        sort_rows(v);
        v.truncate(cap);
    }
    deleted.truncate(cap);
    rewritten.truncate(cap);
    feats.truncate(cap);
    let side = |s: &Side, r: &str| json!({ "ref": r, "commit": s.commit, "run": s.run });
    Ok(json!({
        "repo": repo_name,
        "base": side(&base, o.base), "head": side(&head, o.head),
        "merge_base": merge_base,
        "summary": summary,
        "added": added, "removed": removed, "changed": changed,
        "features_touched": feats.into_iter().map(|(f, n)| json!({ "feature": f, "head_only_symbols": n })).collect::<Vec<_>>(),
        "base_files": { "deleted": deleted, "rewritten": rewritten },
        "feature_pool": FEATURE_POOL,
        "limitation_items": limitations,
    }))
}
