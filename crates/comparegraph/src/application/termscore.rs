//! Term quality and candidate scoring for `map`. Read-only over a run.
//!
//! Terms: tokens of the feature's group, slug tail and title (weight 1), the distinctive tokens of its definition (weight
//! `K_DEF_W`), and the slug's compound forms (`call-graph` -> `callgraph`). A term is INFORMATIVE when its share is at most
//! `K_CUTOFF`, or the run has fewer than `K_MIN_COMMON` nodes carrying it. The share is the sum of the document frequencies of
//! the vocabulary tokens that `tok_match` the term, over the run's node count and capped at 1 (the larger of the name and path
//! channels); its weight is `ln(1/share)`, floored at 0.1. A term above the
//! cutoff (`code`, `graph`, `tool`, `file` in a code-graph tool) weighs 0. Nothing is hard-coded per word or per tool: the
//! weight comes from the run being searched.
//!
//! Score of a node = (sum over distinct terms of weight x source weight x best channel: name 1, path `K_PATH_W`, community
//! keywords `K_COMM_W`) x name coverage x kind prior x (1 + `K_GRAPH_W` x ln(1 + callers)). Top `limit`, ties by qualified name.
use super::communities::tokens;
use crate::store::{FeatureRow, Store};
use rusqlite::params;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

// Parameters, chosen on the TUNE clones only (see docs/decisions.md, "Plan 7 IMP").
/// a term carried by more than this share of the run's node names (or of its path tokens) weighs 0
const K_CUTOFF: f64 = 0.1;
/// fewest nodes that must carry a term before the cutoff applies to it
const K_MIN_COMMON: f64 = 8.0;
/// weight of a definition token against a group, slug or title token
const K_DEF_W: f64 = 0.5;
/// credit of a path-token match and of a community-keyword match, against 1 for a name-token match
const K_PATH_W: f64 = 0.35;
const K_COMM_W: f64 = 0.2;
/// kind and graph priors: a test node or a node under a test path, a file/module node when the feature wants no file; callers
const K_TEST_PEN: f64 = 0.15;
const K_FILE_PEN: f64 = 0.5;
const K_GRAPH_W: f64 = 0.3;
/// the score is scaled by `K_LN_W + (1 - K_LN_W) * (share of the node's name tokens that match a term)`
const K_LN_W: f64 = 0.5;

const ENGLISH: &[&str] = &[
    "the", "and", "for", "with", "from", "are", "its", "that", "which", "each", "not", "only", "than", "into", "can", "one",
    "more", "both", "beyond", "without", "other", "any", "all", "these", "those", "when", "where", "how", "also", "then",
    "their", "them", "they", "given", "such", "like", "uses", "used", "use", "via", "has", "have", "does", "but", "same",
    "own", "out", "see", "per", "ones", "would", "will", "may", "was", "were", "been", "being", "kept", "kinds", "kind",
];

#[derive(Clone, Debug)]
pub struct Term {
    pub text: String,
    pub compound: bool,
    /// source weight: 1 for group/slug/title, `K_DEF_W` for definition tokens
    pub sw: f64,
    /// ln(1/share), 0 when above the cutoff
    pub idf: f64,
    pub share: f64,
}
/// One ranked candidate.
pub struct Ranked {
    /// index into `RunIndex::nodes`
    pub i: usize,
    pub score: f64,
    /// the informative terms the node's NAME carries, in term order
    pub matched: Vec<String>,
    /// the name carries a term that is the feature's own (group, slug tail, title or the compound)
    pub named: bool,
}

impl Term {
    /// A term the feature is named by (its group, slug tail, title or compound), not a word of its definition.
    pub fn names_feature(&self) -> bool {
        self.sw >= 1.0
    }
    pub fn informative(&self) -> bool {
        self.idf > 0.0
    }
}

/// A file's interned path, its distinct path-token ids and whether it is a test path.
type PathInfo = (Rc<str>, Rc<[u32]>, bool);

