//! Graph queries over one run's stored edges: traverse, impact, tests-for, entry points, dead-code candidates.
//! Adjacency is read lazily from SQLite, one batch per breadth-first frontier (`json_each` id sets), so a query touches
//! only the nodes and edges it reaches, never the whole run; each node is visited once, bounded by depth and max-nodes.
//! Every answer is a JSON object; `evidence_confidence` and `limitation_items` are lifted into the envelope by the CLI layer.
use super::{AppError, Result};
use crate::domain::{EdgeKind, Limitation, Node, NodeKind};
use crate::providers::reference::is_test_path;
use crate::store::Store;
use serde_json::{json, Value};
use rusqlite::params;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

pub const MAX_DEPTH: usize = 10;
pub const MAX_NODES: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Callers,
    Callees,
    Both,
}
impl Direction {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "callers" => Ok(Self::Callers),
            "callees" => Ok(Self::Callees),
            "both" => Ok(Self::Both),
            d => Err(AppError::InvalidArguments(format!(
                "unknown direction {d} (callers|callees|both)"
            ))),
        }
    }
}

pub fn check_bounds(depth: usize, max_nodes: usize, min_conf: f64) -> Result<()> {
    if !(1..=MAX_DEPTH).contains(&depth) {
        return Err(AppError::InvalidArguments(format!(
            "depth must be in 1..{MAX_DEPTH}"
        )));
    }
    if !(1..=MAX_NODES).contains(&max_nodes) {
        return Err(AppError::InvalidArguments(format!(
            "max-nodes must be in 1..{MAX_NODES}"
        )));
    }
    check_conf(min_conf)
}
pub fn check_conf(min_conf: f64) -> Result<()> {
    if min_conf == 0.5 || min_conf == 1.0 {
        Ok(())
    } else {
        Err(AppError::InvalidArguments(
            "min-confidence must be 0.5 or 1.0".into(),
        ))
    }
}
pub fn parse_kinds(s: &str) -> Result<Vec<EdgeKind>> {
    let mut v = vec![];
    for k in s.split(',').map(str::trim).filter(|k| !k.is_empty()) {
        match EdgeKind::parse(k) {
            Some(
                e @ (EdgeKind::Calls
                | EdgeKind::Extends
                | EdgeKind::Implements
                | EdgeKind::Imports
                | EdgeKind::Tests),
            ) => v.push(e),
            _ => {
                return Err(AppError::InvalidArguments(format!(
                    "unknown edge kind {k} (calls|extends|implements|imports|tests)"
                )))
            }
        }
    }
    if v.is_empty() {
        return Err(AppError::InvalidArguments("kinds must not be empty".into()));
    }
    Ok(v)
}

type Adj = Vec<(i64, EdgeKind, f64)>;

/// One run, read on demand. Nodes, adjacency and community labels are cached as they are first needed.
struct Ctx {
    store: Store,
    run_id: i64,
    repo: String,
    nodes: HashMap<i64, Node>,
    /// raw adjacency per (node, outgoing?) for the edge kinds of the current walk, parallel edges collapsed to the best
    /// confidence, unsorted; cleared when the walk's kinds change
    adj: HashMap<(i64, bool), Adj>,
    adj_kinds: Vec<EdgeKind>,
    adj_min_conf: f64,
    community: HashMap<i64, String>,
}

fn ids_json(ids: &[i64]) -> String {
    serde_json::to_string(ids).unwrap()
}
fn node_from_row(r: &rusqlite::Row) -> rusqlite::Result<Node> {
    Ok(Node {
        id: Some(crate::domain::NodeId(r.get(0)?)),
        kind: NodeKind::parse(&r.get::<_, String>(1)?).unwrap_or(NodeKind::Function),
        name: r.get(2)?,
        qualified_name: r.get(3)?,
        path: r.get(4)?,
        line_start: r.get(5)?,
        line_end: r.get(6)?,
        content_hash: r.get(7)?,
        language: r.get(8)?,
        provider_ref: r.get(9)?,
    })
}
const NODE_COLS: &str = "id,kind,name,qualified_name,path,line_start,line_end,content_hash,language,provider_ref";

