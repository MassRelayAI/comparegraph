//! `health <REPO>`: structural findings over one repo's latest run, each with evidence (path and line), bounded and sorted.
//!
//! Kinds: import cycles and `extends` cycles, dead imports, duplicate function bodies, size and coupling outliers, boundary
//! rules from `comparegraph.toml`, and the `dead-code` candidates folded in. All of it is a plain query over the store plus
//! the clone's files (read only; a file that changed since the build is skipped and said so). No model, no network.
//!
//! The ratchet: `--baseline FILE` writes the sorted finding keys; `--check FILE` fails only on a key not in the baseline.
//! Keys never carry a measured value (a file's line count, a fan-in), so growth of an existing outlier is not a new finding;
//! a cycle is keyed by its canonical rotation, so a new edge that closes a cycle is always a new key.
use super::{AppError, Result};
use crate::domain::{content_hash, Limitation};
use crate::providers::reference::is_test_path;
use crate::store::Store;
use regex::Regex;
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// Section names, in output order. `Options::disable` takes these (a test hook for mutation proofs).
pub const SECTIONS: &[&str] =
    &["cycles", "dead_imports", "duplicate_bodies", "size_outliers", "coupling_outliers", "boundary_violations", "dead_code"];

/// Sections whose measured precision is under the bar and that have no mechanical fix: reported, never in the ratchet.
const ADVISORY: &[&str] = &[];

const MAX_CYCLES: usize = 10_000;
const BFS_BUDGET: usize = 20_000_000;

/// One finding. Built only through [`Finding::new`], which refuses a finding without an evidence location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub kind: &'static str,
    pub key: String,
    pub message: String,
    pub evidence: Vec<(String, u32)>,
    /// impact for ranking, larger first (ties broken by key)
    pub weight: (i64, i64),
}
impl Finding {
    pub fn new(kind: &'static str, key: String, message: String, evidence: Vec<(String, u32)>) -> Result<Self> {
        if key.is_empty() || evidence.is_empty() || evidence.iter().any(|(p, l)| p.is_empty() || *l == 0) {
            return Err(AppError::InvalidArguments(format!(
                "health: finding `{key}` has no evidence location (a path and a 1-based line are required)"
            )));
        }
        Ok(Self { kind, key, message, evidence, weight: (0, 0) })
    }
    fn weighted(mut self, a: i64, b: i64) -> Self {
        self.weight = (a, b);
        self
    }
    fn to_json(&self) -> Value {
        json!({
            "kind": self.kind, "key": self.key, "message": self.message, "impact": [self.weight.0, self.weight.1],
            "evidence": self.evidence.iter().map(|(p, l)| json!({"path": p, "line": l})).collect::<Vec<_>>(),
        })
    }
}

type RuleDraft = Option<(Option<String>, Option<String>)>;

#[derive(Debug, Clone)]
pub struct Boundary {
    pub from: String,
    pub never: String,
}

/// `[health]` and `[[health.boundary]]` of `<root>/comparegraph.toml`.
#[derive(Debug, Clone)]
pub struct Config {
    pub min_lines: usize,
    pub max_file_lines: usize,
    pub max_function_lines: usize,
    pub max_file_fan_in: usize,
    pub max_file_fan_out: usize,
    pub max_function_fan_in: usize,
    pub max_function_fan_out: usize,
    pub boundaries: Vec<Boundary>,
    /// globs of paths left out of every section, the counts and the ratchet
    pub exclude: Vec<String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            min_lines: 8,
            max_file_lines: 1000,
            max_function_lines: 150,
            max_file_fan_in: 50,
            max_file_fan_out: 30,
            max_function_fan_in: 50,
            max_function_fan_out: 40,
            boundaries: vec![],
            exclude: vec![],
        }
    }
}

fn unquote(v: &str) -> Option<String> {
    let v = v.trim();
    (v.len() >= 2 && v.starts_with('"') && v.ends_with('"')).then(|| v[1..v.len() - 1].to_string())
}
fn strip_comment(v: &str) -> &str {
    let mut in_q = false;
    for (i, c) in v.char_indices() {
        match c {
            '"' => in_q = !in_q,
            '#' if !in_q => return v[..i].trim_end(),
            _ => {}
        }
    }
    v
}

impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        let Ok(text) = std::fs::read_to_string(root.join("comparegraph.toml")) else {
            return Ok(Self::default());
        };
        Self::parse(&text)
    }
    pub fn parse(text: &str) -> Result<Self> {
        #[derive(PartialEq)]
        enum S {
            Other,
            Health,
            Boundary,
        }
        let bad = |m: String| AppError::InvalidArguments(format!("comparegraph.toml: {m}"));
        let mut c = Self::default();
        let (mut sect, mut cur): (S, RuleDraft) = (S::Other, None);
        let flush = |cur: &mut RuleDraft, c: &mut Self| -> Result<()> {
            if let Some((f, n)) = cur.take() {
                match (f, n) {
                    (Some(from), Some(never)) => c.boundaries.push(Boundary { from, never }),
                    _ => return Err(bad("[[health.boundary]] needs both `from` and `never`".into())),
                }
            }
            Ok(())
        };
        for line in text.lines() {
            let t = strip_comment(line.trim());
            if t.is_empty() {
                continue;
            }
            if t.starts_with('[') {
                flush(&mut cur, &mut c)?;
                let name = t.trim_start_matches('[').trim_end_matches(']').trim();
                sect = match name {
                    "health" if !t.starts_with("[[") => S::Health,
                    "health.boundary" if t.starts_with("[[") => {
                        cur = Some((None, None));
                        S::Boundary
                    }
                    _ => S::Other,
                };
                continue;
            }
            let Some((k, v)) = t.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match sect {
                S::Other => {}
                S::Boundary => {
                    let s = unquote(v).ok_or_else(|| bad(format!("{k} must be a quoted glob")))?;
                    let b = cur.as_mut().unwrap();
                    match k {
                        "from" => b.0 = Some(s),
                        "never" => b.1 = Some(s),
                        _ => return Err(bad(format!("unknown key `{k}` in [[health.boundary]] (from, never)"))),
                    }
                }
                S::Health if k == "exclude" => {
                    let inner = v
                        .strip_prefix('[')
                        .and_then(|x| x.strip_suffix(']'))
                        .ok_or_else(|| bad("[health] exclude must be a list of quoted globs on one line".into()))?;
                    c.exclude = inner
                        .split(',')
                        .map(str::trim)
                        .filter(|x| !x.is_empty())
                        .map(|x| unquote(x).ok_or_else(|| bad("[health] exclude holds quoted globs".into())))
                        .collect::<Result<_>>()?;
                }
                S::Health => {
                    let n: usize = v.parse().map_err(|_| bad(format!("[health] {k} must be a non-negative integer")))?;
                    match k {
                        "min_lines" => c.min_lines = n,
                        "max_file_lines" => c.max_file_lines = n,
                        "max_function_lines" => c.max_function_lines = n,
                        "max_file_fan_in" => c.max_file_fan_in = n,
                        "max_file_fan_out" => c.max_file_fan_out = n,
                        "max_function_fan_in" => c.max_function_fan_in = n,
                        "max_function_fan_out" => c.max_function_fan_out = n,
                        _ => return Err(bad(format!("unknown key `{k}` in [health]"))),
                    }
                }
            }
        }
        flush(&mut cur, &mut c)?;
        Ok(c)
    }
    fn thresholds(&self) -> Value {
        json!({
            "min_lines": self.min_lines, "max_file_lines": self.max_file_lines, "max_function_lines": self.max_function_lines,
            "max_file_fan_in": self.max_file_fan_in, "max_file_fan_out": self.max_file_fan_out,
            "max_function_fan_in": self.max_function_fan_in, "max_function_fan_out": self.max_function_fan_out,
            "exclude": self.exclude, "boundaries": self.boundaries.iter().map(|b| json!({"from": b.from, "never": b.never})).collect::<Vec<_>>(),
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// items listed per section (the counts and the ratchet always cover every finding)
    pub limit: usize,
    /// test hook: sections whose detector returns nothing (mutation proofs)
    pub disable: Vec<String>,
    pub baseline: Option<PathBuf>,
    pub check: Option<PathBuf>,
    /// dead-code candidates are never in the ratchet unless asked for
    pub ratchet_dead_code: bool,
}

/// Limitation codes that make a `--check` partial (`check.partial`): the read left files out or cut the cycle search short.
const PARTIAL_CODES: &[&str] = &["files_skipped", "cycles_truncated"];

/// The answer, and the ratchet verdict: `failed` is set when `--check` found a new finding.
pub struct Outcome {
    pub data: Value,
    pub failed: Option<String>,
}

// ---- store reads ----

struct N {
    id: i64,
    kind: String,
    qn: String,
    path: String,
    ls: u32,
    le: u32,
    hash: String,
}

fn load_nodes(store: &Store, run: i64) -> Result<Vec<N>> {
    let mut st = store.conn().prepare(
        "SELECT id, kind, name, qualified_name, path, line_start, line_end, content_hash FROM node WHERE run_id=?1 ORDER BY path, line_start, id",
    )?;
    let rows = st.query_map(params![run], |r| {
        Ok(N { id: r.get(0)?, kind: r.get(1)?, qn: r.get(3)?, path: r.get(4)?, ls: r.get(5)?, le: r.get(6)?, hash: r.get(7)? })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

type Edges = Vec<(i64, i64)>;
fn load_edges(store: &Store, run: i64, kind: &str, min_conf: f64) -> Result<Edges> {
    let mut st = store
        .conn()
        .prepare("SELECT source_node_id, target_node_id FROM edge WHERE run_id=?1 AND kind=?2 AND confidence >= ?3")?;
    let rows = st.query_map(params![run, kind, min_conf], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

// ---- cycles ----

/// The directory prefix a Rust file's child modules live under: `lib.rs`/`main.rs`/`mod.rs` -> its directory, `name.rs` -> `dir/name/`.
fn rust_module_dir(path: &str) -> Option<String> {
    let stem = path.strip_suffix(".rs")?;
    let (dir, name) = stem.rsplit_once('/').unwrap_or(("", stem));
    let pre = if dir.is_empty() { String::new() } else { format!("{dir}/") };
    Some(if matches!(name, "lib" | "main" | "mod") { pre } else { format!("{pre}{name}/") })
}
/// `a` declares (directly or through nested modules) the module `b`: an edge either way is `mod b;` or `use super::`/`use crate::`, never a dependency cycle.
fn rust_ancestor(a: &str, b: &str) -> bool {
    a != b && rust_module_dir(a).is_some_and(|d| b.ends_with(".rs") && b.starts_with(&d))
}


/// Elementary cycles of a directed graph given as sorted labels: for every edge inside a strongly connected component, the
/// shortest cycle through that edge, rotated to start at its smallest label. A new edge that closes a cycle therefore always
/// yields a new cycle. Returns the cycles sorted, and whether a cap was hit.
pub fn find_cycles(edges: &BTreeSet<(String, String)>) -> (Vec<Vec<String>>, bool) {
    let labels: Vec<&String> = edges.iter().flat_map(|(a, b)| [a, b]).collect::<BTreeSet<_>>().into_iter().collect();
    let idx: HashMap<&String, usize> = labels.iter().enumerate().map(|(i, l)| (*l, i)).collect();
    let mut adj: Vec<Vec<usize>> = vec![vec![]; labels.len()];
    for (a, b) in edges {
        let (a, b) = (idx[a], idx[b]);
        if a != b {
            adj[a].push(b);
        }
    }
    for a in &mut adj {
        a.sort_unstable();
        a.dedup();
    }
    // Tarjan, iterative
    let n = labels.len();
    let (mut index, mut low, mut on, mut comp) = (vec![usize::MAX; n], vec![0usize; n], vec![false; n], vec![usize::MAX; n]);
    let (mut stack, mut next, mut ncomp) = (vec![], 0usize, 0usize);
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        let mut work = vec![(root, 0usize)];
        while let Some(&mut (v, ref mut i)) = work.last_mut() {
            if *i == 0 && index[v] == usize::MAX {
                index[v] = next;
                low[v] = next;
                next += 1;
                stack.push(v);
                on[v] = true;
            }
            if *i < adj[v].len() {
                let w = adj[v][*i];
                *i += 1;
                if index[w] == usize::MAX {
                    work.push((w, 0));
                } else if on[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                if low[v] == index[v] {
                    while let Some(w) = stack.pop() {
                        on[w] = false;
                        comp[w] = ncomp;
                        if w == v {
                            break;
                        }
                    }
                    ncomp += 1;
                }
                work.pop();
                if let Some(&(p, _)) = work.last() {
                    low[p] = low[p].min(low[v]);
                }
            }
        }
    }
    let mut found: BTreeSet<Vec<usize>> = BTreeSet::new();
    let (mut budget, mut truncated) = (BFS_BUDGET, false);
    'outer: for u in 0..n {
        for &v in &adj[u] {
            if comp[u] != comp[v] {
                continue;
            }
            // shortest path v -> u inside the component
            let mut parent: HashMap<usize, usize> = HashMap::new();
            let mut q = VecDeque::from([v]);
            parent.insert(v, v);
            while let Some(x) = q.pop_front() {
                if x == u {
                    break;
                }
                for &y in &adj[x] {
                    if comp[y] == comp[u] && !parent.contains_key(&y) {
                        parent.insert(y, x);
                        q.push_back(y);
                    }
                }
                budget = budget.saturating_sub(adj[x].len() + 1);
            }
            if budget == 0 || found.len() >= MAX_CYCLES {
                truncated = true;
                break 'outer;
            }
            let mut path = vec![u];
            let mut x = u;
            while x != v {
                x = parent[&x];
                path.push(x);
            }
            path.reverse(); // v .. u
            let m = path.iter().enumerate().min_by_key(|(_, l)| **l).map(|(i, _)| i).unwrap();
            path.rotate_left(m);
            found.insert(path);
        }
    }
    let mut out: Vec<Vec<String>> = found.into_iter().map(|c| c.into_iter().map(|i| labels[i].clone()).collect()).collect();
    out.sort();
    (out, truncated)
}

fn cycle_findings(
    kind_label: &str,
    edges: &Edges,
    by_id: &HashMap<i64, &N>,
    label: &dyn Fn(&N) -> String,
    lines: bool,
    fan_in: &HashMap<String, usize>,
    lims: &mut Vec<Limitation>,
) -> Result<Vec<Finding>> {
    let mut set = BTreeSet::new();
    let mut at: HashMap<String, (String, u32)> = HashMap::new();
    for (a, b) in edges {
        let (Some(na), Some(nb)) = (by_id.get(a), by_id.get(b)) else { continue };
        let (la, lb) = (label(na), label(nb));
        at.entry(la.clone()).or_insert((na.path.clone(), if lines { na.ls.max(1) } else { 1 }));
        at.entry(lb.clone()).or_insert((nb.path.clone(), if lines { nb.ls.max(1) } else { 1 }));
        set.insert((la, lb));
    }
    let (cycles, truncated) = find_cycles(&set);
    if truncated {
        lims.push(Limitation::new("cycles_truncated", format!("{kind_label}: listing stopped at {MAX_CYCLES} cycles or the search budget")));
    }
    let mut out = vec![];
    for c in cycles {
        let mut shown = c.clone();
        shown.push(c[0].clone());
        out.push(Finding::new(
            "cycle",
            format!("cycle:{kind_label}:{}", c.join(">")),
            format!("{kind_label} cycle of {}: {}", c.len(), shown.join(" -> ")),
            c.iter().map(|l| at[l].clone()).collect(),
        )?
        .weighted(-(c.len() as i64), c.iter().map(|l| fan_in.get(l).copied().unwrap_or(0) as i64).sum()));
    }
    Ok(out)
}

// ---- per-file source analysis ----

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}
/// Does `name` occur in `text` as a whole identifier?
fn mentions(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(i, _)| {
        let before = text[..i].chars().next_back().is_none_or(|c| !is_ident(c));
        let after = text[i + name.len()..].chars().next().is_none_or(|c| !is_ident(c));
        before && after
    })
}
fn line_of(text: &str, byte: usize) -> u32 {
    text[..byte].bytes().filter(|b| *b == b'\n').count() as u32 + 1
}
fn blank(text: &str, ranges: &[(usize, usize)]) -> String {
    let mut out = text.as_bytes().to_vec();
    for &(a, b) in ranges {
        for x in &mut out[a..b] {
            if *x != b'\n' {
                *x = b' ';
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One import statement: its local names, module text, line and byte range.
struct Import {
    names: Vec<String>,
    module: String,
    line: u32,
    range: (usize, usize),
}

/// `//` and `/* */` comments removed (an import clause can carry them between its names).
fn strip_js_comments(s: &str) -> String {
    let line = Regex::new(r"//[^\n]*").unwrap();
    let block = Regex::new(r"(?s)/\*.*?\*/").unwrap();
    line.replace_all(&block.replace_all(s, " "), " ").into_owned()
}

fn ts_imports(src: &str, is_jsx: bool) -> Vec<Import> {
    let re = Regex::new(r#"(?m)^[ \t]*import\s+(?:type\s+)?([^'";]*?)\s*from\s*['"]([^'"]+)['"]"#).unwrap();
    let mut out = vec![];
    for c in re.captures_iter(src) {
        let m = c.get(0).unwrap();
        let clause = strip_js_comments(c.get(1).unwrap().as_str());
        let clause = clause.as_str();
        let mut names = vec![];
        let (head, braces) = match (clause.find('{'), clause.find('}')) {
            (Some(a), Some(b)) if b > a => (&clause[..a], Some(&clause[a + 1..b])),
            _ => (clause, None),
        };
        for part in head.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let n = part.strip_prefix("* as ").unwrap_or(part).trim();
            if !n.is_empty() && !(is_jsx && n == "React") {
                names.push(n.to_string());
            }
        }
        for part in braces.into_iter().flat_map(|b| b.split(',')).map(str::trim).filter(|p| !p.is_empty()) {
            let part = part.strip_prefix("type ").unwrap_or(part);
            let n = part.rsplit(" as ").next().unwrap().trim();
            names.push(n.to_string());
        }
        let start = m.start() + (m.as_str().len() - m.as_str().trim_start().len());
        out.push(Import { names, module: c[2].to_string(), line: line_of(src, start), range: (start, m.end()) });
    }
    out
}

fn py_imports(src: &str) -> Vec<Import> {
    let mut out = vec![];
    let plain = Regex::new(r"(?m)^[ \t]*import[ \t]+([\w.]+(?:[ \t]+as[ \t]+\w+)?(?:[ \t]*,[ \t]*[\w.]+(?:[ \t]+as[ \t]+\w+)?)*)").unwrap();
    for c in plain.captures_iter(src) {
        let m = c.get(0).unwrap();
        let names = c[1]
            .split(',')
            .map(str::trim)
            .map(|p| match p.split_once(" as ") {
                Some((_, a)) => a.trim().to_string(),
                None => p.split('.').next().unwrap().to_string(),
            })
            .collect();
        let start = m.start() + (m.as_str().len() - m.as_str().trim_start().len());
        out.push(Import { names, module: c[1].to_string(), line: line_of(src, start), range: (start, m.end()) });
    }
    let from = Regex::new(r"(?m)^[ \t]*from[ \t]+([\w.]+)[ \t]+import[ \t]+(\([^)]*\)|[^\n\\]+)").unwrap();
    for c in from.captures_iter(src) {
        let m = c.get(0).unwrap();
        if &c[1] == "__future__" {
            continue;
        }
        let body = c[2].trim().trim_start_matches('(').trim_end_matches(')');
        let mut names = vec![];
        for l in body.lines() {
            let l = l.split('#').next().unwrap();
            for p in l.split(',').map(str::trim).filter(|p| !p.is_empty() && *p != "*") {
                names.push(p.rsplit(" as ").next().unwrap().trim().to_string());
            }
        }
        let start = m.start() + (m.as_str().len() - m.as_str().trim_start().len());
        out.push(Import { names, module: c[1].to_string(), line: line_of(src, start), range: (start, m.end()) });
    }
    out
}

/// Traits used only through method calls never show their name: not reported.
const RUST_TRAIT_NAMES: &[&str] = &[
    "Write", "Read", "BufRead", "Seek", "FromStr", "TryFrom", "TryInto", "Hasher", "Borrow", "BorrowMut", "Deref", "DerefMut",
    "IntoIterator", "FromIterator", "Extend", "Digest", "Itertools", "StreamExt", "FutureExt", "TryStreamExt", "Rng", "RngCore", "SliceRandom",
];

fn split_top(s: &str) -> Vec<&str> {
    let (mut out, mut depth, mut from) = (vec![], 0i32, 0usize);
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[from..i]);
                from = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[from..]);
    out
}
fn rust_use_names(tree: &str, parent: Option<&str>, out: &mut Vec<String>) {
    let t = tree.trim();
    if t.is_empty() || t == "*" || t.ends_with("::*") {
        return;
    }
    if let Some(open) = t.find('{') {
        let close = t.rfind('}').unwrap_or(t.len());
        let prefix = t[..open].trim().trim_end_matches("::");
        let last = prefix.rsplit("::").next().filter(|s| !s.is_empty()).or(parent);
        for part in split_top(&t[open + 1..close]) {
            rust_use_names(part, last, out);
        }
        return;
    }
    let (path, alias) = match t.split_once(" as ") {
        Some((p, a)) => (p.trim(), Some(a.trim())),
        None => (t, None),
    };
    let mut name = alias.unwrap_or_else(|| path.rsplit("::").next().unwrap());
    if name == "self" {
        match parent {
            Some(p) => name = p,
            None => return,
        }
    }
    if name != "_" && !name.is_empty() {
        out.push(name.to_string());
    }
}
fn rs_imports(src: &str) -> Vec<Import> {
    let re = Regex::new(r"(?m)^[ \t]*(pub(?:\([^)]*\))?[ \t]+)?use[ \t\n]+([^;]+);").unwrap();
    let mut out = vec![];
    for c in re.captures_iter(src) {
        if c.get(1).is_some() {
            continue;
        }
        let m = c.get(0).unwrap();
        let compact: String = c[2].split_whitespace().collect::<Vec<_>>().join(" ");
        let mut names = vec![];
        rust_use_names(&compact, None, &mut names);
        // a capitalised name may be a trait used only through its methods (`sqlx::Row`, `base64::Engine`): the text cannot tell, so only lowercase names (functions, modules, macros) are judged
        names.retain(|n| !RUST_TRAIT_NAMES.contains(&n.as_str()) && !n.ends_with("Ext") && !n.starts_with(char::is_uppercase));
        let start = m.start() + (m.as_str().len() - m.as_str().trim_start().len());
        out.push(Import { names, module: compact, line: line_of(src, start), range: (start, m.end()) });
    }
    out
}

fn dead_imports_of(path: &str, src: &str) -> Vec<(String, String, u32)> {
    let ext = path.rsplit('.').next().unwrap_or("");
    let imports = match ext {
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" => ts_imports(src, matches!(ext, "tsx" | "jsx")),
        "py" if !path.ends_with("__init__.py") => py_imports(src),
        "rs" => rs_imports(src),
        _ => return vec![],
    };
    let rest = blank(src, &imports.iter().map(|i| i.range).collect::<Vec<_>>());
    let mut out = vec![];
    for i in &imports {
        for n in &i.names {
            if !mentions(&rest, n) {
                out.push((n.clone(), i.module.clone(), i.line));
            }
        }
    }
    out
}

/// A function body normalised for comparison: blank and comment-only lines dropped, whitespace collapsed, the first
/// significant line (the signature) left out. Returns the significant line count of the whole function and the hash.
fn body_signature(lines: &[&str]) -> (usize, Option<String>) {
    let sig: Vec<String> = lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with('#') && !l.starts_with("/*") && !l.starts_with('*'))
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    if sig.len() < 2 {
        return (sig.len(), None);
    }
    let refs: Vec<&str> = sig[1..].iter().map(String::as_str).collect();
    (sig.len(), Some(content_hash(&refs)))
}

// ---- globs ----

pub fn glob_regex(g: &str) -> Result<Regex> {
    let mut re = String::from("^");
    let cs: Vec<char> = g.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '*' if cs.get(i + 1) == Some(&'*') => {
                if cs.get(i + 2) == Some(&'/') {
                    re.push_str("(?:.*/)?");
                    i += 3;
                } else {
                    re.push_str(".*");
                    i += 2;
                }
                continue;
            }
            '*' => re.push_str("[^/]*"),
            '?' => re.push_str("[^/]"),
            c => re.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    re.push('$');
    Regex::new(&re).map_err(|e| AppError::InvalidArguments(format!("bad glob `{g}`: {e}")))
}

// ---- the verb ----

pub fn health(root: &Path, repo: &str, o: &Options) -> Result<Outcome> {
    if !(1..=500).contains(&o.limit) {
        return Err(AppError::InvalidArguments("limit must be in 1..500".into()));
    }
    if o.baseline.is_some() && o.check.is_some() {
        return Err(AppError::InvalidArguments("--baseline and --check are exclusive".into()));
    }
    let cfg = Config::load(root)?;
    let store = Store::open_existing(root)?;
    let r = store.repo_by_name(repo)?.ok_or_else(|| AppError::NotFound(format!("no repo named {repo}")))?;
    let run = store.latest_done_run(r.id.unwrap())?.ok_or_else(|| AppError::NotFound(format!("{repo} has no finished run")))?;
    let run_id = run.id.unwrap().0;
    let on = |s: &str| !o.disable.iter().any(|d| d == s);
    let mut lims: Vec<Limitation> = vec![];
    let mut nodes = load_nodes(&store, run_id)?;
    let excludes: Vec<Regex> = cfg.exclude.iter().map(|g| glob_regex(g)).collect::<Result<_>>()?;
    let excluded = |p: &str| excludes.iter().any(|r| r.is_match(p));
    if !excludes.is_empty() {
        let before = nodes.len();
        nodes.retain(|n| !excluded(&n.path));
        lims.push(Limitation::new(
            "excludes",
            format!("[health] exclude {:?} left {} of {} nodes out of every section", cfg.exclude, before - nodes.len(), before),
        ));
    }
    let by_id: HashMap<i64, &N> = nodes.iter().map(|n| (n.id, n)).collect();
    let is_file = |n: &N| n.kind == "file" || n.kind == "test";
    let is_fn = |n: &N| n.kind == "function" || n.kind == "method";
    let mut sections: BTreeMap<&str, Vec<Finding>> = SECTIONS.iter().map(|s| (*s, vec![])).collect();

    // cycles
    if on("cycles") {
        let raw = load_edges(&store, run_id, "imports", 0.5)?;
        let mut fi: HashMap<String, HashSet<i64>> = HashMap::new();
        for (a, b) in &raw {
            if let (Some(_), Some(nb)) = (by_id.get(a), by_id.get(b)) {
                fi.entry(nb.path.clone()).or_default().insert(*a);
            }
        }
        let fan_in: HashMap<String, usize> = fi.into_iter().map(|(k, v)| (k, v.len())).collect();
        let imports: Edges = raw
            .into_iter()
            .filter(|(a, b)| match (by_id.get(a), by_id.get(b)) {
                (Some(x), Some(y)) => is_file(x) && is_file(y) && !rust_ancestor(&x.path, &y.path) && !rust_ancestor(&y.path, &x.path),
                _ => false,
            })
            .collect();
        let mut f = cycle_findings("imports", &imports, &by_id, &|n| n.path.clone(), false, &fan_in, &mut lims)?;
        let ext = load_edges(&store, run_id, "extends", 0.5)?;
        f.extend(cycle_findings("extends", &ext, &by_id, &|n| format!("{}@{}", n.qn, n.path), true, &HashMap::new(), &mut lims)?);
        sections.insert("cycles", f);
    }

    // imports adjacency for coupling and boundaries
    let imports = if on("coupling_outliers") || on("boundary_violations") { load_edges(&store, run_id, "imports", 0.5)? } else { vec![] };

    // size
    if on("size_outliers") {
        let mut f = vec![];
        for n in &nodes {
            if is_file(n) && !is_test_path(&n.path) && n.le as usize > cfg.max_file_lines {
                f.push(Finding::new(
                    "size",
                    format!("size:file:{}", n.path),
                    format!("file has {} lines (limit {})", n.le, cfg.max_file_lines),
                    vec![(n.path.clone(), 1)],
                )?
                .weighted(n.le as i64, 0));
            } else if is_fn(n) && !is_test_path(&n.path) {
                let len = (n.le + 1).saturating_sub(n.ls) as usize;
                if len > cfg.max_function_lines {
                    f.push(Finding::new(
                        "size",
                        format!("size:function:{}@{}", n.qn, n.path),
                        format!("{} has {} lines (limit {})", n.qn, len, cfg.max_function_lines),
                        vec![(n.path.clone(), n.ls.max(1))],
                    )?
                    .weighted(len as i64, 0));
                }
            }
        }
        sections.insert("size_outliers", f);
    }

    // coupling
    if on("coupling_outliers") {
        let mut f = vec![];
        let (mut fin, mut fout): (HashMap<i64, HashSet<i64>>, HashMap<i64, HashSet<i64>>) = Default::default();
        for (a, b) in &imports {
            // `mod x;` and `use super::`/`use crate::` between a Rust module and the modules it declares are not coupling
            let declared = matches!((by_id.get(a), by_id.get(b)), (Some(x), Some(y)) if rust_ancestor(&x.path, &y.path) || rust_ancestor(&y.path, &x.path));
            if a != b && !declared {
                fout.entry(*a).or_default().insert(*b);
                fin.entry(*b).or_default().insert(*a);
            }
        }
        let push = |what: &str, n: &N, count: usize, limit: usize, f: &mut Vec<Finding>| -> Result<()> {
            if count > limit {
                let key = if n.kind == "file" || n.kind == "test" { n.path.clone() } else { format!("{}@{}", n.qn, n.path) };
                f.push(Finding::new(
                    "coupling",
                    format!("coupling:{what}:{key}"),
                    format!("{what} {count} (limit {limit}): {key}"),
                    vec![(n.path.clone(), n.ls.max(1))],
                )?
                .weighted(count as i64, 0));
            }
            Ok(())
        };
        for n in nodes.iter().filter(|n| is_file(n) && !is_test_path(&n.path)) {
            push("file-fan-in", n, fin.get(&n.id).map_or(0, HashSet::len), cfg.max_file_fan_in, &mut f)?;
            push("file-fan-out", n, fout.get(&n.id).map_or(0, HashSet::len), cfg.max_file_fan_out, &mut f)?;
        }
        let (mut cin, mut cout): (HashMap<i64, HashSet<i64>>, HashMap<i64, HashSet<i64>>) = Default::default();
        // only resolved calls (1.0): a name-guessed edge (0.5) would make `new`, `from` or `start` look like the most-called code
        for (a, b) in load_edges(&store, run_id, "calls", 1.0)? {
            if a != b {
                cout.entry(a).or_default().insert(b);
                cin.entry(b).or_default().insert(a);
            }
        }
        for n in nodes.iter().filter(|n| is_fn(n) && !is_test_path(&n.path)) {
            push("function-fan-in", n, cin.get(&n.id).map_or(0, HashSet::len), cfg.max_function_fan_in, &mut f)?;
            push("function-fan-out", n, cout.get(&n.id).map_or(0, HashSet::len), cfg.max_function_fan_out, &mut f)?;
        }
        sections.insert("coupling_outliers", f);
    }

    // boundaries
    if on("boundary_violations") && !cfg.boundaries.is_empty() {
        let rules: Vec<(&Boundary, Regex, Regex)> = cfg
            .boundaries
            .iter()
            .map(|b| Ok((b, glob_regex(&b.from)?, glob_regex(&b.never)?)))
            .collect::<Result<_>>()?;
        let mut f = vec![];
        for (a, b) in &imports {
            let (Some(na), Some(nb)) = (by_id.get(a), by_id.get(b)) else { continue };
            for (rule, from, never) in &rules {
                if from.is_match(&na.path) && never.is_match(&nb.path) {
                    f.push(Finding::new(
                        "boundary",
                        format!("boundary:{}>{}|{}", na.path, nb.path, rule.never),
                        format!("{} imports {}, forbidden by from={} never={}", na.path, nb.path, rule.from, rule.never),
                        vec![(na.path.clone(), 1), (nb.path.clone(), 1)],
                    )?);
                }
            }
        }
        sections.insert("boundary_violations", f);
    }

    // per-file source: dead imports and duplicate bodies
    if on("dead_imports") || on("duplicate_bodies") {
        let clone = Path::new(&r.path);
        let mut fns_by_path: HashMap<&str, Vec<&N>> = HashMap::new();
        for n in nodes.iter().filter(|n| is_fn(n) && !is_test_path(&n.path)) {
            fns_by_path.entry(n.path.as_str()).or_default().push(n);
        }
        let (mut skipped, mut read) = (0usize, 0usize);
        let mut groups: BTreeMap<String, Vec<(String, u32, String, usize)>> = BTreeMap::new();
        let mut dead = vec![];
        for fnode in nodes.iter().filter(|n| is_file(n)) {
            let src = std::fs::read_to_string(clone.join(&fnode.path)).ok();
            let Some(src) = src else {
                skipped += 1;
                continue;
            };
            let ls: Vec<&str> = src.lines().collect();
            if content_hash(&ls) != fnode.hash {
                skipped += 1;
                continue;
            }
            read += 1;
            if on("dead_imports") {
                for (name, module, line) in dead_imports_of(&fnode.path, &src) {
                    dead.push(Finding::new(
                        "dead-import",
                        format!("dead-import:{}:{}", fnode.path, name),
                        format!("`{name}` imported from `{module}` is never referenced in {}", fnode.path),
                        vec![(fnode.path.clone(), line)],
                    )?);
                }
            }
            if on("duplicate_bodies") {
                for n in fns_by_path.get(fnode.path.as_str()).into_iter().flatten() {
                    let (a, b) = (n.ls.max(1) as usize, n.le as usize);
                    if b < a || b > ls.len() || b - a + 1 < cfg.min_lines {
                        continue;
                    }
                    if let (sig, Some(h)) = body_signature(&ls[a - 1..b]) {
                        if sig >= cfg.min_lines {
                            groups.entry(h).or_default().push((n.path.clone(), n.ls.max(1), n.qn.clone(), sig));
                        }
                    }
                }
            }
        }
        if skipped > 0 {
            lims.push(Limitation::new(
                "files_skipped",
                format!("{skipped} file(s) unreadable or changed since the build were left out of dead-imports and duplicate-bodies ({read} read); rebuild to include them"),
            ));
        }
        if on("dead_imports") {
            lims.push(Limitation::new(
                "dead_imports_scope",
                "statement-level, from the file text (edges carry no names): TypeScript/JavaScript, Python and Rust; a Rust trait imported only for its methods is skipped by name, other uses through a macro or a string are not seen",
            ));
            sections.insert("dead_imports", dead);
        }
        if on("duplicate_bodies") {
            let mut f = vec![];
            for (h, mut m) in groups {
                if m.len() < 2 {
                    continue;
                }
                m.sort();
                let lines = m[0].3;
                let names: Vec<String> = m.iter().take(5).map(|x| x.2.clone()).collect();
                // keyed by the body AND the member set: a third copy of a baselined pair is a new finding
                let mut members: Vec<&str> = m.iter().map(|x| x.2.as_str()).collect();
                members.dedup();
                let set = content_hash(&members);
                f.push(Finding::new(
                    "duplicate-body",
                    format!("duplicate-body:{}:{}", &h[..12], &set[..12.min(set.len())]),
                    format!("{} functions share one body of {} significant lines: {}{}", m.len(), lines, names.join(", "), if m.len() > 5 { ", ..." } else { "" }),
                    m.iter().take(20).map(|x| (x.0.clone(), x.1)).collect(),
                )?
                .weighted((m.len() * lines) as i64, m.len() as i64));
            }
            sections.insert("duplicate_bodies", f);
        }
    }

    // dead code, folded in from the dead-code query
    if on("dead_code") {
        let mut f = vec![];
        let mut cursor: Option<String> = None;
        loop {
            // the query pages at 500; the ratchet needs every candidate
            let v = super::graph::dead_code(root, repo, 500, cursor.as_deref(), 0.5)?;
            for it in v["items"].as_array().into_iter().flatten() {
                if excluded(it["path"].as_str().unwrap_or("")) {
                    continue;
                }
                let (qn, path, line) = (it["qualified_name"].as_str().unwrap_or(""), it["path"].as_str().unwrap_or(""), it["line_start"].as_u64().unwrap_or(0) as u32);
                f.push(Finding::new(
                    "dead-code",
                    format!("dead-code:{qn}@{path}"),
                    format!("{qn} is referenced by nothing (candidate; dynamic dispatch is invisible)"),
                    vec![(path.to_string(), line)],
                )?);
            }
            match v["page"]["next"].as_str() {
                Some(n) => cursor = Some(n.to_string()),
                None => break,
            }
        }
        lims.push(Limitation::new("dead_code_candidates", "dead-code findings are candidates (confidence unknown); they are in the baseline and the check only with --ratchet-dead-code"));
        sections.insert("dead_code", f);
    }

    for f in sections.values_mut() {
        f.sort_by(|a, b| a.key.cmp(&b.key));
        f.dedup_by(|a, b| a.key == b.key);
        // by impact, largest first; the key order above is the stable tiebreak
        f.sort_by_key(|x| std::cmp::Reverse(x.weight));
    }

    // output
    let mut summary = serde_json::Map::new();
    let mut out_sections = serde_json::Map::new();
    let mut total = 0usize;
    let mut truncated = false;
    for s in SECTIONS {
        let f = &sections[s];
        total += f.len();
        summary.insert((*s).into(), json!(f.len()));
        truncated |= f.len() > o.limit;
        out_sections.insert(
            (*s).into(),
            json!({ "total": f.len(), "advisory": ADVISORY.contains(s) || (*s == "dead_code" && !o.ratchet_dead_code), "items": f.iter().take(o.limit).map(Finding::to_json).collect::<Vec<_>>() }),
        );
    }
    summary.insert("total".into(), json!(total));
    if truncated {
        lims.push(Limitation::new("truncated", format!("each section lists its first {} findings; the counts, the baseline and the check cover all of them", o.limit)));
    }
    let ratchet: Vec<&Finding> = SECTIONS
        .iter()
        .filter(|s| !ADVISORY.contains(s) && (**s != "dead_code" || o.ratchet_dead_code))
        .flat_map(|s| sections[s].iter())
        .collect();
    let keys: BTreeSet<&str> = ratchet.iter().map(|f| f.key.as_str()).collect();

    let mut data = json!({
        "repo": r.name, "run_id": run_id, "thresholds": cfg.thresholds(),
        "summary": Value::Object(summary), "sections": Value::Object(out_sections),
        "evidence_confidence": "derived",
        "limitation_items": serde_json::to_value(&lims).unwrap(),
    });
    let mut failed = None;
    if let Some(p) = &o.baseline {
        let body = serde_json::to_string_pretty(&json!({ "schema": 1, "keys": keys })).unwrap() + "\n";
        std::fs::write(p, body).map_err(|e| AppError::Io(p.clone(), e))?;
        data["baseline"] = json!({ "written": p, "keys": keys.len() });
    }
    if let Some(p) = &o.check {
        let text = std::fs::read_to_string(p).map_err(|e| AppError::Io(p.clone(), e))?;
        let v: Value = serde_json::from_str(&text).map_err(|e| AppError::InvalidArguments(format!("{}: not a health baseline: {e}", p.display())))?;
        let base: BTreeSet<String> = v["keys"]
            .as_array()
            .ok_or_else(|| AppError::InvalidArguments(format!("{}: not a health baseline (no keys)", p.display())))?
            .iter()
            .filter_map(|k| k.as_str().map(String::from))
            .collect();
        let new: Vec<&&Finding> = ratchet.iter().filter(|f| !base.contains(&f.key)).collect();
        let fixed: Vec<&String> = base.iter().filter(|k| !keys.contains(k.as_str())).collect();
        let message = if !new.is_empty() {
            failed = Some(format!("{} new finding(s) not in the baseline", new.len()));
            format!("{} new finding(s) not in the baseline", new.len())
        } else if !fixed.is_empty() {
            format!("{} finding(s) now clean, shrink the baseline (rerun with --baseline)", fixed.len())
        } else {
            "no new findings; the baseline matches".to_string()
        };
        // a check over a partial read (files left out, a cycle search cut short) cannot see every finding: it says so
        let partial_reasons: Vec<&Limitation> = lims.iter().filter(|l| PARTIAL_CODES.contains(&l.code.as_str())).collect();
        let partial = !partial_reasons.is_empty();
        let message = if partial { format!("partial: {message} (over a partial read: {})", partial_reasons.iter().map(|l| l.code.as_str()).collect::<Vec<_>>().join(", ")) } else { message };
        data["check"] = json!({
            "baseline": p, "message": message, "new_total": new.len(), "fixed_total": fixed.len(),
            "partial": partial, "partial_reasons": partial_reasons,
            "new": new.iter().take(o.limit).map(|f| f.to_json()).collect::<Vec<_>>(),
            "fixed": fixed.iter().take(o.limit).collect::<Vec<_>>(),
        });
    }
    Ok(Outcome { data, failed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finding_without_evidence_is_refused() {
        assert!(Finding::new("cycle", "k".into(), "m".into(), vec![]).is_err());
        assert!(Finding::new("cycle", "k".into(), "m".into(), vec![("".into(), 1)]).is_err());
        assert!(Finding::new("cycle", "k".into(), "m".into(), vec![("a".into(), 0)]).is_err());
        assert!(Finding::new("cycle", "".into(), "m".into(), vec![("a".into(), 1)]).is_err());
        assert!(Finding::new("cycle", "k".into(), "m".into(), vec![("a".into(), 1)]).is_ok());
    }

    fn e(pairs: &[(&str, &str)]) -> BTreeSet<(String, String)> {
        pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn cycles_are_rotated_to_the_smallest_label_and_acyclic_graphs_have_none() {
        let (c, t) = find_cycles(&e(&[("b", "c"), ("c", "a"), ("a", "b"), ("d", "a")]));
        assert!(!t);
        assert_eq!(c, vec![vec!["a", "b", "c"]]);
        assert!(find_cycles(&e(&[("a", "b"), ("b", "c")])).0.is_empty());
        assert!(find_cycles(&e(&[("a", "a")])).0.is_empty());
    }

    #[test]
    fn a_new_edge_inside_a_component_always_makes_a_new_cycle() {
        let before = find_cycles(&e(&[("a", "b"), ("b", "a"), ("b", "c"), ("c", "b")])).0;
        let after = find_cycles(&e(&[("a", "b"), ("b", "a"), ("b", "c"), ("c", "b"), ("c", "a")])).0;
        assert!(after.iter().any(|c| !before.contains(c)), "{after:?}");
    }

    #[test]
    fn dead_imports_per_language() {
        let ts = "import a, { b, // note: from here\n  c as d, /* x */ type E } from './x';\nimport * as ns from './y';\nimport './side';\nconsole.log(a, d);\n";
        let got: Vec<String> = dead_imports_of("s.ts", ts).into_iter().map(|d| d.0).collect();
        assert_eq!(got, vec!["b", "E", "ns"]);
        let py = "import os.path\nimport sys as s\nfrom m import (x,\n   y as z)\nprint(os, z)\n";
        let got: Vec<String> = dead_imports_of("s.py", py).into_iter().map(|d| d.0).collect();
        assert_eq!(got, vec!["s", "x"]);
        let rs = "use std::io::{self, Write};\nuse a::{b, c::{d, e as f}};\npub use q::r;\nfn m() { io::stdout(); b(); }\n";
        let got: Vec<String> = dead_imports_of("s.rs", rs).into_iter().map(|d| d.0).collect();
        assert_eq!(got, vec!["d", "f"]);
    }

    #[test]
    fn globs() {
        assert!(glob_regex("apps/api/src/kernel/**").unwrap().is_match("apps/api/src/kernel/a/b.ts"));
        assert!(glob_regex("**/x.ts").unwrap().is_match("x.ts"));
        assert!(glob_regex("a/*.ts").unwrap().is_match("a/b.ts"));
        assert!(!glob_regex("a/*.ts").unwrap().is_match("a/b/c.ts"));
    }

    #[test]
    fn config_defaults_and_overrides() {
        let c = Config::parse("[providers.crg]\ncommand = \"x\"\n[health]\nmin_lines = 4 # dup\nmax_file_lines = 10\n[[health.boundary]]\nfrom = \"a/**\"\nnever = \"b/**\"\n").unwrap();
        assert_eq!((c.min_lines, c.max_file_lines, c.max_function_lines), (4, 10, 150));
        assert_eq!(c.boundaries.len(), 1);
        assert!(Config::parse("[health]\nbogus = 1\n").is_err());
        assert!(Config::parse("[[health.boundary]]\nfrom = \"a\"\n").is_err());
    }
}