pub struct NodeInfo {
    pub id: i64,
    pub kind: Rc<str>,
    pub qn: String,
    pub path: Rc<str>,
    pub ls: i64,
    pub le: i64,
    /// name tokens, as ids into `RunIndex::vocab`
    ntoks: Vec<u32>,
    /// distinct path tokens (shared by every node of the file)
    ptoks: Rc<[u32]>,
    flat: String,
    pub community: Option<usize>,
    pub callers: usize,
    is_test_path: bool,
}

pub struct RunIndex {
    pub run: i64,
    pub nodes: Vec<NodeInfo>,
    /// every distinct name/path token of the run; nodes hold ids into it
    vocab: Vec<String>,
    comm_toks: Vec<HashSet<String>>,
    comm_label: Vec<String>,
    /// token -> indices of the nodes (distinct, ascending) whose name tokens / path tokens carry it
    name_inv: HashMap<u32, Vec<usize>>,
    path_inv: HashMap<u32, Vec<usize>>,
    /// community index -> its member nodes (those whose first community it is)
    comm_nodes: Vec<Vec<usize>>,
    /// what each distinct term matches in this run, computed once and shared by every feature
    memo: RefCell<HashMap<(String, bool), Rc<TermHits>>>,
}

/// The run-wide facts about one term: its share and every node it can score on (name, path or community channel).
struct TermHits {
    share: f64,
    /// ascending node indices
    nodes: Vec<usize>,
    /// per community: does its token set carry the term
    comm: Vec<bool>,
}

/// Two tokens match when equal, or when they share a prefix of >= 4 chars that is the whole shorter token or leaves at most
/// two of its chars (`extract`~`extraction`, `resolution`~`resolve`, `signature`~`signatures`). A shorter token of fewer
/// than 6 chars must be equal instead, except that the longer may add one plural or verb suffix (`s`, `es`, `ed`, `ing`):
/// `task`~`tasks` matches, `multi`~`multiplier` does not.
pub fn tok_match(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (short, long) = if a.chars().count() <= b.chars().count() { (a, b) } else { (b, a) };
    if short.chars().count() < 6 {
        return ["s", "es", "ed", "ing"].iter().any(|suf| long.strip_prefix(short).is_some_and(|rest| rest == *suf));
    }
    let cp = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    let m = a.chars().count().min(b.chars().count());
    cp >= 4 && cp + 2 >= m
}

fn is_test_path(p: &str) -> bool {
    let l = p.to_lowercase();
    l.split('/').any(|s| s == "test" || s == "tests" || s == "__tests__" || s == "spec" || s == "specs" || s.starts_with("test_") || s.ends_with("_test.py") || s.contains(".test.") || s.contains(".spec.") || s.ends_with("_test.go") || s.ends_with("_test.rs") || s == "fixtures" || s == "testdata")
}