impl Ctx {
    fn open(root: &Path, repo: &str) -> Result<Ctx> {
        let store = Store::open_existing(root)?;
        let r = store
            .repo_by_name(repo)?
            .ok_or_else(|| AppError::NotFound(format!("no repo named {repo}")))?;
        let run = store
            .latest_done_run(r.id.unwrap())?
            .ok_or_else(|| AppError::NotFound(format!("{repo} has no finished run")))?;
        let run_id = run.id.unwrap().0;
        Ok(Ctx {
            store,
            run_id,
            repo: r.name,
            nodes: HashMap::new(),
            adj: HashMap::new(),
            adj_kinds: vec![],
            adj_min_conf: 2.0,
            community: HashMap::new(),
        })
    }

    /// The symbol with this qualified name (the last in path/line order when several share it).
    fn node(&mut self, qn: &str) -> Result<i64> {
        let id: Option<i64> = {
            let c = self.store.conn();
            let mut st = c.prepare_cached(
                "SELECT id FROM node WHERE run_id=?1 AND qualified_name=?2 ORDER BY path DESC, line_start DESC, kind DESC LIMIT 1",
            )?;
            let mut rows = st.query(params![self.run_id, qn])?;
            match rows.next()? {
                Some(r) => Some(r.get(0)?),
                None => None,
            }
        };
        let id = id.ok_or_else(|| AppError::NotFound(format!("no symbol {qn} in {}", self.repo)))?;
        self.ensure_nodes(&[id])?;
        Ok(id)
    }

