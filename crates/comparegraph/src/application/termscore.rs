//! Term quality and candidate scoring for `map` (and the informative-term filter of `absent-check`). Read-only over a run.
//!
//! Terms: tokens of the feature's group, slug tail and title (weight 1), the distinctive tokens of its definition (weight
//! `K_DEF_W`), and the slug's compound forms (`call-graph` -> `callgraph`). A term is INFORMATIVE when the share of the run's
//! nodes whose name tokens (or whose path tokens) carry it is at most `K_CUTOFF`; its weight is `ln(1/share)`. A term above the
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

// Parameters, chosen on the TUNE clones only (see docs/design-notes-for-review.md, "Plan 7 IMP").
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
    "own", "out", "see", "per", "not", "ones", "would", "will", "may", "was", "were", "been", "being", "kept", "kinds", "kind",
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
/// two of its chars (`extract`~`extraction`, `resolution`~`resolve`, `signature`~`signatures`).
pub fn tok_match(a: &str, b: &str) -> bool {
    if a == b {
        return true;
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
        let cutoff = K_CUTOFF;
        let n_nodes = self.nodes.len() as f64;
        let def_w = K_DEF_W;
        let slug_tail = f.slug.rsplit('.').next().unwrap_or(&f.slug);
        let mut raw: BTreeMap<String, (f64, bool)> = BTreeMap::new();
        for t in tokens(&f.grp).into_iter().chain(tokens(slug_tail)).chain(tokens(&f.title)) {
            raw.insert(t, (1.0, false));
        }
        let def = f.definition.split("[kinds:").next().unwrap_or("");
        for t in tokens(def) {
            if !ENGLISH.contains(&t.as_str()) {
                raw.entry(t).or_insert((def_w, false));
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
                let common = share > cutoff && share * n_nodes >= K_MIN_COMMON;
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
        let (path_w, comm_w) = (K_PATH_W, K_COMM_W);
        let (graph_w, test_pen, file_pen) = (K_GRAPH_W, K_TEST_PEN, K_FILE_PEN);
        let ln_w = K_LN_W;
        let fkinds = feature_kinds(f);
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
                let ch = if nm { 1.0 } else if pa { path_w } else if cm { comm_w } else { 0.0 };
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
            s *= ln_w + (1.0 - ln_w) * cov;
            let kp = kind_prior(&n.kind, &fkinds, n.is_test_path && !about_tests, test_pen, file_pen);
            let score = s * kp * (1.0 + graph_w * (1.0 + n.callers as f64).ln());
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

fn kind_prior(kind: &str, fkinds: &[&str], test_path: bool, test_pen: f64, file_pen: f64) -> f64 {
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
            if wants("file") || wants("config") { 0.9 } else { file_pen }
        }
        "test" => {
            if wants("test") { 1.0 } else { test_pen }
        }
        _ => 0.5,
    };
    if test_path { base * test_pen } else { base }
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
