//! CG-DELTA: what a branch introduced since it left its base (merge base -> head), and whether the base has it now.
//! Two runs built with `build --ref` are compared symbol by symbol; the engine reports facts and writes no verdict (restack, take
//! parts and retire are a reader's reading of the fields).
//!
//! Identity is `(kind, qualified_name)`. A symbol's body is its source lines normalised to a token stream (whitespace ignored, the
//! symbol's own name replaced by a placeholder), hashed with sha256 (already a dependency). From that:
//! - the branch's own work is what differs between the merge base and head: `added`, `changed`, `removed`. Base's own growth
//!   since the merge base is not the branch's (`--whole-tree` adds the old everything-against-everything comparison).
//! - each added or changed symbol is looked up on the base: the very same symbol with the same body (`same_symbol`), the same
//!   body hash under another name or path is
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
    /// add the whole-tree comparison (everything head holds against everything base holds)
    pub whole_tree: bool,
    /// build a ref that was never built (the missing refs, the merge base included)
    pub build_missing: bool,
    /// also rebuild only the refs that moved since they were built, then answer
    pub refresh: bool,
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

/// How a ref without a run at its current commit is treated.
#[derive(Clone, Copy)]
struct Mode {
    /// build a ref that was never built
    build_missing: bool,
    /// also rebuild a ref whose name has runs at older commits (it moved)
    refresh: bool,
}

fn stale_refusal(repo: &str, label: &str, commit: &str, moved: bool) -> AppError {
    let hint = if moved {
        format!("{label} moved since it was built: run `comparegraph build {repo} --ref {label}`, or pass --refresh to rebuild only what moved")
    } else {
        format!("run `comparegraph build {repo} --ref {label}` first, or pass --build-missing (--refresh also rebuilds refs that moved)")
    };
    AppError::structured(
        "not_found",
        format!("{repo} has no finished run at {label} ({})", &commit[..12.min(commit.len())]),
        Some(hint),
        Some(json!({ "ref": label, "commit": commit, "repo": repo, "stale": if moved { json!("moved") } else { Value::Null } })),
    )
}

/// The side of one ref: its run at the commit the ref names now (built here when the mode allows), and its symbols.
fn load_side(store: &Store, root: &Path, repo: &crate::store::RepoRow, label: &str, commit: &str, mode: Mode, rebuilt: &mut Vec<Value>) -> Result<Side> {
    let clone = Path::new(&repo.path);
    let rid = repo.id.expect("stored repo has id");
    let mut run = store.done_run_at_commit(rid, commit)?;
    if run.is_none() {
        let moved = store.ref_runs(rid)?.iter().any(|(_, r, _)| r == label);
        if !(mode.refresh || (mode.build_missing && !moved)) {
            return Err(stale_refusal(&repo.name, label, commit, moved));
        }
        let opts = crate::providers::BuildOptions { git_ref: Some(label.to_string()), ..Default::default() };
        super::build::build_repo(root, &repo.name, "reference", &opts)?;
        rebuilt.push(json!({ "ref": label, "commit": commit }));
        run = store.done_run_at_commit(rid, commit)?;
    }
    let run = run.ok_or_else(|| stale_refusal(&repo.name, label, commit, false))?;
    let run_id = run.id.expect("stored run has id").0;
    let dir = gitref::materialise(clone, root, &repo.name, commit)?;
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
    let mut side = Side { run: run_id, commit: commit.to_string(), syms: BTreeMap::new(), files: HashMap::new(), unreadable: 0, duplicates: 0 };
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
    /// base changed (or, for an added symbol, holds a different version of) the same symbol since the merge base
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    base_also_changed: bool,
    /// the symbol the branch changed or removed is gone from base (under its kind and qualified name)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    base_removed: bool,
    /// a changed symbol base no longer holds under its name: where base keeps its merge-base body (same body hash), i.e. base
    /// moved or renamed it without the branch's change
    #[serde(skip_serializing_if = "Vec::is_empty")]
    base_moved_to: Vec<Match>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    features: Vec<String>,
    /// a row of `removed` (the branch deleted the symbol): only there does base removing it too resolve the row
    #[serde(skip)]
    is_removal: bool,
}