    fn ensure_nodes(&mut self, ids: &[i64]) -> Result<()> {
        let missing: Vec<i64> = ids.iter().copied().filter(|i| !self.nodes.contains_key(i)).collect();
        if missing.is_empty() {
            return Ok(());
        }
        let got: Vec<Node> = {
            let c = self.store.conn();
            let mut st = c.prepare(&format!(
                "SELECT {NODE_COLS} FROM node WHERE id IN (SELECT value FROM json_each(?1))"
            ))?;
            let rows = st.query_map(params![ids_json(&missing)], node_from_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for n in got {
            self.nodes.insert(n.id.unwrap().0, n);
        }
        Ok(())
    }

    /// Qualified names of `ids` (cached rows first, one light query for the rest): enough to order neighbours without loading rows.
    fn names_of(&self, ids: &[i64]) -> Result<HashMap<i64, String>> {
        let mut out: HashMap<i64, String> = HashMap::new();
        let mut missing: Vec<i64> = vec![];
        for i in ids {
            match self.nodes.get(i) {
                Some(n) => {
                    out.insert(*i, n.qualified_name.clone());
                }
                None => missing.push(*i),
            }
        }
        if !missing.is_empty() {
            let c = self.store.conn();
            let mut st = c.prepare("SELECT id, qualified_name FROM node WHERE id IN (SELECT value FROM json_each(?1))")?;
            let rows = st.query_map(params![ids_json(&missing)], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (i, n) = row?;
                out.insert(i, n);
            }
        }
        Ok(out)
    }

    fn ensure_communities(&mut self, ids: &[i64]) -> Result<()> {
        let missing: Vec<i64> = ids.iter().copied().filter(|i| !self.community.contains_key(i)).collect();
        if missing.is_empty() {
            return Ok(());
        }
        let got: Vec<(i64, String)> = {
            let c = self.store.conn();
            let mut st = c.prepare(
                "SELECT x.node_id, cm.label FROM community_member x JOIN community cm ON cm.id = x.community_id WHERE cm.run_id = ?1 AND x.node_id IN (SELECT value FROM json_each(?2))",
            )?;
            let rows = st.query_map(params![self.run_id, ids_json(&missing)], |x| {
                Ok((x.get::<_, i64>(0)?, x.get::<_, String>(1)?))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (n, l) in got {
            self.community.insert(n, l);
        }
        Ok(())
    }

    /// Reads the raw adjacency of `ids` in one edge scan (the edge table is indexed by run only, so a frontier costs one
    /// pass however many ids it holds).
    fn prefetch(&mut self, ids: &[i64], outgoing: bool, kinds: &[EdgeKind], min_conf: f64) -> Result<()> {
        if self.adj_kinds != kinds || self.adj_min_conf != min_conf {
            self.adj.clear();
            self.adj_kinds = kinds.to_vec();
            self.adj_min_conf = min_conf;
        }
        let missing: Vec<i64> = ids
            .iter()
            .copied()
            .filter(|i| !self.adj.contains_key(&(*i, outgoing)))
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let (key, other) = if outgoing {
            ("source_node_id", "target_node_id")
        } else {
            ("target_node_id", "source_node_id")
        };
        let kind_list = kinds
            .iter()
            .map(|k| format!("'{}'", k.as_str()))
            .collect::<Vec<_>>()
            .join(",");
        let mut best: HashMap<(i64, EdgeKind, i64), f64> = HashMap::new();
        {
            let c = self.store.conn();
            let mut st = c.prepare(&format!(
                "SELECT {key}, kind, {other}, confidence FROM edge WHERE run_id=?1 AND confidence>=?2 AND kind IN ({kind_list}) AND {key} IN (SELECT value FROM json_each(?3))"
            ))?;
            let rows = st.query_map(params![self.run_id, min_conf, ids_json(&missing)], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    EdgeKind::parse(&r.get::<_, String>(1)?).unwrap_or(EdgeKind::Calls),
                    r.get::<_, i64>(2)?,
                    r.get::<_, f64>(3)?,
                ))
            })?;
            for row in rows {
                let (a, k, b, conf) = row?;
                let e = best.entry((a, k, b)).or_insert(conf);
                if conf > *e {
                    *e = conf;
                }
            }
        }
        for id in &missing {
            self.adj.insert((*id, outgoing), vec![]);
        }
        for ((a, k, b), conf) in best {
            self.adj.get_mut(&(a, outgoing)).unwrap().push((b, k, conf));
        }
        Ok(())
    }

    /// `tests` edges into each of `ids` (confidence >= `min_conf`, parallel edges collapsed), sources in qualified-name order.
    fn tests_into(&mut self, ids: &[i64], min_conf: f64) -> Result<HashMap<i64, Vec<(i64, f64)>>> {
        let mut best: HashMap<(i64, i64), f64> = HashMap::new();
        if !ids.is_empty() {
            let c = self.store.conn();
            let mut st = c.prepare(
                "SELECT target_node_id, source_node_id, confidence FROM edge WHERE run_id=?1 AND confidence>=?2 AND kind='tests' AND target_node_id IN (SELECT value FROM json_each(?3))",
            )?;
            let rows = st.query_map(params![self.run_id, min_conf, ids_json(ids)], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, f64>(2)?))
            })?;
            for row in rows {
                let (t, s, conf) = row?;
                let e = best.entry((t, s)).or_insert(conf);
                if conf > *e {
                    *e = conf;
                }
            }
        }
        let srcs: Vec<i64> = best.keys().map(|(_, s)| *s).collect();
        self.ensure_nodes(&srcs)?;
        let mut out: HashMap<i64, Vec<(i64, f64)>> = HashMap::new();
        for ((t, s), conf) in best {
            out.entry(t).or_default().push((s, conf));
        }
        for v in out.values_mut() {
            v.sort_by(|a, b| {
                self.nodes[&a.0]
                    .qualified_name
                    .cmp(&self.nodes[&b.0].qualified_name)
                    .then(a.0.cmp(&b.0))
            });
        }
        Ok(out)
    }

    fn hit(&self, v: &[Reached], i: usize) -> Value {
        let r = &v[i];
        let n = &self.nodes[&r.id];
        let mut via = vec![];
        let mut cur = Some(i);
        while let Some(j) = cur {
            via.push(self.nodes[&v[j].id].qualified_name.clone());
            cur = v[j].parent;
        }
        via.reverse();
        json!({ "qualified_name": n.qualified_name, "kind": n.kind.as_str(), "path": n.path,
                "line_start": n.line_start, "hop": r.hop, "path_confidence": r.conf,
                "via": via, "community": self.community.get(&r.id) })
    }
    /// Loads what `hit` reads for every reached node.
    fn load_reached(&mut self, v: &[Reached]) -> Result<()> {
        let ids: Vec<i64> = v.iter().map(|r| r.id).collect();
        self.ensure_nodes(&ids)?;
        self.ensure_communities(&ids)
    }
}

struct Reached {
    id: i64,
    hop: usize,
    conf: f64,
    parent: Option<usize>,
}

