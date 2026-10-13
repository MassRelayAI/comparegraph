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
use std::collections::{BTreeMap, HashMap, HashSet};

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
impl Term {
    pub fn informative(&self) -> bool {
        self.idf > 0.0
    }
}

pub struct NodeInfo {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub qn: String,
    pub path: String,
    pub ls: i64,
    pub le: i64,
    ntoks: Vec<String>,
    ptoks: Vec<String>,
    flat: String,
    pub community: Option<usize>,
    pub callers: usize,
    is_test_path: bool,
}

pub struct RunIndex {
    pub run: i64,
    pub nodes: Vec<NodeInfo>,
    comm_toks: Vec<HashSet<String>>,
    comm_label: Vec<String>,
    name_df: HashMap<String, usize>,
    path_df: HashMap<String, usize>,
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
        let mut nodes: Vec<NodeInfo> = st
            .query_map(params![run], |r| {
                let name: String = r.get(2)?;
                let path: String = r.get(4)?;
                Ok(NodeInfo {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    ntoks: tokens(&name),
                    ptoks: tokens(&path),
                    flat: name.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect(),
                    is_test_path: is_test_path(&path),
                    name,
                    qn: r.get(3)?,
                    path,
                    ls: r.get(5)?,
                    le: r.get(6)?,
                    community: None,
                    callers: 0,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
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
        let (mut name_df, mut path_df) = (HashMap::new(), HashMap::new());
        for n in &nodes {
            for t in n.ntoks.iter().collect::<HashSet<_>>() {
                *name_df.entry(t.clone()).or_insert(0) += 1;
            }
            for t in n.ptoks.iter().collect::<HashSet<_>>() {
                *path_df.entry(t.clone()).or_insert(0) += 1;
            }
        }
        Ok(Self { run, nodes, comm_toks, comm_label, name_df, path_df })
    }

    fn share(&self, t: &str, compound: bool) -> f64 {
        let n = self.nodes.len().max(1) as f64;
        if compound {
            let c = self.nodes.iter().filter(|x| x.flat.contains(t)).count();
            return c as f64 / n;
        }
        let f = |m: &HashMap<String, usize>| m.iter().filter(|(k, _)| tok_match(t, k)).map(|(_, v)| *v).sum::<usize>() as f64 / n;
        f(&self.name_df).max(f(&self.path_df)).min(1.0)
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
                let share = self.share(&text, compound);
                // a term is a stop word only when it is both a large share and a large count: a three-node graph has no statistics
                let common = share > cutoff && share * n_nodes >= K_MIN_COMMON;
                let idf = if common || share == 0.0 { 0.0 } else { (1.0 / share).ln().max(0.1) };
                Term { text, compound, sw, idf, share }
            })
            .collect()
    }

    fn matched(&self, n: &NodeInfo, t: &Term) -> (bool, bool, bool) {
        let name = if t.compound { n.flat.contains(&t.text) } else { n.ntoks.iter().any(|k| tok_match(&t.text, k)) };
        let path = !t.compound && n.ptoks.iter().any(|k| tok_match(&t.text, k));
        let comm = !t.compound && n.community.is_some_and(|c| self.comm_toks[c].iter().any(|k| tok_match(&t.text, k)));
        (name, path, comm)
    }

    pub fn community_label(&self, i: usize) -> Option<String> {
        self.nodes[i].community.map(|c| self.comm_label[c].clone())
    }

    /// Ranked candidates: indices into `nodes` with their scores, best first, at most `limit`.
    pub fn rank(&self, f: &FeatureRow, terms: &[Term], limit: usize) -> Vec<(usize, f64)> {
        let (path_w, comm_w) = (K_PATH_W, K_COMM_W);
        let (graph_w, test_pen, file_pen) = (K_GRAPH_W, K_TEST_PEN, K_FILE_PEN);
        let ln_w = K_LN_W;
        let fkinds = feature_kinds(f);
        let about_tests = fkinds.contains(&"test") || terms.iter().any(|t| t.text.starts_with("test"));
        let active: Vec<&Term> = terms.iter().filter(|t| t.informative()).collect();
        let mut raw: Vec<f64> = vec![0.0; self.nodes.len()];
        for (i, n) in self.nodes.iter().enumerate() {
            let mut s = 0.0;
            for t in &active {
                let (nm, pa, cm) = self.matched(n, t);
                let ch = if nm { 1.0 } else if pa { path_w } else if cm { comm_w } else { 0.0 };
                s += t.idf * t.sw * ch;
            }
            if s <= 0.0 {
                continue;
            }
            let cov = if n.ntoks.is_empty() {
                0.0
            } else {
                n.ntoks.iter().filter(|k| active.iter().any(|t| !t.compound && tok_match(&t.text, k))).count() as f64 / n.ntoks.len() as f64
            };
            s *= ln_w + (1.0 - ln_w) * cov;
            let kp = kind_prior(&n.kind, &fkinds, n.is_test_path && !about_tests, test_pen, file_pen);
            raw[i] = s * kp * (1.0 + graph_w * (1.0 + n.callers as f64).ln());
        }
        let score = raw;
        let mut pool: Vec<(usize, f64)> = score.iter().enumerate().filter(|(_, s)| **s > 0.0).map(|(i, s)| (i, *s)).collect();
        pool.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(self.nodes[a.0].qn.cmp(&self.nodes[b.0].qn)).then(a.0.cmp(&b.0)));
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