impl RunIndex {
    pub fn load(store: &Store, run: i64) -> super::Result<Self> {
        let c = store.conn();
        let mut st = c.prepare("SELECT id, kind, name, qualified_name, path, line_start, line_end FROM node WHERE run_id=?1 ORDER BY id")?;
        let mut vocab: Vec<String> = vec![];
        let mut vmap: HashMap<String, u32> = HashMap::new();
        let mut intern = |t: String| -> u32 {
            *vmap.entry(t.clone()).or_insert_with(|| {
                vocab.push(t);
                (vocab.len() - 1) as u32
            })
        };
        let mut kinds: HashMap<String, Rc<str>> = HashMap::new();
        let mut paths: HashMap<String, PathInfo> = HashMap::new();
        let mut nodes: Vec<NodeInfo> = vec![];
        let rows = st.query_map(params![run], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, i64>(5)?, r.get::<_, i64>(6)?))
        })?;
        for row in rows {
            let (id, kind, name, qn, path, ls, le) = row?;
            let kind = kinds.entry(kind.clone()).or_insert_with(|| kind.as_str().into()).clone();
            if !paths.contains_key(&path) {
                let mut pt: Vec<u32> = tokens(&path).into_iter().map(&mut intern).collect();
                pt.sort_unstable();
                pt.dedup();
                paths.insert(path.clone(), (path.as_str().into(), pt.into(), is_test_path(&path)));
            }
            let (pr, pt, tp) = paths[&path].clone();
            nodes.push(NodeInfo {
                id,
                kind,
                ntoks: tokens(&name).into_iter().map(&mut intern).collect(),
                ptoks: pt,
                flat: name.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect(),
                is_test_path: tp,
                qn,
                path: pr,
                ls,
                le,
                community: None,
                callers: 0,
            });
        }
        drop(st);
        let idx: HashMap<i64, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
        let comms = store.communities_of_run(crate::domain::RunId(run))?;
        let mut comm_toks = vec![];
        let mut comm_label = vec![];
        for (ci, cm) in comms.iter().enumerate() {
            let set: HashSet<String> = tokens(&cm.label).into_iter().chain(cm.keywords.iter().map(|k| k.to_lowercase())).collect();
            comm_toks.push(set);
            comm_label.push(cm.label.clone());
            for m in &cm.members {
                if let Some(&i) = idx.get(&m.0) {
                    if nodes[i].community.is_none() {
                        nodes[i].community = Some(ci);
                    }
                }
            }
        }
        let mut st = c.prepare("SELECT source_node_id, target_node_id FROM edge WHERE run_id=?1 AND kind='calls'")?;
        for r in st.query_map(params![run], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
            let (s, t) = r?;
            if let (Some(&a), Some(&b)) = (idx.get(&s), idx.get(&t)) {
                if a != b {
                    nodes[b].callers += 1;
                }
            }
        }
        let (mut name_inv, mut path_inv): (HashMap<u32, Vec<usize>>, HashMap<u32, Vec<usize>>) = (HashMap::new(), HashMap::new());
        let mut comm_nodes: Vec<Vec<usize>> = vec![vec![]; comm_toks.len()];
        for (i, n) in nodes.iter().enumerate() {
            for t in n.ntoks.iter().collect::<HashSet<_>>() {
                name_inv.entry(*t).or_default().push(i);
            }
            for t in n.ptoks.iter() {
                path_inv.entry(*t).or_default().push(i);
            }
            if let Some(c) = n.community {
                comm_nodes[c].push(i);
            }
        }
        Ok(Self { run, nodes, vocab, comm_toks, comm_label, name_inv, path_inv, comm_nodes, memo: RefCell::new(HashMap::new()) })
    }

    /// What term `t` matches in this run (memoised across features).
    fn hits(&self, t: &str, compound: bool) -> Rc<TermHits> {
        if let Some(h) = self.memo.borrow().get(&(t.to_string(), compound)) {
            return h.clone();
        }
        let n = self.nodes.len().max(1) as f64;
        let h = if compound {
            let nodes: Vec<usize> = (0..self.nodes.len()).filter(|&i| self.nodes[i].flat.contains(t)).collect();
            TermHits { share: nodes.len() as f64 / n, nodes, comm: vec![false; self.comm_toks.len()] }
        } else {
            let (mut sn, mut sp) = (0usize, 0usize);
            let mut nodes: Vec<usize> = vec![];
            for (_, v) in self.name_inv.iter().filter(|(k, _)| tok_match(t, &self.vocab[**k as usize])) {
                sn += v.len();
                nodes.extend_from_slice(v);
            }
            for (_, v) in self.path_inv.iter().filter(|(k, _)| tok_match(t, &self.vocab[**k as usize])) {
                sp += v.len();
                nodes.extend_from_slice(v);
            }
            let comm: Vec<bool> = self.comm_toks.iter().map(|set| set.iter().any(|k| tok_match(t, k))).collect();
            for (c, _) in comm.iter().enumerate().filter(|(_, m)| **m) {
                nodes.extend_from_slice(&self.comm_nodes[c]);
            }
            nodes.sort_unstable();
            nodes.dedup();
            TermHits { share: (sn as f64 / n).max(sp as f64 / n).min(1.0), nodes, comm }
        };
        let h = Rc::new(h);
        self.memo.borrow_mut().insert((t.to_string(), compound), h.clone());
        h
    }

    /// The feature's terms with their weights in this run.
    pub fn terms(&self, f: &FeatureRow) -> Vec<Term> {
        let n_nodes = self.nodes.len() as f64;
        let slug_tail = f.slug.rsplit('.').next().unwrap_or(&f.slug);
        let mut raw: BTreeMap<String, (f64, bool)> = BTreeMap::new();
        for t in tokens(&f.grp).into_iter().chain(tokens(slug_tail)).chain(tokens(&f.title)).chain(tokens(&f.synonyms)) {
            raw.insert(t, (1.0, false));
        }
        let def = f.definition.split("[kinds:").next().unwrap_or("");
        for t in tokens(def) {
            if !ENGLISH.contains(&t.as_str()) {
                raw.entry(t).or_insert((K_DEF_W, false));
            }
        }
        let parts: Vec<String> = slug_tail.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|p| !p.is_empty()).map(String::from).collect();
        if parts.len() > 1 {
            raw.insert(parts.concat(), (1.0, true));
        }
        raw.into_iter()
            .map(|(text, (sw, compound))| {
                let share = self.hits(&text, compound).share;
                // a term is a stop word only when it is both a large share and a large count: a three-node graph has no statistics
                let common = share > K_CUTOFF && share * n_nodes >= K_MIN_COMMON;
                let idf = if common || share == 0.0 { 0.0 } else { (1.0 / share).ln().max(0.1) };
                Term { text, compound, sw, idf, share }
            })
            .collect()
    }

    fn matched(&self, n: &NodeInfo, t: &Term, h: &TermHits) -> (bool, bool, bool) {
        let name = if t.compound { n.flat.contains(&t.text) } else { n.ntoks.iter().any(|&k| tok_match(&t.text, &self.vocab[k as usize])) };
        let path = !t.compound && n.ptoks.iter().any(|&k| tok_match(&t.text, &self.vocab[k as usize]));
        let comm = !t.compound && n.community.is_some_and(|c| h.comm[c]);
        (name, path, comm)
    }

    pub fn community_label(&self, i: usize) -> Option<String> {
        self.nodes[i].community.map(|c| self.comm_label[c].clone())
    }

    /// Every community of the run, in store order: its label and its token set (label tokens and lowercased keywords).
    pub fn communities(&self) -> impl Iterator<Item = (&str, &HashSet<String>)> {
        self.comm_label.iter().map(String::as_str).zip(self.comm_toks.iter())
    }

    /// Ranked candidates, best first, at most `limit`.
    pub fn rank(&self, f: &FeatureRow, terms: &[Term], limit: usize) -> Vec<Ranked> {
        let fkinds = feature_kinds(f);
        // `tokens` drops the word "test" itself, so the second clause fires only for "tests", "testing" and the like
        let about_tests = fkinds.contains(&"test") || terms.iter().any(|t| t.text.starts_with("test"));
        let active: Vec<&Term> = terms.iter().filter(|t| t.informative()).collect();
        // a node no active term can reach scores 0: only the union of the terms' hit lists is scored
        let hits: Vec<Rc<TermHits>> = active.iter().map(|t| self.hits(&t.text, t.compound)).collect();
        let mut cand: Vec<usize> = hits.iter().flat_map(|h| h.nodes.iter().copied()).collect();
        cand.sort_unstable();
        cand.dedup();
        let mut pool: Vec<Ranked> = vec![];
        for i in cand {
            let n = &self.nodes[i];
            let mut s = 0.0;
            let (mut by_name, mut named) = (vec![], false);
            for (t, h) in active.iter().zip(&hits) {
                let (nm, pa, cm) = self.matched(n, t, h);
                let ch = if nm { 1.0 } else if pa { K_PATH_W } else if cm { K_COMM_W } else { 0.0 };
                s += t.idf * t.sw * ch;
                if nm {
                    by_name.push(t.text.clone());
                    named |= t.names_feature();
                }
            }
            if s <= 0.0 {
                continue;
            }
            let cov = if n.ntoks.is_empty() {
                0.0
            } else {
                n.ntoks.iter().filter(|&&k| active.iter().any(|t| !t.compound && tok_match(&t.text, &self.vocab[k as usize]))).count() as f64 / n.ntoks.len() as f64
            };
            s *= K_LN_W + (1.0 - K_LN_W) * cov;
            let kp = kind_prior(&n.kind, &fkinds, n.is_test_path && !about_tests);
            let score = s * kp * (1.0 + K_GRAPH_W * (1.0 + n.callers as f64).ln());
            if score > 0.0 {
                pool.push(Ranked { i, score, matched: by_name, named });
            }
        }
        pool.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap().then(self.nodes[a.i].qn.cmp(&self.nodes[b.i].qn)).then(a.i.cmp(&b.i)));
        pool.truncate(limit);
        pool
    }
}

