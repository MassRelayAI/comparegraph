//! CG-DELTA: what a branch introduced since it left its base (merge base -> head), and whether the base has it now.
//! Two runs built with `build --ref` are compared symbol by symbol; the engine reports facts and writes no verdict (restack, take
//! parts and retire are a reader's reading of the fields).
//!
//! Identity is `(kind, qualified_name)`. A symbol's body is its source lines normalised to a token stream (whitespace ignored, the
//! symbol's own name replaced by a placeholder), hashed with sha256 (already a dependency). From that:
//! - the branch's own work is what differs between the merge base and head: `added`, `changed`, `removed`. Base's own growth
//!   since the merge base is not the branch's (`--whole-tree` adds the old everything-against-everything comparison).
//! - whether base already holds each of those rows is ONE rule for every row type, `Judge::rule` (docs/decisions.md, "Plan 14
//!   review round 2"): a branch change is on base only if base holds the branch's version of it, and that version was not
//!   already on the merge base. A match is the very same symbol (`same_symbol`), the same body hash under another name or path
//!   (`body_hash`, confidence 1.0), or a base symbol of the same name and kind whose token-bigram Jaccard is >= 0.9 and nearer
//!   the branch than the merge base (`name_and_tokens`, floored to two decimals, never above 0.99).
//! - the families and features each head-only symbol ranks under come from `map`'s own ranking (`map::symbol_features`).
use super::{gitref, AppError, Result};
use crate::store::Store;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

/// How deep into each feature's ranking a symbol still counts as mapped to it.
pub const FEATURE_POOL: usize = 500;
/// The similarity floor of the `name_and_tokens` rule.
pub const SIMILARITY_FLOOR: f64 = 0.9;
/// A body with fewer tokens than this after the end of its signature (`body_tokens`: `def f(x): pass`, `return 1`, a docstring
/// and `pass`) is too little evidence for a `body_hash` or `name_and_tokens` match: such a row answers `already_on_base: null`
/// with the note `trivial_body`. `r2_the_trivial_floor_is_exactly_eight_tokens_after_the_signature` pins the value.
pub const MIN_BODY_TOKENS: usize = 8;
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
    /// fewer than MIN_BODY_TOKENS tokens after the signature line
    trivial: bool,
}

struct Side {
    run: i64,
    commit: String,
    syms: BTreeMap<(String, String), Sym>,
    files: HashMap<String, Vec<String>>,
    unreadable: usize,
    duplicates: usize,
    /// the file limits the run was read with (`build::run_limits`); None for a run recorded before they were
    limits: Option<Value>,
    /// what the run's own read left out (truncation, skipped files, the size limit, tree hazards): carried into the delta's
    limitations: Vec<crate::domain::Limitation>,
}

/// The run limitation codes a delta carries from each side: each says the side's graph misses files.
const CARRIED: &[&str] = &["truncated_files", "skipped_oversize", "skipped_file", "case_collision_skipped", "non_utf8_path", "submodule_skipped"];

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

/// The tokens of a symbol's body after the end of its signature, decorators and a leading docstring excluded. The provider
/// records no body start, so the signature ends at the first depth-0 `:` (Python) or `{` (`=>` for an arrow function) after the
/// symbol's name, brackets and string literals skipped; else after the depth-0 `)` that closes its parameters; else after the
/// first line. Decorator lines (`@...`, with their bracketed continuation) above the name are skipped first.
fn body_tokens(body: &str, name: &str, python: bool) -> usize {
    let chars: Vec<char> = body.chars().collect();
    let n = chars.len();
    // skip decorator lines: a line starting with `@`, and the lines its open brackets carry on to
    let mut i = 0usize;
    loop {
        let mut j = i;
        while j < n && chars[j].is_whitespace() {
            j += 1;
        }
        if j >= n || chars[j] != '@' {
            break;
        }
        let mut depth = 0i32;
        while j < n {
            match chars[j] {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                '\n' if depth <= 0 => break,
                _ => {}
            }
            j += 1;
        }
        i = j;
    }
    // the symbol's name as a whole word
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let name_chars: Vec<char> = name.chars().collect();
    let mut start = i;
    if !name_chars.is_empty() {
        let mut k = i;
        while k + name_chars.len() <= n {
            let whole = (k == 0 || !is_word(chars[k - 1])) && chars.get(k + name_chars.len()).is_none_or(|c| !is_word(*c));
            if chars[k..k + name_chars.len()] == name_chars[..] && whole {
                start = k + name_chars.len();
                break;
            }
            k += 1;
        }
    }
    let (mut depth, mut k) = (0i32, start);
    let (mut body_at, mut closed_at): (Option<usize>, Option<usize>) = (None, None);
    while k < n {
        let c = chars[k];
        match c {
            '"' | '\'' | '`' => {
                k += 1;
                while k < n && chars[k] != c {
                    if chars[k] == '\\' {
                        k += 1;
                    }
                    k += 1;
                }
            }
            '(' | '[' => depth += 1,
            ')' | ']' => {
                depth -= 1;
                if depth == 0 && c == ')' && closed_at.is_none() {
                    closed_at = Some(k + 1);
                }
            }
            ':' if depth == 0 && python => {
                body_at = Some(k + 1);
                break;
            }
            '{' if depth == 0 && !python => {
                body_at = Some(k + 1);
                break;
            }
            '=' if depth == 0 && !python && chars.get(k + 1) == Some(&'>') => {
                body_at = Some(k + 2);
                break;
            }
            _ => {}
        }
        k += 1;
    }
    let from = body_at.or(closed_at).unwrap_or_else(|| chars.iter().position(|c| *c == '\n').map_or(n, |p| p + 1)).min(n);
    let rest: String = chars[from..].iter().collect();
    let rest = if python { strip_docstring(&rest) } else { &rest };
    tokens(rest, name).len()
}

