//! Graph queries over one run's stored edges: traverse, impact, tests-for, entry points, dead-code candidates.
//! The edges are loaded once into memory and walked breadth-first, each node visited once, bounded by depth and max-nodes.
//! Every answer is a JSON object; `evidence_confidence` and `limitation_items` are lifted into the envelope by the CLI layer.
use super::{AppError, Result};
use crate::domain::{EdgeKind, Limitation, Node, NodeKind};
use crate::providers::reference::is_test_path;
use crate::store::Store;
use serde_json::{json, Value};
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

type Adj = HashMap<i64, Vec<(i64, EdgeKind, f64)>>;
struct Ctx {
    repo: String,
    nodes: HashMap<i64, Node>,
    by_qn: HashMap<String, i64>,
    out: Adj,
    inn: Adj,
    community: HashMap<i64, String>,
}

fn load(root: &Path, repo: &str) -> Result<Ctx> {
    let store = Store::open_existing(root)?;
    let r = store
        .repo_by_name(repo)?
        .ok_or_else(|| AppError::NotFound(format!("no repo named {repo}")))?;
    let run = store
        .latest_done_run(r.id.unwrap())?
        .ok_or_else(|| AppError::NotFound(format!("{repo} has no finished run")))?;
    let run_id = run.id.unwrap();
    let mut nodes = HashMap::new();
    let mut by_qn = HashMap::new();
    for n in store.nodes_of_run(run_id)? {
        let id = n.id.unwrap().0;
        by_qn.insert(n.qualified_name.clone(), id);
        nodes.insert(id, n);
    }
    // parallel edges of one kind collapse to the best confidence
    let mut best: HashMap<(EdgeKind, i64, i64), f64> = HashMap::new();
    for e in store.edges_of_run(run_id)? {
        let k = (e.kind, e.source.0, e.target.0);
        let c = best.entry(k).or_insert(e.confidence);
        if e.confidence > *c {
            *c = e.confidence;
        }
    }
    let mut out: Adj = HashMap::new();
    let mut inn: Adj = HashMap::new();
    for ((k, s, t), c) in best {
        out.entry(s).or_default().push((t, k, c));
        inn.entry(t).or_default().push((s, k, c));
    }
    for adj in [&mut out, &mut inn] {
        for v in adj.values_mut() {
            v.sort_by(|a, b| {
                nodes[&a.0]
                    .qualified_name
                    .cmp(&nodes[&b.0].qualified_name)
                    .then(a.1.as_str().cmp(b.1.as_str()))
            });
        }
    }
    let mut community = HashMap::new();
    {
        let c = store.conn();
        let mut st = c.prepare(
            "SELECT x.node_id, cm.label FROM community_member x JOIN community cm ON cm.id = x.community_id WHERE cm.run_id = ?1",
        )?;
        let rows = st.query_map(rusqlite::params![run_id.0], |x| {
            Ok((x.get::<_, i64>(0)?, x.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (n, l) = row?;
            community.insert(n, l);
        }
    }
    Ok(Ctx {
        repo: r.name,
        nodes,
        by_qn,
        out,
        inn,
        community,
    })
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
    c: &Ctx,
    starts: &[i64],
    dir: Direction,
    kinds: &[EdgeKind],
    min_conf: f64,
    depth: usize,
    max_nodes: usize,
) -> (Vec<Reached>, bool) {
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
    // best-confidence paths: exhaust the 1.0 edges first, then extend the frontier with the 0.5 edges (a node keeps the
    // 1.0 path even when a shorter 0.5 path exists)
    if min_conf < 1.0 {
        if expand(c, &mut v, &mut seen, q.clone(), n_starts, dir, kinds, 1.0, depth, max_nodes) {
            return (v, true);
        }
        q = (0..v.len()).collect();
    }
    let cut = expand(c, &mut v, &mut seen, q, n_starts, dir, kinds, min_conf, depth, max_nodes);
    (v, cut)
}

#[allow(clippy::too_many_arguments)]
fn expand(
    c: &Ctx,
    v: &mut Vec<Reached>,
    seen: &mut HashSet<i64>,
    mut q: VecDeque<usize>,
    n_starts: usize,
    dir: Direction,
    kinds: &[EdgeKind],
    min_conf: f64,
    depth: usize,
    max_nodes: usize,
) -> bool {
    let empty = vec![];
    while let Some(i) = q.pop_front() {
        let (id, hop, conf) = (v[i].id, v[i].hop, v[i].conf);
        if hop >= depth {
            continue;
        }
        let mut next: Vec<&(i64, EdgeKind, f64)> = vec![];
        if matches!(dir, Direction::Callees | Direction::Both) {
            next.extend(c.out.get(&id).unwrap_or(&empty));
        }
        if matches!(dir, Direction::Callers | Direction::Both) {
            next.extend(c.inn.get(&id).unwrap_or(&empty));
        }
        for (t, k, ec) in next {
            if !kinds.contains(k) || *ec < min_conf || seen.contains(t) {
                continue;
            }
            if v.len() - n_starts >= max_nodes {
                return true;
            }
            seen.insert(*t);
            v.push(Reached {
                id: *t,
                hop: hop + 1,
                conf: conf.min(*ec),
                parent: Some(i),
            });
            q.push_back(v.len() - 1);
        }
    }
    false
}

impl Ctx {
    fn node(&self, qn: &str) -> Result<i64> {
        self.by_qn
            .get(qn)
            .copied()
            .ok_or_else(|| AppError::NotFound(format!("no symbol {qn} in {}", self.repo)))
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
    let c = load(root, repo)?;
    let start = c.node(o.qualified_name)?;
    let (v, truncated) = bfs(&c, &[start], o.direction, &o.kinds, o.min_conf, o.depth, o.max_nodes);
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
    let c = load(root, repo)?;
    let mut starts: Vec<i64> = vec![];
    if let Some(qn) = o.qualified_name {
        starts.push(c.node(qn)?);
    }
    for f in o.files {
        let mut ids: Vec<i64> = c
            .nodes
            .iter()
            .filter(|(_, n)| &n.path == f)
            .map(|(i, _)| *i)
            .collect();
        if ids.is_empty() {
            return Err(AppError::NotFound(format!("no file {f} in {}", c.repo)));
        }
        ids.sort_by(|a, b| c.nodes[a].qualified_name.cmp(&c.nodes[b].qualified_name));
        starts.extend(ids);
    }
    let kinds = [
        EdgeKind::Calls,
        EdgeKind::Extends,
        EdgeKind::Implements,
        EdgeKind::Imports,
    ];
    let (v, truncated) = bfs(&c, &starts, Direction::Callers, &kinds, o.min_conf, o.depth, o.max_nodes);
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
        for (s, k, ec) in c.inn.get(&r.id).map(Vec::as_slice).unwrap_or(&[]) {
            if *k == EdgeKind::Tests && *ec >= o.min_conf {
                add_test(*s, r.conf.min(*ec), "tests_edge");
            }
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
    let c = load(root, repo)?;
    let start = c.node(qn)?;
    let (v, truncated) = bfs(&c, &[start], Direction::Callers, &[EdgeKind::Calls], min_conf, 3, max_nodes);
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
    // a `tests` edge into the symbol itself, or into its file
    let file_id = c
        .nodes
        .iter()
        .find(|(_, n)| n.kind == NodeKind::File && n.path == c.nodes[&start].path)
        .map(|(i, _)| *i);
    for (target, base_hop) in [(Some(start), 0usize), (file_id, 0)] {
        let Some(t) = target else { continue };
        for (s, k, ec) in c.inn.get(&t).map(Vec::as_slice).unwrap_or(&[]) {
            if *k == EdgeKind::Tests && *ec >= min_conf {
                put(*s, base_hop, *ec, "tests_edge");
            }
        }
    }
    for r in &v {
        if is_test_path(&c.nodes[&r.id].path) && r.hop > 0 {
            put(r.id, r.hop, r.conf, "calls");
        }
        // a `tests` edge into a caller sits one hop further out than that caller
        if r.hop > 0 && r.hop < 3 {
            for (s, k, ec) in c.inn.get(&r.id).map(Vec::as_slice).unwrap_or(&[]) {
                if *k == EdgeKind::Tests && *ec >= min_conf {
                    put(*s, r.hop + 1, r.conf.min(*ec), "tests_edge");
                }
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
fn is_entry_file(n: &Node) -> bool {
    if n.provider_ref.as_deref() == Some(crate::providers::ENTRY_FILE_REF) {
        return true;
    }
    let base = n.path.rsplit('/').next().unwrap_or(&n.path);
    let stem = base.rsplit_once('.').map_or(base, |(s, _)| s);
    matches!(stem, "main" | "__main__" | "cli" | "index")
}

fn callers_of(c: &Ctx, id: i64, kinds: &[EdgeKind], min_conf: f64) -> usize {
    c.inn
        .get(&id)
        .map(|v| {
            v.iter()
                // a module-level call counts only from an entry file: it does not make the callee live in any other file
                .filter(|(s, _, _)| c.nodes[s].kind != NodeKind::File || is_entry_file(&c.nodes[s]))
                .filter(|(s, k, ec)| *s != id && kinds.contains(k) && *ec >= min_conf)
                .count()
        })
        .unwrap_or(0)
}
fn handler_name(n: &str) -> bool {
    let l = n.to_ascii_lowercase();
    l == "main" || l.starts_with("handle") || l.ends_with("handler") || l.ends_with("_handler")
}
fn is_entry(c: &Ctx, id: i64, min_conf: f64) -> bool {
    let n = &c.nodes[&id];
    if !matches!(n.kind, NodeKind::Function | NodeKind::Method | NodeKind::Route)
        || is_test_path(&n.path)
        || callers_of(c, id, &[EdgeKind::Calls], min_conf) > 0
    {
        return false;
    }
    let exported = n.kind == NodeKind::Function && !(n.language == "python" && n.name.starts_with('_'));
    handler_name(&n.name) || n.kind == NodeKind::Route || exported
}

fn page(mut items: Vec<Value>, limit: usize, cursor: Option<&str>) -> Result<(Vec<Value>, Value)> {
    let offset = match cursor {
        Some(c) => c
            .parse::<usize>()
            .map_err(|_| AppError::InvalidArguments(format!("bad cursor {c}")))?,
        None => 0,
    };
    let total = items.len();
    let limit = limit.clamp(1, 500);
    let rest = items.split_off(offset.min(total));
    let next = (offset + limit < total).then(|| (offset + limit).to_string());
    let items: Vec<Value> = rest.into_iter().take(limit).collect();
    Ok((items, json!({ "next": next, "total": total })))
}
fn brief(n: &Node) -> Value {
    json!({ "qualified_name": n.qualified_name, "kind": n.kind.as_str(), "path": n.path, "line_start": n.line_start })
}
fn sorted_ids(c: &Ctx) -> Vec<i64> {
    let mut ids: Vec<i64> = c.nodes.keys().copied().collect();
    ids.sort_by(|a, b| c.nodes[a].qualified_name.cmp(&c.nodes[b].qualified_name));
    ids
}

pub fn entry_points(root: &Path, repo: &str, limit: usize, cursor: Option<&str>, min_conf: f64) -> Result<Value> {
    check_conf(min_conf)?;
    let c = load(root, repo)?;
    let items: Vec<Value> = sorted_ids(&c)
        .into_iter()
        .filter(|i| is_entry(&c, *i, min_conf))
        .map(|i| brief(&c.nodes[&i]))
        .collect();
    let (items, pg) = page(items, limit, cursor)?;
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
    let c = load(root, repo)?;
    let refs = [EdgeKind::Calls, EdgeKind::Extends, EdgeKind::Implements];
    let items: Vec<Value> = sorted_ids(&c)
        .into_iter()
        .filter(|i| {
            let n = &c.nodes[i];
            matches!(n.kind, NodeKind::Function | NodeKind::Method | NodeKind::Class)
                && !is_test_path(&n.path)
                && !(n.name.starts_with("__") && n.name.ends_with("__"))
                && callers_of(&c, *i, &refs, min_conf) == 0
                && !is_entry(&c, *i, min_conf)
                && !handler_name(&n.name)
        })
        .map(|i| {
            let mut b = brief(&c.nodes[&i]);
            b["label"] = json!("candidate");
            b["confidence"] = json!("unknown");
            b
        })
        .collect();
    let (items, pg) = page(items, limit, cursor)?;
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