/// The node kinds a feature says implement it, from the `[kinds: ...]` suffix of its definition.
pub fn feature_kinds(f: &FeatureRow) -> Vec<&'static str> {
    let Some(tail) = f.definition.split("[kinds:").nth(1) else { return vec![] };
    let body = tail.split(']').next().unwrap_or("");
    let mut v = vec![];
    for k in body.split(',').map(str::trim) {
        let s = match k {
            "function" => "function",
            "method" => "method",
            "class" => "class",
            "test" => "test",
            "file" => "file",
            "config" => "config",
            "cli" => "cli",
            "mcp-tool" => "mcp-tool",
            _ => continue,
        };
        v.push(s);
    }
    v
}

/// Prior of a node's kind given the kinds the feature asks for; `K_TEST_PEN` for tests, `K_FILE_PEN` for files, 0.5 for any other kind.
fn kind_prior(kind: &str, fkinds: &[&str], test_path: bool) -> f64 {
    let wants = |k: &str| fkinds.contains(&k);
    let base = match kind {
        "function" | "method" => 1.0,
        "class" | "type" => {
            if wants("class") { 1.0 } else { 0.8 }
        }
        "route" => {
            if wants("mcp-tool") || wants("cli") { 1.0 } else { 0.7 }
        }
        "file" | "module" => {
            if wants("file") || wants("config") { 0.9 } else { K_FILE_PEN }
        }
        "test" => {
            if wants("test") { 1.0 } else { K_TEST_PEN }
        }
        _ => 0.5,
    };
    if test_path { base * K_TEST_PEN } else { base }
}