/// `text` without a leading Python string literal (a docstring: triple-quoted or one-quote, with an `r`/`u`/`b`/`f` prefix).
fn strip_docstring(text: &str) -> &str {
    let t = text.trim_start();
    let body = t.trim_start_matches(|c: char| "rRuUbBfF".contains(c));
    if body.len() + 2 < t.len() {
        return text; // more than a two-letter prefix: a name, not a string prefix
    }
    for q in ["\"\"\"", "'''", "\"", "'"] {
        if let Some(after) = body.strip_prefix(q) {
            return match after.find(q) {
                Some(end) => &after[end + q.len()..],
                None => text,
            };
        }
    }
    text
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

type NodeRow = (String, String, String, String, i64, i64, String);

/// The options every side of one delta is read with: the running reference provider's version and the repo's `add` subset
/// (sorted), exactly what `build --ref` uses when given none. A run read otherwise is never mixed into the delta.
struct Want {
    version: String,
    include: Vec<String>,
    exclude: Vec<String>,
}

/// The first of `runs` (newest first) read with the wanted options, else the newest one's options for the refusal.
fn pick_run(store: &Store, runs: Vec<crate::domain::Run>, w: &Want) -> Result<(Option<crate::domain::Run>, Option<Value>)> {
    let mut other: Option<Value> = None;
    for r in runs {
        let id = r.id.expect("stored run has id");
        let (mut inc, mut exc) = store.run_subset(id)?;
        for v in [&mut inc, &mut exc] {
            v.sort();
            v.dedup();
        }
        if r.provider_version == w.version && inc == w.include && exc == w.exclude {
            return Ok((Some(r), None));
        }
        other.get_or_insert_with(|| json!({ "run": id.0, "provider_version": r.provider_version, "include": inc, "exclude": exc }));
    }
    Ok((None, other))
}

fn options_refusal(repo: &str, label: &str, commit: &str, w: &Want, found: Value) -> AppError {
    let mut diff = vec![];
    if found["provider_version"] != json!(w.version) {
        diff.push(format!("provider_version {} (wanted {})", found["provider_version"], w.version));
    }
    if found["include"] != json!(w.include) {
        diff.push(format!("include {} (wanted {})", found["include"], json!(w.include)));
    }
    if found["exclude"] != json!(w.exclude) {
        diff.push(format!("exclude {} (wanted {})", found["exclude"], json!(w.exclude)));
    }
    AppError::structured(
        "not_found",
        format!("{repo}'s run at {label} ({}) was read with other options: {}; a delta never mixes runs of different options", &commit[..12.min(commit.len())], diff.join(", ")),
        Some(format!("pass --build-missing (or --refresh) to build {label} with the delta's options (the repo's `add` subset and this engine version), or `comparegraph build {repo} --ref {label}`")),
        Some(json!({ "ref": label, "commit": commit, "repo": repo, "found": found, "wanted": { "provider_version": w.version, "include": w.include, "exclude": w.exclude } })),
    )
}

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
#[allow(clippy::too_many_arguments)]
fn load_side(store: &Store, root: &Path, repo: &crate::store::RepoRow, label: &str, commit: &str, want: &Want, mode: Mode, rebuilt: &mut Vec<Value>) -> Result<Side> {
    let clone = Path::new(&repo.path);
    let rid = repo.id.expect("stored repo has id");
    let (mut run, mut other) = pick_run(store, store.done_runs_at_commit(rid, commit)?, want)?;
    if run.is_none() {
        // a run at this commit read with other options is a missing run of the wanted options, not a moved ref
        let moved = other.is_none() && store.ref_runs(rid)?.iter().any(|(_, r, _)| r == label);
        if !(mode.refresh || (mode.build_missing && !moved)) {
            return Err(match other {
                Some(found) => options_refusal(&repo.name, label, commit, want, found),
                None => stale_refusal(&repo.name, label, commit, moved),
            });
        }
        let opts = crate::providers::BuildOptions { git_ref: Some(label.to_string()), ..Default::default() };
        super::build::build_repo(root, &repo.name, "reference", &opts)?;
        rebuilt.push(json!({ "ref": label, "commit": commit }));
        (run, other) = pick_run(store, store.done_runs_at_commit(rid, commit)?, want)?;
    }
    let run = match (run, other) {
        (Some(r), _) => r,
        (None, Some(found)) => return Err(options_refusal(&repo.name, label, commit, want, found)),
        (None, None) => return Err(stale_refusal(&repo.name, label, commit, false)),
    };
    let run_id = run.id.expect("stored run has id").0;
    let limits: Option<Value> = store.meta_get(&super::build::run_limits_key(run_id))?.and_then(|t| serde_json::from_str(&t).ok());
    let max_bytes = limits.as_ref().and_then(|l| l["max_file_bytes"].as_u64()).unwrap_or(crate::providers::BuildOptions::default().max_file_bytes);
    let dir = gitref::materialise(clone, root, &repo.name, commit, max_bytes)?.dir;
    let mut rows: Vec<NodeRow> = vec![];
    {
        let mut st = store.conn().prepare(
            "SELECT kind, name, qualified_name, path, line_start, line_end, language FROM node WHERE run_id=?1 AND kind IN ('function','method','class','type','route') \
             ORDER BY qualified_name, kind, path, line_start",
        )?;
        for r in st.query_map([run_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))? {
            rows.push(r?);
        }
    }
    let limitations = run.limitations.iter().filter(|l| CARRIED.contains(&l.code.as_str())).cloned().collect();
    let mut side = Side { run: run_id, commit: commit.to_string(), syms: BTreeMap::new(), files: HashMap::new(), unreadable: 0, duplicates: 0, limits, limitations };
    for (kind, name, qn, path, ls, le, language) in rows {
        if !side.files.contains_key(&path) {
            let text = std::fs::read(dir.join(&path)).ok().and_then(|b| String::from_utf8(b).ok());
            side.files.insert(path.clone(), text.map(|t| t.lines().map(String::from).collect()).unwrap_or_default());
        }
        let mut s = Sym { kind: kind.clone(), name, qn: qn.clone(), path, ls: ls as u32, le: le as u32, hash: String::new(), trivial: false };
        let Some(body) = body_of(&side.files, &s) else {
            side.unreadable += 1;
            continue;
        };
        let mut h = Sha256::new();
        h.update(tokens(&body, &s.name).join("\u{0}").as_bytes());
        s.hash = hex::encode(h.finalize());
        s.trivial = body_tokens(&body, &s.name, language == "python") < MIN_BODY_TOKENS;
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
    /// absent: not on base; null: not judged (`note` says why); else the match
    #[serde(skip_serializing_if = "Option::is_none")]
    already_on_base: Option<Option<OnBase>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<&'static str>,
    /// base changed (or, for an added symbol, holds a different version of) the same symbol since the merge base
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    base_also_changed: bool,
    /// the symbol the branch changed or removed is gone from base (under its kind and qualified name)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    base_removed: bool,
    /// a changed or removed symbol base no longer holds under its name: where base keeps its merge-base body (`Judge::copies_of`),
    /// i.e. base moved or renamed it without the branch's change (or deletion)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    base_moved_to: Vec<Match>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    features: Vec<String>,
    /// a row of `removed` that the rule found on base: base holds the symbol nowhere (no copy of its merge-base body)
    #[serde(skip)]
    removal_resolved: bool,
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
            note: None,
            base_also_changed: false,
            base_removed: false,
            base_moved_to: vec![],
            features: vec![],
            removal_resolved: false,
        }
    }
    /// nothing left for the branch to carry, by `Judge::rule`: base holds the branch's version (added, changed), or holds the
    /// removed symbol nowhere (removed).
    fn resolved(&self) -> bool {
        self.on_base().is_some() || self.removal_resolved
    }
    /// changed or removed, not on base, and base changed, removed or moved the same symbol its own way since the merge base
    fn stale_conflict(&self) -> bool {
        !self.resolved() && (self.base_also_changed || self.base_removed)
    }
    fn on_base(&self) -> Option<&OnBase> {
        self.already_on_base.as_ref().and_then(|m| m.as_ref())
    }
    /// set the base lookup of a head symbol: a trivial body that is not the very same symbol is not judged
    fn set_on_base(&mut self, s: &Sym, m: Option<OnBase>) {
        match m {
            Some(m) => self.already_on_base = Some(Some(m)),
            None if s.trivial => {
                self.already_on_base = Some(None);
                self.note = Some("trivial_body");
            }
            None => {}
        }
    }
}
/// The token bigrams of one side's symbol bodies, computed once per symbol and kept for the whole delta (a body is compared with
/// several candidates, and a candidate with several branch symbols).
type GramSet = Option<Rc<HashSet<String>>>;
struct Grams<'a> {
    side: &'a Side,
    cache: RefCell<HashMap<(String, String), GramSet>>,
    computed: std::cell::Cell<usize>,
}
impl<'a> Grams<'a> {
    fn new(side: &'a Side) -> Self {
        Grams { side, cache: RefCell::new(HashMap::new()), computed: std::cell::Cell::new(0) }
    }
    /// `s` must be a symbol of this side; None when its source cannot be read
    fn of(&self, s: &Sym) -> GramSet {
        let k = (s.kind.clone(), s.qn.clone());
        if let Some(v) = self.cache.borrow().get(&k) {
            return v.clone();
        }
        self.computed.set(self.computed.get() + 1);
        let v = body_of(&self.side.files, s).map(|b| Rc::new(bigrams(&tokens(&b, &s.name))));
        self.cache.borrow_mut().insert(k, v.clone());
        v
    }
}