/// Breadth-first walk from `starts`; each node once; stops adding at `max_nodes` (starts not counted). Returns the visit list
/// (starts first, hop 0) and whether it was cut short.
fn bfs(
    c: &mut Ctx,
    starts: &[i64],
    dir: Direction,
    kinds: &[EdgeKind],
    min_conf: f64,
    depth: usize,
    max_nodes: usize,
) -> Result<(Vec<Reached>, bool)> {
    let mut seen: HashSet<i64> = HashSet::new();
    let mut v: Vec<Reached> = vec![];
    let mut q: VecDeque<usize> = VecDeque::new();
    for s in starts {
        if seen.insert(*s) {
            v.push(Reached {
                id: *s,
                hop: 0,
                conf: 1.0,
                parent: None,
            });
            q.push_back(v.len() - 1);
        }
    }
    let n_starts = v.len();
    let w = Walk { dir, kinds, depth, max_nodes, floor: min_conf };
    // best-confidence paths: exhaust the 1.0 edges first, then extend the frontier with the 0.5 edges (a node keeps the
    // 1.0 path even when a shorter 0.5 path exists)
    if min_conf < 1.0 {
        if expand(c, &mut v, &mut seen, q.clone(), n_starts, &w, 1.0)? {
            return Ok((v, true));
        }
        q = (0..v.len()).collect();
    }
    let cut = expand(c, &mut v, &mut seen, q, n_starts, &w, min_conf)?;
    Ok((v, cut))
}

struct Walk<'a> {
    dir: Direction,
    kinds: &'a [EdgeKind],
    depth: usize,
    max_nodes: usize,
    /// the lowest confidence any pass of this walk reads: adjacency is fetched at this floor and cached across passes
    floor: f64,
}

fn expand(
    c: &mut Ctx,
    v: &mut Vec<Reached>,
    seen: &mut HashSet<i64>,
    mut q: VecDeque<usize>,
    n_starts: usize,
    w: &Walk,
    min_conf: f64,
) -> Result<bool> {
    let want_out = matches!(w.dir, Direction::Callees | Direction::Both);
    let want_in = matches!(w.dir, Direction::Callers | Direction::Both);
    while let Some(i) = q.pop_front() {
        let (id, hop, conf) = (v[i].id, v[i].hop, v[i].conf);
        if hop >= w.depth {
            continue;
        }
        // one batch per frontier: this node and everything still queued that will be expanded and has no adjacency yet
        for (want, outgoing) in [(want_out, true), (want_in, false)] {
            if want && !c.adj.contains_key(&(id, outgoing)) {
                let mut batch: Vec<i64> = vec![id];
                batch.extend(
                    q.iter()
                        .filter(|j| v[**j].hop < w.depth)
                        .map(|j| v[*j].id)
                        .filter(|x| !c.adj.contains_key(&(*x, outgoing))),
                );
                c.prefetch(&batch, outgoing, w.kinds, w.floor)?;
            }
        }
        // neighbours passing the filters, each direction sorted by (qualified name, kind, id) as the whole-run walk did;
        // only the first `budget + 1` distinct neighbours can be taken before the cut, so only those node rows are loaded
        let budget = w.max_nodes.saturating_sub(v.len() - n_starts) + 1;
        let mut next: Vec<(i64, EdgeKind, f64)> = vec![];
        let mut taken: HashSet<i64> = HashSet::new();
        for (want, outgoing) in [(want_out, true), (want_in, false)] {
            if !want {
                continue;
            }
            let mut part: Vec<(i64, EdgeKind, f64)> = c.adj[&(id, outgoing)]
                .iter()
                .filter(|(t, k, ec)| w.kinds.contains(k) && *ec >= min_conf && !seen.contains(t) && !taken.contains(t))
                .copied()
                .collect();
            let ids: Vec<i64> = part.iter().map(|p| p.0).collect();
            let names = c.names_of(&ids)?;
            part.sort_by(|a, b| {
                names[&a.0]
                    .cmp(&names[&b.0])
                    .then(a.1.as_str().cmp(b.1.as_str()))
                    .then(a.0.cmp(&b.0))
            });
            let mut once: HashSet<i64> = HashSet::new();
            part.retain(|p| once.insert(p.0));
            part.truncate(budget);
            let keep: Vec<i64> = part.iter().map(|p| p.0).collect();
            c.ensure_nodes(&keep)?;
            taken.extend(keep);
            next.extend(part);
        }
        for (t, _k, ec) in next {
            if seen.contains(&t) {
                continue;
            }
            if v.len() - n_starts >= w.max_nodes {
                return Ok(true);
            }
            seen.insert(t);
            v.push(Reached {
                id: t,
                hop: hop + 1,
                conf: conf.min(ec),
                parent: Some(i),
            });
            q.push_back(v.len() - 1);
        }
    }
    Ok(false)
}