impl Row {
    fn of(s: &Sym) -> Row {
        Row {
            qualified_name: s.qn.clone(),
            kind: s.kind.clone(),
            path: s.path.clone(),
            line_start: s.ls,
            line_end: s.le,
            body_hash: s.hash.clone(),
            base_body_hash: None,
            already_on_base: None,
            base_also_changed: false,
            base_removed: false,
            base_moved_to: vec![],
            features: vec![],
            is_removal: false,
        }
    }
    /// nothing left for the branch to carry: base holds the branch's version (added, changed), or the branch removed the symbol
    /// and base removed it too. A changed symbol base removed or moved is NOT resolved: the branch's change is not on base.
    fn resolved(&self) -> bool {
        self.already_on_base.is_some() || (self.is_removal && self.base_removed)
    }
}
/// Not-on-base first, then by path and line.
fn sort_rows(v: &mut [Row]) {
    v.sort_by(|a, b| (a.resolved(), &a.path, a.line_start, &a.qualified_name, &a.kind).cmp(&(b.resolved(), &b.path, b.line_start, &b.qualified_name, &b.kind)));
}

fn git_lines(clone: &Path, args: &[&str]) -> Option<Vec<String>> {
    let out = std::process::Command::new("git").arg("-C").arg(clone).args(["-c", "core.quotepath=false"]).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).lines().map(String::from).collect())
}

/// The commit a ref names now. A ref that no longer resolves but has runs is `stale: gone`.
fn resolve_live(store: &Store, repo: &crate::store::RepoRow, label: &str) -> Result<String> {
    match gitref::resolve(Path::new(&repo.path), label) {
        Ok(c) => Ok(c),
        Err(AppError::Structured(mut e)) => {
            let has_runs = store.ref_runs(repo.id.expect("stored repo has id"))?.iter().any(|(_, r, _)| r == label);
            if has_runs {
                if let Some(Value::Object(d)) = e.data.as_mut() {
                    d.insert("stale".into(), json!("gone"));
                }
                e.message = format!("{} (runs built from it remain: stale, gone; `gc --refs` removes them)", e.message);
            }
            Err(AppError::Structured(e))
        }
        Err(e) => Err(e),
    }
}

fn record_last(root: &Path, repo: &str, head_commit: &str, v: Value) {
    if let Ok(store) = Store::open_guarded(root) {
        let _ = store.with_write_lock(|s| -> std::result::Result<(), crate::store::StoreError> { s.meta_set(&format!("delta:{repo}:{head_commit}"), &v.to_string()) });
    }
}

/// The last delta computed for a head commit (full sha or a prefix of it): `{base_ref, base_commit, head_ref, head_commit, merge_base, summary}`.
pub fn last_delta(store: &Store, repo: &str, commit: &str) -> Option<Value> {
    let rows = store.meta_with_prefix(&format!("delta:{repo}:{commit}")).ok()?;
    rows.last().and_then(|(_, v)| serde_json::from_str(v).ok())
}

fn limit_note(limitations: &mut Vec<Value>, name: &str, n: usize, cap: usize) {
    if n > cap {
        limitations.push(json!({ "code": "truncated", "detail": format!("{name}: {n} rows, the first {cap} listed (not-on-base first, then path and line); raise --max-nodes, up to 2000") }));
    }
}