fn key(s: &Sym) -> (String, String) {
    (s.kind.clone(), s.qn.clone())
}
fn matches_of(v: &[&Sym]) -> Vec<Match> {
    let mut m: Vec<Match> = v.iter().map(|b| Match { qualified_name: b.qn.clone(), path: b.path.clone() }).collect();
    m.sort_by(|a, b| (&a.path, &a.qualified_name).cmp(&(&b.path, &b.qualified_name)));
    m.dedup_by(|a, b| a.path == b.path && a.qualified_name == b.qualified_name);
    m.truncate(MAX_MATCHES);
    m
}

/// One branch change, as the rule reads it.
enum Change<'a> {
    /// an added or changed head symbol `h`; `m` is its merge-base body (a changed symbol's own, an added symbol's partner, or none)
    Head { h: &'a Sym, m: Option<&'a Sym> },
    /// a merge-base symbol the branch deleted
    Removed { m: &'a Sym },
}
struct Verdict<'a> {
    on_base: Option<OnBase>,
    /// Removed: base holds the symbol nowhere
    removal_resolved: bool,
    /// Removed: where base keeps the merge-base body under another name or path
    copies: Vec<&'a Sym>,
}

/// The three sides of a delta and their lookups. `fork` is the merge base; the whole-tree comparison has none.
struct Judge<'a> {
    base: &'a Side,
    head: &'a Side,
    fork: Option<&'a Side>,
    by_hash: HashMap<&'a str, Vec<&'a Sym>>,
    by_name: HashMap<(&'a str, &'a str), Vec<&'a Sym>>,
    fork_by_name: HashMap<(&'a str, &'a str), Vec<&'a Sym>>,
    head_by_name: HashMap<(&'a str, &'a str), Vec<&'a Sym>>,
    g_base: Grams<'a>,
    g_head: Grams<'a>,
    g_fork: Option<Grams<'a>>,
}