fn limitations(truncated: bool, max_nodes: usize, guessed: usize) -> Vec<Limitation> {
    let mut l = vec![];
    if truncated {
        l.push(Limitation::new(
            "truncated",
            format!("stopped at max-nodes {max_nodes}; the answer is a prefix in breadth-first order (raise --max-nodes, at most {MAX_NODES})"),
        ));
    }
    if guessed > 0 {
        l.push(Limitation::new(
            "name_guessed_hops",
            format!("{guessed} returned node(s) were reached through at least one name-guessed edge (confidence 0.5); pass --min-confidence 1.0 to drop them"),
        ));
    }
    l
}
fn answer(mut data: Value, guessed: usize, mut lims: Vec<Limitation>, base: &str) -> Value {
    let conf = if base.is_empty() {
        if guessed == 0 {
            "derived"
        } else {
            "observed"
        }
    } else {
        base
    };
    let o = data.as_object_mut().unwrap();
    o.insert("evidence_confidence".into(), json!(conf));
    o.insert(
        "limitation_items".into(),
        serde_json::to_value(std::mem::take(&mut lims)).unwrap(),
    );
    data
}

pub struct TraverseOpts<'a> {
    pub qualified_name: &'a str,
    pub direction: Direction,
    pub depth: usize,
    pub max_nodes: usize,
    pub kinds: Vec<EdgeKind>,
    pub min_conf: f64,
}

pub fn traverse(root: &Path, repo: &str, o: &TraverseOpts) -> Result<Value> {
    check_bounds(o.depth, o.max_nodes, o.min_conf)?;
    let mut c = Ctx::open(root, repo)?;
    let start = c.node(o.qualified_name)?;
    let (v, truncated) = bfs(&mut c, &[start], o.direction, &o.kinds, o.min_conf, o.depth, o.max_nodes)?;
    c.load_reached(&v)?;
    let nodes: Vec<Value> = (1..v.len()).map(|i| c.hit(&v, i)).collect();
    let guessed = v.iter().skip(1).filter(|r| r.conf < 1.0).count();
    let total = nodes.len();
    Ok(answer(
        json!({ "repo": c.repo, "start": o.qualified_name, "items": nodes, "total": total, "truncated": truncated }),
        guessed,
        limitations(truncated, o.max_nodes, guessed),
        "",
    ))
}

pub struct ImpactOpts<'a> {
    pub qualified_name: Option<&'a str>,
    pub files: &'a [String],
    pub depth: usize,
    pub max_nodes: usize,
    pub min_conf: f64,
}