#[cfg(test)]
impl RunIndex {
    /// An index over plain `(kind, name, path)` rows, without communities or edges.
    pub(crate) fn from_rows(rows: &[(&str, &str, &str)]) -> Self {
        let mut vocab: Vec<String> = vec![];
        let mut vmap: HashMap<String, u32> = HashMap::new();
        let mut intern = |t: String| -> u32 {
            *vmap.entry(t.clone()).or_insert_with(|| {
                vocab.push(t);
                (vocab.len() - 1) as u32
            })
        };
        let mut nodes = vec![];
        for (i, (kind, name, path)) in rows.iter().enumerate() {
            let mut pt: Vec<u32> = tokens(path).into_iter().map(&mut intern).collect();
            pt.sort_unstable();
            pt.dedup();
            nodes.push(NodeInfo {
                id: i as i64,
                kind: (*kind).into(),
                qn: format!("{path}::{name}"),
                path: (*path).into(),
                ls: 1,
                le: 2,
                ntoks: tokens(name).into_iter().map(&mut intern).collect(),
                ptoks: pt.into(),
                flat: name.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect(),
                community: None,
                callers: 0,
                is_test_path: is_test_path(path),
            });
        }
        let (mut name_inv, mut path_inv): (HashMap<u32, Vec<usize>>, HashMap<u32, Vec<usize>>) = (HashMap::new(), HashMap::new());
        for (i, n) in nodes.iter().enumerate() {
            for t in n.ntoks.iter().collect::<HashSet<_>>() {
                name_inv.entry(*t).or_default().push(i);
            }
            for t in n.ptoks.iter() {
                path_inv.entry(*t).or_default().push(i);
            }
        }
        Self { run: 1, nodes, vocab, comm_toks: vec![], comm_label: vec![], name_inv, path_inv, comm_nodes: vec![], memo: RefCell::new(HashMap::new()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feat(grp: &str, slug: &str, title: &str, definition: &str) -> FeatureRow {
        FeatureRow {
            id: 1,
            family: "f".into(),
            grp: grp.into(),
            slug: slug.into(),
            title: title.into(),
            definition: definition.into(),
            counts_as_has: "".into(),
            tier: "core".into(),
            weight: 1,
            synonyms: "".into(),
        }
    }
    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }
    fn term<'a>(ts: &'a [Term], text: &str) -> &'a Term {
        ts.iter().find(|t| t.text == text).unwrap_or_else(|| panic!("no term {text} in {ts:?}"))
    }