impl<'a> Judge<'a> {
    fn new(base: &'a Side, head: &'a Side, fork: Option<&'a Side>) -> Judge<'a> {
        let index = |s: &'a Side| {
            let mut by_name: HashMap<(&'a str, &'a str), Vec<&'a Sym>> = HashMap::new();
            for x in s.syms.values() {
                by_name.entry((x.name.as_str(), x.kind.as_str())).or_default().push(x);
            }
            by_name
        };
        let mut by_hash: HashMap<&'a str, Vec<&'a Sym>> = HashMap::new();
        for s in base.syms.values() {
            by_hash.entry(s.hash.as_str()).or_default().push(s);
        }
        Judge {
            base,
            head,
            fork,
            by_hash,
            by_name: index(base),
            fork_by_name: fork.map(index).unwrap_or_default(),
            head_by_name: index(head),
            g_base: Grams::new(base),
            g_head: Grams::new(head),
            g_fork: fork.map(Grams::new),
        }
    }

    /// base symbol `b` is new or changed on base since the fork (with no fork, every base symbol is)
    fn fresh(&self, b: &Sym) -> bool {
        self.fork.is_none_or(|f| f.syms.get(&key(b)).is_none_or(|m| m.hash != b.hash))
    }

    fn sim_base(&self, b: &Sym, other: &HashSet<String>) -> Option<f64> {
        self.g_base.of(b).map(|g| jaccard(&g, other))
    }