pub fn impact(root: &Path, repo: &str, o: &ImpactOpts) -> Result<Value> {
    check_bounds(o.depth, o.max_nodes, o.min_conf)?;
    if o.qualified_name.is_none() && o.files.is_empty() {
        return Err(AppError::InvalidArguments(
            "give a qualified name or at least one --file".into(),
        ));
    }
    let mut c = Ctx::open(root, repo)?;
    let mut starts: Vec<i64> = vec![];
    if let Some(qn) = o.qualified_name {
        starts.push(c.node(qn)?);
    }
    for f in o.files {
        let mut ids: Vec<i64> = {
            let conn = c.store.conn();
            let mut st = conn.prepare_cached("SELECT id FROM node WHERE run_id=?1 AND path=?2")?;
            let rows = st.query_map(params![c.run_id, f], |r| r.get::<_, i64>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        if ids.is_empty() {
            return Err(AppError::NotFound(format!("no file {f} in {}", c.repo)));
        }
        c.ensure_nodes(&ids)?;
        ids.sort_by(|a, b| c.nodes[a].qualified_name.cmp(&c.nodes[b].qualified_name).then(a.cmp(b)));
        starts.extend(ids);
    }
    let kinds = [
        EdgeKind::Calls,
        EdgeKind::Extends,
        EdgeKind::Implements,
        EdgeKind::Imports,
    ];
    let (v, truncated) = bfs(&mut c, &starts, Direction::Callers, &kinds, o.min_conf, o.depth, o.max_nodes)?;
    c.load_reached(&v)?;
    let n_starts = v.iter().take_while(|r| r.hop == 0).count();
    let mut by_hop: Vec<Value> = vec![];
    let mut hop_no = 1;
    loop {
        let items: Vec<Value> = (n_starts..v.len())
            .filter(|i| v[*i].hop == hop_no)
            .map(|i| c.hit(&v, i))
            .collect();
        if items.is_empty() {
            break;
        }
        by_hop.push(json!({ "hop": hop_no, "nodes": items }));
        hop_no += 1;
    }
    // related tests: test-path nodes among the impacted, and sources of `tests` edges into the target or an impacted node
    let reached: Vec<i64> = v.iter().map(|r| r.id).collect();
    let into = c.tests_into(&reached, o.min_conf)?;
    let mut tests: HashMap<String, Value> = HashMap::new();
    let mut add_test = |id: i64, conf: f64, link: &str| {
        let n = &c.nodes[&id];
        let e = tests.entry(n.qualified_name.clone()).or_insert_with(|| {
            json!({ "qualified_name": n.qualified_name, "kind": n.kind.as_str(), "path": n.path,
                    "path_confidence": conf, "link": link })
        });
        if conf > e["path_confidence"].as_f64().unwrap_or(0.0) {
            e["path_confidence"] = json!(conf);
            e["link"] = json!(link);
        }
    };
    for (i, r) in v.iter().enumerate() {
        let n = &c.nodes[&r.id];
        if i >= n_starts && is_test_path(&n.path) {
            add_test(r.id, r.conf, "impacted");
        }
        for (s, ec) in into.get(&r.id).map(Vec::as_slice).unwrap_or(&[]) {
            add_test(*s, r.conf.min(*ec), "tests_edge");
        }
    }
    let mut tests: Vec<Value> = tests.into_values().collect();
    tests.sort_by(|a, b| a["qualified_name"].as_str().cmp(&b["qualified_name"].as_str()));
    let guessed = v.iter().skip(n_starts).filter(|r| r.conf < 1.0).count();
    let total = v.len() - n_starts;
    Ok(answer(
        json!({ "repo": c.repo, "targets": starts.iter().take(n_starts).map(|i| c.nodes[i].qualified_name.clone()).collect::<Vec<_>>(),
                "total": total, "by_hop": by_hop, "tests": tests, "truncated": truncated }),
        guessed,
        limitations(truncated, o.max_nodes, guessed),
        "",
    ))
}

pub fn tests_for(root: &Path, repo: &str, qn: &str, max_nodes: usize, min_conf: f64) -> Result<Value> {
    check_bounds(1, max_nodes, min_conf)?;
    let mut c = Ctx::open(root, repo)?;
    let start = c.node(qn)?;
    let (v, truncated) = bfs(&mut c, &[start], Direction::Callers, &[EdgeKind::Calls], min_conf, 3, max_nodes)?;
    c.ensure_nodes(&v.iter().map(|r| r.id).collect::<Vec<_>>())?;
    // a `tests` edge into the symbol itself, or into its file
    let start_path = c.nodes[&start].path.clone();
    let file_id: Option<i64> = {
        let conn = c.store.conn();
        let mut st = conn.prepare_cached(
            "SELECT id FROM node WHERE run_id=?1 AND path=?2 AND kind=?3 ORDER BY id LIMIT 1",
        )?;
        let mut rows = st.query(params![c.run_id, start_path, NodeKind::File.as_str()])?;
        match rows.next()? {
            Some(r) => Some(r.get(0)?),
            None => None,
        }
    };
    let mut want: Vec<i64> = vec![start];
    want.extend(file_id);
    want.extend(v.iter().filter(|r| r.hop > 0 && r.hop < 3).map(|r| r.id));
    let into = c.tests_into(&want, min_conf)?;
    let mut found: HashMap<String, Value> = HashMap::new();
    let mut put = |id: i64, hop: usize, conf: f64, link: &str| {
        let n = &c.nodes[&id];
        let better = found.get(&n.qualified_name).is_none_or(|e| {
            let eh = e["hop"].as_u64().unwrap_or(u64::MAX) as usize;
            hop < eh || (hop == eh && conf > e["path_confidence"].as_f64().unwrap_or(0.0))
        });
        if better {
            found.insert(
                n.qualified_name.clone(),
                json!({ "qualified_name": n.qualified_name, "kind": n.kind.as_str(), "path": n.path,
                        "hop": hop, "path_confidence": conf, "link": link }),
            );
        }
    };
    for (target, base_hop) in [(Some(start), 0usize), (file_id, 0)] {
        let Some(t) = target else { continue };
        for (s, ec) in into.get(&t).map(Vec::as_slice).unwrap_or(&[]) {
            put(*s, base_hop, *ec, "tests_edge");
        }
    }
    for r in &v {
        if is_test_path(&c.nodes[&r.id].path) && r.hop > 0 {
            put(r.id, r.hop, r.conf, "calls");
        }
        // a `tests` edge into a caller sits one hop further out than that caller
        if r.hop > 0 && r.hop < 3 {
            for (s, ec) in into.get(&r.id).map(Vec::as_slice).unwrap_or(&[]) {
                put(*s, r.hop + 1, r.conf.min(*ec), "tests_edge");
            }
        }
    }
    let mut items: Vec<Value> = found.into_values().collect();
    items.sort_by(|a, b| {
        a["hop"]
            .as_u64()
            .cmp(&b["hop"].as_u64())
            .then(a["qualified_name"].as_str().cmp(&b["qualified_name"].as_str()))
    });
    let guessed = items
        .iter()
        .filter(|i| i["path_confidence"].as_f64().unwrap_or(1.0) < 1.0)
        .count();
    let total = items.len();
    Ok(answer(
        json!({ "repo": c.repo, "symbol": qn, "items": items, "total": total, "truncated": truncated }),
        guessed,
        limitations(truncated, max_nodes, guessed),
        "",
    ))
}

/// A file whose module-level code runs on its own: a conventional entry name, or a script with `if __name__ == "__main__":`
/// (the provider marks those file nodes `entry-file`).
fn is_entry_file(path: &str, provider_ref: Option<&str>) -> bool {
    if provider_ref == Some(crate::providers::ENTRY_FILE_REF) {
        return true;
    }
    let base = path.rsplit('/').next().unwrap_or(path);
    let stem = base.rsplit_once('.').map_or(base, |(s, _)| s);
    matches!(stem, "main" | "__main__" | "cli" | "index")
}

/// Which symbols have at least one counted caller, for entry points and dead code. One streaming pass over the run's
/// reference edges (no adjacency lists): `called` holds targets of a `calls` edge, `referenced` targets of a calls, extends
/// or implements edge. A module-level call counts only from an entry file: it does not make the callee live in any other file.
struct Referenced {
    called: HashSet<i64>,
    referenced: HashSet<i64>,
}
fn referenced(c: &Ctx, min_conf: f64) -> Result<Referenced> {
    let conn = c.store.conn();
    let mut st = conn.prepare(
        "SELECT e.target_node_id, e.kind, CASE WHEN s.kind = ?3 THEN s.path END, CASE WHEN s.kind = ?3 THEN s.provider_ref END, \
                s.kind = ?4, CASE WHEN s.name LIKE 'describe: %' THEN s.path END \
         FROM edge e JOIN node s ON s.id = e.source_node_id \
         WHERE e.run_id = ?1 AND e.confidence >= ?2 AND e.kind IN ('calls','extends','implements') AND e.source_node_id <> e.target_node_id \
         GROUP BY 1, 2, 3, 4, 5, 6",
    )?;
    let mut called = HashSet::new();
    let mut referenced = HashSet::new();
    let rows = st.query_map(params![c.run_id, min_conf, NodeKind::File.as_str(), NodeKind::Test.as_str()], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, bool>(4)?,
            r.get::<_, Option<String>>(5)?,
        ))
    })?;
    for row in rows {
        let (t, k, file_path, pref, is_test_node, describe_path) = row?;
        // a test file's own scope and a `describe` body run at load time, not as a test: neither makes its callee live
        if is_test_node || describe_path.as_deref().is_some_and(is_test_path) {
            continue;
        }
        if let Some(p) = &file_path {
            if !is_entry_file(p, pref.as_deref()) {
                continue;
            }
        }
        referenced.insert(t);
        if k == EdgeKind::Calls.as_str() {
            called.insert(t);
        }
    }
    Ok(Referenced { called, referenced })
}