pub fn delta(root: &Path, repo_name: &str, o: &DeltaOpts) -> Result<Value> {
    let store = Store::open_existing(root)?;
    let repo = store.repo_by_name(repo_name)?.ok_or_else(|| AppError::NotFound(format!("no repo named {repo_name}")))?;
    super::archive::refuse_if_archived(&store, repo_name)?;
    if !(1..=2000).contains(&o.max_nodes) {
        return Err(AppError::InvalidArguments("--max-nodes must be 1..=2000".into()));
    }
    let clone = Path::new(&repo.path);
    let (base_c, head_c) = (resolve_live(&store, &repo, o.base)?, resolve_live(&store, &repo, o.head)?);
    let mb_c = git_lines(clone, &["merge-base", &base_c, &head_c])
        .and_then(|v| v.into_iter().next())
        .ok_or_else(|| AppError::InvalidArguments(format!("{} and {} have no merge base (unrelated histories)", o.base, o.head)))?;
    let mode = Mode { build_missing: o.build_missing, refresh: o.refresh };
    let mut rebuilt = vec![];
    let base = load_side(&store, root, &repo, o.base, &base_c, mode, &mut rebuilt)?;
    let head = load_side(&store, root, &repo, o.head, &head_c, mode, &mut rebuilt)?;
    let mb = load_side(&store, root, &repo, &mb_c, &mb_c, mode, &mut rebuilt)?;

    let mut by_hash: HashMap<&str, Vec<&Sym>> = HashMap::new();
    let mut by_name: HashMap<(&str, &str), Vec<&Sym>> = HashMap::new();
    for s in base.syms.values() {
        by_hash.entry(s.hash.as_str()).or_default().push(s);
        by_name.entry((s.name.as_str(), s.kind.as_str())).or_default().push(s);
    }
    let mk = |v: &[&Sym]| -> Vec<Match> {
        let mut m: Vec<Match> = v.iter().map(|b| Match { qualified_name: b.qn.clone(), path: b.path.clone() }).collect();
        m.sort_by(|a, b| (&a.path, &a.qualified_name).cmp(&(&b.path, &b.qualified_name)));
        m.truncate(MAX_MATCHES);
        m
    };
    // is this head symbol already on base: the very same symbol, the same body elsewhere, or a near copy of the same name.
    // `prev` is the merge-base version of a changed symbol: a near copy then counts only when it is strictly nearer to head than
    // to that version (a moved copy of the old body is near head too, but nearer the merge base: base did not take the change).
    let on_base = |s: &Sym, prev: Option<&Sym>| -> Option<OnBase> {
        if let Some(b) = base.syms.get(&(s.kind.clone(), s.qn.clone())) {
            if b.hash == s.hash {
                return Some(OnBase { confidence: 1.0, via: "same_symbol", matches: mk(&[b]) });
            }
        }
        let exact: Vec<&Sym> = by_hash.get(s.hash.as_str()).map(|v| v.iter().copied().filter(|b| b.qn != s.qn || b.kind != s.kind).collect()).unwrap_or_default();
        if !exact.is_empty() {
            return Some(OnBase { confidence: 1.0, via: "body_hash", matches: mk(&exact) });
        }
        let cands: Vec<&Sym> = by_name.get(&(s.name.as_str(), s.kind.as_str())).map(|v| v.iter().copied().filter(|b| b.qn != s.qn).collect()).unwrap_or_default();
        if cands.is_empty() {
            return None;
        }
        let mine = bigrams(&tokens(&body_of(&head.files, s)?, &s.name));
        let old = match prev {
            Some(p) => Some(bigrams(&tokens(&body_of(&mb.files, p)?, &p.name))),
            None => None,
        };
        let mut best: Vec<(f64, &Sym)> = vec![];
        for c in cands {
            let theirs = bigrams(&tokens(&body_of(&base.files, c)?, &c.name));
            let sim = jaccard(&mine, &theirs);
            if sim >= SIMILARITY_FLOOR && old.as_ref().is_none_or(|o| sim > jaccard(o, &theirs)) {
                best.push((sim, c));
            }
        }
        best.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then((&a.1.path, &a.1.qn).cmp(&(&b.1.path, &b.1.qn))));
        let top = best.first()?.0;
        let confidence = ((top * 100.0).floor() / 100.0).min(0.99);
        let at_top: Vec<&Sym> = best.iter().filter(|b| b.0 == top).map(|b| b.1).collect();
        Some(OnBase { confidence, via: "name_and_tokens", matches: mk(&at_top) })
    };

    // the branch's own work: merge base -> head
    let (mut added, mut changed, mut removed) = (vec![], vec![], vec![]);
    for (k, h) in &head.syms {
        let in_base = base.syms.get(k);
        match mb.syms.get(k) {
            None => {
                let mut r = Row::of(h);
                r.already_on_base = on_base(h, None);
                r.base_also_changed = r.already_on_base.is_none() && in_base.is_some();
                added.push(r);
            }
            Some(m) if m.hash != h.hash => {
                let mut r = Row::of(h);
                r.base_body_hash = Some(m.hash.clone());
                r.already_on_base = on_base(h, Some(m));
                match in_base {
                    None => {
                        r.base_removed = true;
                        let moved: Vec<&Sym> = by_hash.get(m.hash.as_str()).map(|v| v.to_vec()).unwrap_or_default();
                        r.base_moved_to = mk(&moved);
                    }
                    Some(b) => r.base_also_changed = b.hash != m.hash,
                }
                changed.push(r);
            }
            Some(_) => {}
        }
    }
    for (k, m) in &mb.syms {
        if !head.syms.contains_key(k) {
            let mut r = Row::of(m);
            r.is_removal = true;
            match base.syms.get(k) {
                None => r.base_removed = true,
                Some(b) => r.base_also_changed = b.hash != m.hash,
            }
            removed.push(r);
        }
    }

    // families and features of the symbols the branch added or changed (`map`'s ranking, deeper)
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

    // the files base deleted or rewrote since the merge base, among the ones the branch touched
    let (mut deleted, mut rewritten) = (vec![], vec![]);
    let touched_head: BTreeSet<String> = git_lines(clone, &["diff", "--name-only", "--no-renames", &mb_c, &head_c]).unwrap_or_default().into_iter().collect();
    let head_dir = gitref::cache_dir(root, &repo.name, &head_c);
    for l in git_lines(clone, &["diff", "--name-status", "--no-renames", &mb_c, &base_c]).unwrap_or_default() {
        let Some((st, path)) = l.split_once('\t') else { continue };
        if !touched_head.contains(path) {
            continue;
        }
        match st {
            "D" if head_dir.join(path).is_file() => deleted.push(json!({ "path": path, "head_modified": true })),
            "M" => rewritten.push(json!({ "path": path, "head_modified": true })),
            _ => {}
        }
    }
    deleted.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    rewritten.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));

    let branch_symbols = added.len() + changed.len() + removed.len();
    let on_base_n = added.iter().chain(changed.iter()).filter(|r| r.already_on_base.is_some()).count();
    let exact_n = added.iter().chain(changed.iter()).filter(|r| r.already_on_base.as_ref().is_some_and(|m| m.via != "name_and_tokens")).count();
    let base_removed_n = changed.iter().chain(removed.iter()).filter(|r| r.base_removed).count();
    // a change base does not hold, to a symbol base changed otherwise, removed, or moved without the change
    let stale_conflicts = changed.iter().filter(|r| (r.base_also_changed || r.base_removed) && r.already_on_base.is_none()).count();
    let resolved = added.iter().chain(changed.iter()).chain(removed.iter()).filter(|r| r.resolved()).count();
    let share = (branch_symbols > 0).then(|| ((resolved as f64 / branch_symbols as f64) * 10_000.0).round() / 10_000.0);
    let summary = json!({
        "branch_symbols": branch_symbols,
        "added": added.len(), "changed": changed.len(), "removed": removed.len(),
        "already_on_base": on_base_n, "already_on_base_exact": exact_n, "base_removed": base_removed_n,
        "share_already_on_base": share,
        "stale_conflicts": stale_conflicts,
        "features_touched": touched.len(),
        "files_base_deleted": deleted.len(), "files_base_rewritten": rewritten.len(), "files_branch_touched": touched_head.len(),
    });
    let mut feats: Vec<(String, usize)> = touched.into_iter().collect();
    feats.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    // the whole-tree comparison, on request: everything head holds against everything base holds
    let whole = if o.whole_tree {
        let (mut wa, mut wc, mut wr) = (vec![], vec![], vec![]);
        let mut unchanged = 0usize;
        for (k, h) in &head.syms {
            match base.syms.get(k) {
                None => {
                    let mut r = Row::of(h);
                    r.already_on_base = on_base(h, None);
                    wa.push(r);
                }
                Some(b) if b.hash != h.hash => {
                    let mut r = Row::of(h);
                    r.base_body_hash = Some(b.hash.clone());
                    r.already_on_base = on_base(h, None);
                    wc.push(r);
                }
                Some(_) => unchanged += 1,
            }
        }
        for (k, b) in &base.syms {
            if !head.syms.contains_key(k) {
                wr.push(Row::of(b));
            }
        }
        let head_only = wa.len() + wc.len();
        let already = wa.iter().chain(wc.iter()).filter(|r| r.already_on_base.is_some()).count();
        let ws = json!({
            "base_symbols": base.syms.len(), "head_symbols": head.syms.len(),
            "added": wa.len(), "removed": wr.len(), "changed": wc.len(), "unchanged": unchanged,
            "head_only": head_only, "head_only_already_on_base": already,
            "share_already_on_base": (head_only > 0).then(|| ((already as f64 / head_only as f64) * 10_000.0).round() / 10_000.0),
        });
        Some((ws, wa, wr, wc))
    } else {
        None
    };

    let cap = o.max_nodes;
    let mut limitations = vec![];
    limit_note(&mut limitations, "branch.added", added.len(), cap);
    limit_note(&mut limitations, "branch.changed", changed.len(), cap);
    limit_note(&mut limitations, "branch.removed", removed.len(), cap);
    limit_note(&mut limitations, "features_touched", feats.len(), cap);
    limit_note(&mut limitations, "base_files.deleted", deleted.len(), cap);
    limit_note(&mut limitations, "base_files.rewritten", rewritten.len(), cap);
    if let Some((_, wa, wr, wc)) = &whole {
        limit_note(&mut limitations, "whole_tree.added", wa.len(), cap);
        limit_note(&mut limitations, "whole_tree.removed", wr.len(), cap);
        limit_note(&mut limitations, "whole_tree.changed", wc.len(), cap);
    }
    for (n, what) in [
        (base.unreadable + head.unreadable + mb.unreadable, "symbols whose source could not be read from the cached tree were left out"),
        (base.duplicates + head.duplicates + mb.duplicates, "symbols share a kind and qualified name with another; the first by path and line is kept"),
    ] {
        if n > 0 {
            limitations.push(json!({ "code": "partial", "detail": format!("{n} {what}") }));
        }
    }
    for v in [&mut added, &mut changed, &mut removed] {
        sort_rows(v);
        v.truncate(cap);
    }
    deleted.truncate(cap);
    rewritten.truncate(cap);
    feats.truncate(cap);
    let side = |s: &Side, r: &str| json!({ "ref": r, "commit": s.commit, "run": s.run });
    let mut out = json!({
        "repo": repo_name,
        "base": side(&base, o.base), "head": side(&head, o.head), "merge_base": side(&mb, &mb_c),
        "summary": summary,
        "branch": { "added": added, "changed": changed, "removed": removed },
        "features_touched": feats.into_iter().map(|(f, n)| json!({ "feature": f, "branch_symbols": n })).collect::<Vec<_>>(),
        "base_files": { "deleted": deleted, "rewritten": rewritten },
        "rebuilt": rebuilt,
        "feature_pool": FEATURE_POOL,
        "limitation_items": limitations,
    });
    // the last delta computed for this head commit, kept by value for `decide --ref --summary-from-delta` (best effort: a read-only store skips it)
    record_last(root, repo_name, &head_c, json!({
        "base_ref": o.base, "base_commit": base_c, "head_ref": o.head, "head_commit": head_c, "merge_base": mb_c,
        "summary": out["summary"],
    }));
    if let Some((ws, mut wa, mut wr, mut wc)) = whole {
        for v in [&mut wa, &mut wr, &mut wc] {
            sort_rows(v);
            v.truncate(cap);
        }
        out["whole_tree"] = json!({ "summary": ws, "added": wa, "removed": wr, "changed": wc });
    }
    Ok(out)
}