    #[test]
    fn tok_match_cases() {
        for (a, b, want) in [
            ("extract", "extraction", true),
            ("resolution", "resolve", true),
            ("signature", "signatures", true),
            ("call", "callback", false), // short token: whole-token only (was the documented over-match)
            ("data", "database", false),
            ("cat", "cats", true), // plural suffix
            ("task", "tasks", true),
            ("task", "tasked", true),
            ("cach", "caching", true),
            ("class", "classes", true),
            ("multi", "multiplier", false),
            ("multi", "multilingual", false),
            ("lang", "language", false),
            ("abc", "abd", false),
            ("token", "tokio", false),
            ("graph", "grain", false),
            ("graph", "graphql", false),
            ("größen", "größens", true), // chars, not bytes
        ] {
            assert_eq!(tok_match(a, b), want, "{a} ~ {b}");
            assert_eq!(tok_match(b, a), want, "{b} ~ {a}");
        }
    }

    #[test]
    fn feature_kinds_cases() {
        let k = |d: &str| feature_kinds(&feat("g", "g.s", "t", d));
        assert_eq!(k("x [kinds: function, mcp-tool]"), vec!["function", "mcp-tool"]);
        assert!(k("no suffix").is_empty());
        assert_eq!(k("[kinds: bogus, test]"), vec!["test"]);
        assert!(k("[kinds:]").is_empty());
        assert_eq!(k("[kinds: cli"), vec!["cli"]);
    }

    #[test]
    fn kind_prior_cases() {
        for (kind, wants, test_path, want) in [
            ("test", vec![], false, 0.15),
            ("test", vec!["test"], false, 1.0),
            ("file", vec![], false, 0.5),
            ("file", vec!["config"], false, 0.9),
            ("class", vec![], false, 0.8),
            ("class", vec!["class"], false, 1.0),
            ("type", vec![], false, 0.8),
            ("route", vec![], false, 0.7),
            ("route", vec!["cli"], false, 1.0),
            ("function", vec![], true, 0.15),
            ("test", vec![], true, 0.0225),
            ("enum", vec![], false, 0.5),
        ] {
            let got = kind_prior(kind, &wants, test_path);
            assert!(near(got, want), "{kind} {wants:?} test_path={test_path}: {got} != {want}");
        }
    }

    #[test]
    fn small_run_has_no_stop_words() {
        let idx = RunIndex::from_rows(&[("function", "alpha_beta", "a.ts"), ("function", "alpha_gamma", "b.ts"), ("function", "delta", "c.ts")]);
        let ts = idx.terms(&feat("zz", "zz.zz", "alpha delta", ""));
        let (alpha, delta) = (term(&ts, "alpha"), term(&ts, "delta"));
        assert!(near(alpha.share, 2.0 / 3.0));
        assert!(alpha.informative(), "2 nodes < K_MIN_COMMON: a 3-node graph has no statistics");
        assert!(near(alpha.idf, 1.5f64.ln().max(0.1)));
        assert!(near(delta.idf, 3f64.ln()));
    }