fn handler_name(n: &str) -> bool {
    let l = n.to_ascii_lowercase();
    l == "main" || l.starts_with("handle") || l.ends_with("handler") || l.ends_with("_handler")
}
fn is_entry(n: &Node, called: &HashSet<i64>) -> bool {
    if !matches!(n.kind, NodeKind::Function | NodeKind::Method | NodeKind::Route)
        || is_test_path(&n.path)
        || called.contains(&n.id.unwrap().0)
    {
        return false;
    }
    let exported = n.kind == NodeKind::Function && !(n.language == "python" && n.name.starts_with('_'));
    handler_name(&n.name) || n.kind == NodeKind::Route || exported
}

fn brief(n: &Node) -> Value {
    json!({ "qualified_name": n.qualified_name, "kind": n.kind.as_str(), "path": n.path, "line_start": n.line_start })
}

/// One page of the qualified-name-ordered symbols of `kinds` that pass `keep`. The nodes stream in index order and only
/// the page's rows become JSON; `total` is the count of all passing symbols (a cheap predicate per row, no list is built).
fn paged_nodes(
    c: &Ctx,
    kinds: &[NodeKind],
    limit: usize,
    cursor: Option<&str>,
    keep: &dyn Fn(&Node) -> bool,
    render: &dyn Fn(&Node) -> Value,
) -> Result<(Vec<Value>, Value)> {
    let offset = match cursor {
        Some(cur) => cur
            .parse::<usize>()
            .map_err(|_| AppError::InvalidArguments(format!("bad cursor {cur}")))?,
        None => 0,
    };
    let limit = limit.clamp(1, 500);
    let kind_list = kinds.iter().map(|k| format!("'{}'", k.as_str())).collect::<Vec<_>>().join(",");
    let conn = c.store.conn();
    let mut st = conn.prepare(&format!(
        "SELECT {NODE_COLS} FROM node WHERE run_id=?1 AND kind IN ({kind_list}) ORDER BY qualified_name, path, line_start, id"
    ))?;
    let rows = st.query_map(params![c.run_id], node_from_row)?;
    let (mut total, mut items) = (0usize, vec![]);
    for row in rows {
        let n = row?;
        if !keep(&n) {
            continue;
        }
        if total >= offset && items.len() < limit {
            items.push(render(&n));
        }
        total += 1;
    }
    let next = (offset + limit < total).then(|| (offset + limit).to_string());
    Ok((items, json!({ "next": next, "total": total })))
}