    /// **THE RULE** (docs/decisions.md, "Plan 14 review round 2"), one function for every row type:
    ///
    /// A branch change is on base only if base holds the branch's version of it, and that version was not already on the
    /// merge base.
    /// - added / changed (head body H, merge-base body M; M is absent for a pure add): resolved iff some base symbol B (the same
    ///   qualified name, or another name or path) has H's normalised hash, or (H not trivial) has H's name and kind and
    ///   J(B,H) >= 0.9 and J(B,H) > J(B,M); AND B's (kind, qualified name) did not have that same hash on the merge base (B is new
    ///   or changed on base since the fork: old code never shows the branch's work). Matches are reported as `same_symbol`, then
    ///   `body_hash`, then `name_and_tokens`.
    /// - removed (merge-base body M): resolved iff base holds the symbol under its own (kind, qualified name) nowhere and keeps no
    ///   copy of M (`copies_of`) under another name or path. A copy fills `base_moved_to` and leaves the row unresolved.
    /// - with no fork (the whole-tree comparison) every base symbol counts as fresh and the same key is not a near candidate.
    fn rule(&self, change: Change<'a>) -> Verdict<'a> {
        let none = Verdict { on_base: None, removal_resolved: false, copies: vec![] };
        match change {
            Change::Head { h, m } => {
                if let Some(b) = self.base.syms.get(&key(h)) {
                    if b.hash == h.hash && self.fresh(b) {
                        return Verdict { on_base: Some(OnBase { confidence: 1.0, via: "same_symbol", matches: matches_of(&[b]) }), ..none };
                    }
                }
                if h.trivial {
                    return none;
                }
                let exact: Vec<&Sym> = self.by_hash.get(h.hash.as_str()).map(|v| v.iter().copied().filter(|b| key(b) != key(h) && self.fresh(b)).collect()).unwrap_or_default();
                if !exact.is_empty() {
                    return Verdict { on_base: Some(OnBase { confidence: 1.0, via: "body_hash", matches: matches_of(&exact) }), ..none };
                }
                let same_key_ok = self.fork.is_some();
                let cands: Vec<&Sym> = self
                    .by_name
                    .get(&(h.name.as_str(), h.kind.as_str()))
                    .map(|v| v.iter().copied().filter(|b| (same_key_ok || key(b) != key(h)) && self.fresh(b)).collect())
                    .unwrap_or_default();
                if cands.is_empty() {
                    return none;
                }
                let Some(mine) = self.g_head.of(h) else { return none };
                let old = match (m, &self.g_fork) {
                    (Some(m), Some(g)) => g.of(m),
                    _ => None,
                };
                let mut best: Vec<(f64, &Sym)> = vec![];
                for c in cands {
                    let Some(theirs) = self.g_base.of(c) else { continue };
                    let sim = jaccard(&mine, &theirs);
                    if sim >= SIMILARITY_FLOOR && old.as_ref().is_none_or(|o| sim > jaccard(o, &theirs)) {
                        best.push((sim, c));
                    }
                }
                best.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then((&a.1.path, &a.1.qn).cmp(&(&b.1.path, &b.1.qn))));
                let Some(top) = best.first().map(|b| b.0) else { return none };
                let confidence = ((top * 100.0).floor() / 100.0).min(0.99);
                let at_top: Vec<&Sym> = best.iter().filter(|b| b.0 == top).map(|b| b.1).collect();
                Verdict { on_base: Some(OnBase { confidence, via: "name_and_tokens", matches: matches_of(&at_top) }), ..none }
            }
            Change::Removed { m } => {
                if self.base.syms.contains_key(&key(m)) {
                    return none; // base keeps it in place (as it was, or changed its own way: a stale conflict)
                }
                let copies = self.copies_of(m);
                Verdict { on_base: None, removal_resolved: copies.is_empty(), copies }
            }
        }
    }