    fn run_with(n: usize, carrying: usize, word: &str) -> RunIndex {
        let names: Vec<String> = (0..n).map(|i| if i < carrying { format!("{word}_zq{i}") } else { format!("item_zq{i}") }).collect();
        let rows: Vec<(&str, &str, &str)> = names.iter().map(|s| ("function", s.as_str(), "a.ts")).collect();
        RunIndex::from_rows(&rows)
    }

    #[test]
    fn common_word_weighs_zero_rare_word_weighs_ln() {
        let common = run_with(100, 20, "graph");
        let ts = common.terms(&feat("zz", "zz.zz", "graph", ""));
        assert!(near(term(&ts, "graph").share, 0.2));
        assert!(near(term(&ts, "graph").idf, 0.0));
        assert!(!term(&ts, "graph").informative());
        let rare = run_with(100, 5, "foo");
        let ts = rare.terms(&feat("zz", "zz.zz", "foo", ""));
        assert!(near(term(&ts, "foo").idf, 20f64.ln()));
        let ts = rare.terms(&feat("zz", "zz.zz", "absentword", ""));
        assert!(near(term(&ts, "absentword").idf, 0.0));
        assert!(!term(&ts, "absentword").informative());
    }

    #[test]
    fn slug_compound_and_definition_terms() {
        let idx = run_with(12, 1, "foo");
        let ts = idx.terms(&feat("graph", "graph.call-graph", "", ""));
        let c = term(&ts, "callgraph");
        assert!(c.compound && near(c.sw, 1.0));
        let ts = idx.terms(&feat("zz", "zz.zz", "", "the foo [kinds: function]"));
        let foo = term(&ts, "foo");
        assert!(near(foo.sw, 0.5) && !foo.compound);
        assert!(ts.iter().all(|t| t.text != "the" && t.text != "kinds" && t.text != "function"), "{ts:?}");
    }

    #[test]
    fn rank_ties_break_by_qualified_name_and_are_stable() {
        let mut rows = vec![("function", "widget", "src/b.ts"), ("function", "widget", "src/a.ts")];
        rows.extend((0..20).map(|_| ("function", "filler", "src/z.ts")));
        let idx = RunIndex::from_rows(&rows);
        let f = feat("zz", "zz.zz", "widget", "");
        let ts = idx.terms(&f);
        let (r1, r2) = (idx.rank(&f, &ts, 5), idx.rank(&f, &ts, 5));
        assert_eq!(r1.len(), 2);
        assert!(near(r1[0].score, r1[1].score));
        assert_eq!(idx.nodes[r1[0].i].qn, "src/a.ts::widget");
        assert_eq!(idx.nodes[r1[1].i].qn, "src/b.ts::widget");
        assert_eq!(r1.iter().map(|r| r.i).collect::<Vec<_>>(), r2.iter().map(|r| r.i).collect::<Vec<_>>());
    }

    #[test]
    fn rank_is_empty_when_every_term_is_common() {
        let idx = run_with(100, 20, "graph");
        let f = feat("zz", "zz.zz", "graph", "");
        assert!(idx.rank(&f, &idx.terms(&f), 5).is_empty());
    }

    #[test]
    fn rank_on_an_empty_run_is_empty() {
        let idx = RunIndex::from_rows(&[]);
        let f = feat("graph", "graph.call-graph", "Call graph", "the thing");
        assert!(idx.rank(&f, &idx.terms(&f), 5).is_empty());
    }

    #[test]
    fn a_node_with_an_empty_name_scores_finite() {
        let mut rows = vec![("function", "", "src/widget/x.ts")];
        rows.extend((0..11).map(|_| ("function", "filler", "src/y.ts")));
        let idx = RunIndex::from_rows(&rows);
        let f = feat("zz", "zz.zz", "widget", "");
        let r = idx.rank(&f, &idx.terms(&f), 5);
        assert_eq!(r.len(), 1);
        assert!(r[0].score.is_finite());
        // path channel 0.35, coverage 0 -> K_LN_W, function prior 1, no callers
        assert!(near(r[0].score, 12f64.ln() * K_PATH_W * K_LN_W), "{}", r[0].score);
    }
}