pub fn entry_points(root: &Path, repo: &str, limit: usize, cursor: Option<&str>, min_conf: f64) -> Result<Value> {
    check_conf(min_conf)?;
    let c = Ctx::open(root, repo)?;
    let r = referenced(&c, min_conf)?;
    let (items, pg) = paged_nodes(
        &c,
        &[NodeKind::Function, NodeKind::Method, NodeKind::Route],
        limit,
        cursor,
        &|n| is_entry(n, &r.called),
        &brief,
    )?;
    Ok(answer(
        json!({ "repo": c.repo, "items": items, "page": pg }),
        0,
        vec![Limitation::new(
            "exported_heuristic",
            "no export information is stored: an entry point is a function nothing calls that is `main`, handler-named, a route, or (Python) not underscore-prefixed",
        )],
        "inferred",
    ))
}

pub fn dead_code(root: &Path, repo: &str, limit: usize, cursor: Option<&str>, min_conf: f64) -> Result<Value> {
    check_conf(min_conf)?;
    let c = Ctx::open(root, repo)?;
    let r = referenced(&c, min_conf)?;
    let (items, pg) = paged_nodes(
        &c,
        &[NodeKind::Function, NodeKind::Method, NodeKind::Class],
        limit,
        cursor,
        &|n| {
            !is_test_path(&n.path)
                && !(n.name.starts_with("__") && n.name.ends_with("__"))
                && !r.referenced.contains(&n.id.unwrap().0)
                && !is_entry(n, &r.called)
                && !handler_name(&n.name)
        },
        &|n| {
            let mut b = brief(n);
            b["label"] = json!("candidate");
            b["confidence"] = json!("unknown");
            b
        },
    )?;
    Ok(answer(
        json!({ "repo": c.repo, "items": items, "page": pg }),
        0,
        vec![Limitation::new(
            "dynamic_dispatch_invisible",
            "candidates only: calls through dynamic dispatch, reflection, decorators, callbacks and framework wiring are not in the graph, so a listed symbol may be live",
        )],
        "unknown",
    ))
}