    /// Where base keeps merge-base body `m` under another (kind, qualified name): a fresh base symbol with M's hash, or of M's
    /// name and kind with J(B,M) >= 0.9 that is not nearer the branch's own new version of it (the head partner of a move) than M.
    /// A base symbol that already held its body on the merge base is a duplicate there, not a place base moved `m` to.
    fn copies_of(&self, m: &'a Sym) -> Vec<&'a Sym> {
        let mut out: Vec<&'a Sym> = self.by_hash.get(m.hash.as_str()).map(|v| v.iter().copied().filter(|b| key(b) != key(m) && self.fresh(b)).collect()).unwrap_or_default();
        let (Some(gf), false) = (&self.g_fork, m.trivial) else { return out };
        let Some(old) = gf.of(m) else { return out };
        let new = self.head_partner(m).and_then(|h| self.g_head.of(h));
        for b in self.by_name.get(&(m.name.as_str(), m.kind.as_str())).into_iter().flatten().copied() {
            if key(b) == key(m) || b.hash == m.hash || !self.fresh(b) {
                continue;
            }
            let Some(to_old) = self.sim_base(b, &old) else { continue };
            if to_old >= SIMILARITY_FLOOR && new.as_ref().is_none_or(|n| to_old > self.sim_base(b, n).unwrap_or(0.0)) {
                out.push(b);
            }
        }
        out
    }

    /// The merge-base body an added head symbol replaces: the merge-base symbol of its name and kind (one the branch removed
    /// first, the nearest by tokens on a tie), else the removed merge-base symbol of its kind nearest to it at >= 0.9 (a move
    /// with a rename). None for a pure add.
    fn fork_partner(&self, h: &'a Sym) -> Option<&'a Sym> {
        let fork = self.fork?;
        let g = self.g_fork.as_ref()?;
        let mine = self.g_head.of(h)?;
        let removed = |m: &Sym| !self.head.syms.contains_key(&key(m));
        let score = |m: &Sym| g.of(m).map_or(0.0, |o| jaccard(&mine, &o));
        let pick = |v: Vec<&'a Sym>| -> Option<&'a Sym> {
            v.into_iter().map(|m| (removed(m), score(m), m)).max_by(|a, b| (a.0, a.1).partial_cmp(&(b.0, b.1)).unwrap().then((&b.2.path, &b.2.qn).cmp(&(&a.2.path, &a.2.qn)))).map(|x| x.2)
        };
        if let Some(v) = self.fork_by_name.get(&(h.name.as_str(), h.kind.as_str())) {
            return pick(v.clone());
        }
        let near: Vec<&'a Sym> = fork.syms.values().filter(|m| m.kind == h.kind && removed(m) && score(m) >= SIMILARITY_FLOOR).collect();
        pick(near)
    }

    /// The reverse of `fork_partner`: the head symbol the branch added in place of removed `m` (its name and kind, not on the
    /// merge base), if any.
    fn head_partner(&self, m: &'a Sym) -> Option<&'a Sym> {
        let fork = self.fork?;
        self.head_by_name.get(&(m.name.as_str(), m.kind.as_str()))?.iter().copied().find(|h| !fork.syms.contains_key(&key(h)))
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
            // gone only when the clone answered; a clone that cannot be read says nothing about the ref
            if has_runs && gitref::probe(Path::new(&repo.path), label) == gitref::Probe::Missing {
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

/// Keep the summary by value for `decide --ref --summary-from-delta`: one meta row per head commit and one action-log line
/// (`delta-record`, by `engine`: the engine caching its own computation, never a judgement). Best effort: a read-only store skips it.
fn record_last(root: &Path, repo: &str, head_commit: &str, v: Value) {
    if let Ok(store) = Store::open_guarded(root) {
        let key = format!("delta:{repo}:{head_commit}");
        let _ = store.with_write_lock(|s| -> Result<Value> {
            if s.meta_get(&key)?.as_deref() == Some(v.to_string().as_str()) {
                return Ok(Value::Null); // the same value is already kept: no new line
            }
            s.meta_set(&key, &v.to_string())?;
            super::governance::append(
                s,
                root,
                &super::governance::Action {
                    verb: "delta-record".into(),
                    repo: Some(repo.to_string()),
                    by: "engine".into(),
                    inputs: json!({ "meta_key": key, "base_commit": v["base_commit"], "head_commit": head_commit, "merge_base": v["merge_base"] }),
                    ..Default::default()
                },
            )
        });
    }
}

/// The last delta computed for a head commit (full sha or a prefix of it): `{base_ref, base_commit, head_ref, head_commit,
/// merge_base, summary}`. A prefix that names deltas of more than one head commit is refused (`invalid_arguments`).
pub fn last_delta(store: &Store, repo: &str, commit: &str) -> Result<Option<Value>> {
    let prefix = format!("delta:{repo}:{commit}");
    let rows = store.meta_with_prefix(&prefix)?;
    if rows.len() > 1 {
        let heads: Vec<String> = rows.iter().map(|(k, _)| k[prefix.len() - commit.len()..].to_string()).collect();
        return Err(AppError::structured(
            "invalid_arguments",
            format!("--commit {commit} is ambiguous: deltas were recorded for {} head commits ({})", heads.len(), heads.join(", ")),
            Some("pass more of the commit (the full sha is never ambiguous)".into()),
            Some(json!({ "commit": commit, "candidates": heads })),
        ));
    }
    Ok(rows.last().and_then(|(_, v)| serde_json::from_str(v).ok()))
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
    let norm = |v: &[String]| {
        let mut v = v.to_vec();
        v.sort();
        v.dedup();
        v
    };
    let version = crate::providers::by_name_in(root, "reference").map(|p| p.version()).unwrap_or_default();
    let want = Want { version, include: norm(&repo.include), exclude: norm(&repo.exclude) };
    let mut rebuilt = vec![];
    let base = load_side(&store, root, &repo, o.base, &base_c, &want, mode, &mut rebuilt)?;
    let head = load_side(&store, root, &repo, o.head, &head_c, &want, mode, &mut rebuilt)?;
    let mb = load_side(&store, root, &repo, &mb_c, &mb_c, &want, mode, &mut rebuilt)?;
    // one delta, one set of file limits: a side that read fewer files would make the other side's symbols look added or removed
    let recorded: Vec<(&str, &Side)> = vec![("base", &base), ("head", &head), ("merge_base", &mb)];
    let known: Vec<&(&str, &Side)> = recorded.iter().filter(|(_, s)| s.limits.is_some()).collect();
    if known.windows(2).any(|w| w[0].1.limits != w[1].1.limits) {
        let shown: Vec<String> = known.iter().map(|(n, s)| format!("{n} (run {}) max_files {} max_file_bytes {}", s.run, s.limits.as_ref().unwrap()["max_files"], s.limits.as_ref().unwrap()["max_file_bytes"])).collect();
        return Err(AppError::structured(
            "invalid_arguments",
            format!("the sides of this delta were read with different file limits: {}; a delta never mixes them", shown.join("; ")),
            Some(format!("rebuild the sides with the same --max-files and --max-file-bytes (`comparegraph build {repo_name} --ref <ref> --max-file-bytes N`)")),
            Some(json!({ "limits": known.iter().map(|(n, s)| json!({ "side": n, "run": s.run, "limits": s.limits })).collect::<Vec<_>>() })),
        ));
    }

    // the branch's own work: merge base -> head, each row judged by the one rule
    let judge = Judge::new(&base, &head, Some(&mb));
    let (mut added, mut changed, mut removed) = (vec![], vec![], vec![]);
    for (k, h) in &head.syms {
        let in_base = base.syms.get(k);
        match mb.syms.get(k) {
            None => {
                let mut r = Row::of(h);
                r.set_on_base(h, judge.rule(Change::Head { h, m: judge.fork_partner(h) }).on_base);
                r.base_also_changed = r.on_base().is_none() && in_base.is_some();
                added.push(r);
            }
            Some(m) if m.hash != h.hash => {
                let mut r = Row::of(h);
                r.base_body_hash = Some(m.hash.clone());
                r.set_on_base(h, judge.rule(Change::Head { h, m: Some(m) }).on_base);
                match in_base {
                    None => {
                        r.base_removed = true;
                        r.base_moved_to = matches_of(&judge.copies_of(m));
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
            let v = judge.rule(Change::Removed { m });
            r.removal_resolved = v.removal_resolved;
            r.base_moved_to = matches_of(&v.copies);
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
    let mut diff_failed: Vec<String> = vec![];
    let touched_head: BTreeSet<String> = match git_lines(clone, &["diff", "--name-only", "--no-renames", &mb_c, &head_c]) {
        Some(v) => v.into_iter().collect(),
        None => {
            diff_failed.push(format!("git diff {}..{} (the files the branch touched)", &mb_c[..12.min(mb_c.len())], &head_c[..12.min(head_c.len())]));
            BTreeSet::new()
        }
    };
    let head_dir = gitref::cache_dir(root, &repo.name, &head_c);
    let base_diff = git_lines(clone, &["diff", "--name-status", "--no-renames", &mb_c, &base_c]);
    if base_diff.is_none() {
        diff_failed.push(format!("git diff {}..{} (the files base deleted or rewrote)", &mb_c[..12.min(mb_c.len())], &base_c[..12.min(base_c.len())]));
    }
    for l in base_diff.unwrap_or_default() {
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
    let on_base_n = added.iter().chain(changed.iter()).filter(|r| r.on_base().is_some()).count();
    let exact_n = added.iter().chain(changed.iter()).filter(|r| r.on_base().is_some_and(|m| m.via != "name_and_tokens")).count();
    let trivial_n = added.iter().chain(changed.iter()).filter(|r| r.note == Some("trivial_body")).count();
    let base_removed_n = changed.iter().chain(removed.iter()).filter(|r| r.base_removed).count();
    // a change or deletion base does not hold, to a symbol base changed otherwise, removed, or moved without it
    let stale_conflicts = changed.iter().chain(removed.iter()).filter(|r| r.stale_conflict()).count();
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
    let mut summary = summary;
    if !diff_failed.is_empty() {
        // not known is not zero
        for k in ["files_base_deleted", "files_base_rewritten", "files_branch_touched"] {
            summary[k] = Value::Null;
        }
    }
    let mut feats: Vec<(String, usize)> = touched.into_iter().collect();
    feats.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    // the whole-tree comparison, on request: everything head holds against everything base holds
    let whole = if o.whole_tree {
        // no fork here: everything head holds against everything base holds
        let wj = Judge::new(&base, &head, None);
        let (mut wa, mut wc, mut wr) = (vec![], vec![], vec![]);
        let mut unchanged = 0usize;
        for (k, h) in &head.syms {
            match base.syms.get(k) {
                None => {
                    let mut r = Row::of(h);
                    r.set_on_base(h, wj.rule(Change::Head { h, m: None }).on_base);
                    wa.push(r);
                }
                Some(b) if b.hash != h.hash => {
                    let mut r = Row::of(h);
                    r.base_body_hash = Some(b.hash.clone());
                    r.set_on_base(h, wj.rule(Change::Head { h, m: None }).on_base);
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
        let already = wa.iter().chain(wc.iter()).filter(|r| r.on_base().is_some()).count();
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
    if trivial_n > 0 {
        limitations.push(json!({ "code": "trivial_body", "detail": format!("{trivial_n} branch symbol(s) have fewer than {MIN_BODY_TOKENS} tokens after their signature (decorators and docstrings excluded): not matched by body hash or tokens (already_on_base: null)") }));
    }
    if !diff_failed.is_empty() {
        limitations.push(json!({ "code": "partial", "detail": format!("{} failed: base_files and the file counts are not known (null), not empty", diff_failed.join(" and ")) }));
    }
    // what each side's own read left out; the merge base is often the base's very run
    let mut seen_runs: Vec<i64> = vec![];
    for (s, r) in [(&base, o.base), (&head, o.head), (&mb, mb_c.as_str())] {
        let names: Vec<&str> = [("base", base.run), ("head", head.run), ("merge_base", mb.run)].iter().filter(|(_, id)| *id == s.run).map(|(n, _)| *n).collect();
        if seen_runs.contains(&s.run) {
            continue;
        }
        seen_runs.push(s.run);
        for l in &s.limitations {
            limitations.push(json!({ "code": l.code, "detail": format!("{} ({}, run {}): {}", names.join(" and "), &r[..r.len().min(40)], s.run, l.detail), "data": { "sides": names, "run": s.run } }));
        }
    }
    if recorded.iter().any(|(_, s)| s.limits.is_none()) {
        limitations.push(json!({ "code": "partial", "detail": "a side's run was built before file limits were recorded: whether all sides read the same files is not known" }));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn side(files: &[(&str, &str)], syms: &[(&str, &str, u32, u32)]) -> Side {
        let mut s = Side { run: 1, commit: "c".into(), syms: BTreeMap::new(), files: HashMap::new(), unreadable: 0, duplicates: 0, limits: None, limitations: vec![] };
        for (p, t) in files {
            s.files.insert(p.to_string(), t.lines().map(String::from).collect());
        }
        for (name, path, ls, le) in syms {
            let qn = format!("{path}::{name}");
            let sym = Sym { kind: "function".into(), name: name.to_string(), qn: qn.clone(), path: path.to_string(), ls: *ls, le: *le, hash: name.to_string(), trivial: false };
            s.syms.insert(("function".into(), qn), sym);
        }
        s
    }

    /// The bigrams of a body are computed once per symbol, however many times the rule compares it.
    #[test]
    fn bigrams_are_computed_once_per_symbol_and_side() {
        let s = side(&[("a.py", "def f(x):\n    return x + 1\ndef g(y):\n    return y + 2\n")], &[("f", "a.py", 1, 2), ("g", "a.py", 3, 4)]);
        let g = Grams::new(&s);
        let (f, gg) = (&s.syms[&("function".to_string(), "a.py::f".to_string())], &s.syms[&("function".to_string(), "a.py::g".to_string())]);
        for _ in 0..5 {
            assert!(g.of(f).is_some() && g.of(gg).is_some());
        }
        assert_eq!(g.computed.get(), 2);
    }

    /// The signature ends where the body starts, whatever the language and however many lines the parameters take.
    #[test]
    fn body_tokens_start_after_the_signature() {
        let py = |b: &str, n: &str| body_tokens(b, n, true);
        let other = |b: &str, n: &str| body_tokens(b, n, false);
        assert_eq!(py("def f(x): pass", "f"), 1);
        assert_eq!(py("def f(
    a,
    b,
):
    pass", "f"), 1);
        assert_eq!(py("def f(x: int = 1) -> Dict[str, int]:
    return x", "f"), 2);
        assert_eq!(py("def f(s=\")\"):
    return s", "f"), 2, "a `)` in a default string is not the signature's end");
        assert_eq!(py("@route(
  \"/a\", m=[1])
def f(x):
    \"\"\"Doc of many words here.\"\"\"
    return x", "f"), 2);
        assert_eq!(py("class C(Base):
    x = 1", "C"), 3);
        assert_eq!(other("function f(a: number): number {
  return a;
}", "f"), 4);
        assert_eq!(other("fn f<T: Clone>(x: [u8; 4]) -> Vec<T> {
    x
}", "f"), 2);
        assert_eq!(other("func (r *T) Name(x int) (int, error) {
	return 1, nil
}", "Name"), 5);
        assert_eq!(other("const f = (x) => x + 1", "f"), 3);
    }
}
