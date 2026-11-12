//! The interaction graph (ui-graph/1, CMP-UI-1-lanes.md section 15): screens are nodes, actions are edges. Pure, no I/O.
//! Every number is computed from `nodes` + `edges` (+ `registryScreens`), never read from the probe's own `derived` block.
use super::ui::{fmt_num, token_ok, viewport_ok, vs, SEATS};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const GRAPH_SCHEMA: &str = "ui-graph/1";
pub const ACTION_KINDS: &[&str] = &["menu", "tab", "link", "button", "row", "dialog-open", "dialog-close", "back"];
pub const ACTION_REGIONS: &[&str] = &["rail", "sidebar", "topbar", "main", "panel", "bottombar"];
pub const EFFECTS: &[&str] = &["navigate", "open-dialog", "open-panel", "toggle", "none", "error", "external", "mutating-not-clicked"];
/// Edges that move the person to another node: they carry the depth, the paths and the cycles.
const TRAVERSE: &[&str] = &["navigate", "open-dialog", "open-panel"];
/// More nodes than the probe's own cap (300) by a wide margin is a broken file, and keeps the recursion below bounded.
pub const MAX_GRAPH_NODES: usize = 2000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: String,
    pub state: String,
    pub route: String,
    pub dom_hash: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphAction {
    pub kind: String,
    pub name: String,
    pub region: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: String,
    pub action: GraphAction,
    pub to: String,
    pub effect: String,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphTruncated {
    #[serde(default)]
    pub depth: bool,
    #[serde(default)]
    pub nodes: bool,
    /// the probe's time budget ran out
    #[serde(default)]
    pub time: bool,
    #[serde(default)]
    pub unexpanded: Vec<String>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphCaps {
    #[serde(default)]
    pub depth: u32,
    #[serde(default)]
    pub nodes: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiGraph {
    pub schema: String,
    pub product: String,
    pub version: String,
    pub viewport: String,
    pub seat: String,
    pub start: String,
    #[serde(default)]
    pub registry_screens: Vec<String>,
    #[serde(default)]
    pub caps: GraphCaps,
    #[serde(default)]
    pub truncated: GraphTruncated,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

impl UiGraph {
    pub fn stem(&self) -> String {
        format!("{}-{}", self.viewport, self.seat)
    }
    pub fn problems(&self) -> Vec<String> {
        let mut p = vec![];
        if self.schema != GRAPH_SCHEMA {
            p.push(format!("schema `{}` is not `{GRAPH_SCHEMA}`", self.schema));
        }
        if !token_ok(&self.product) {
            p.push(format!("product `{}` is not a path-safe token", self.product));
        }
        if !viewport_ok(&self.viewport) {
            p.push(format!("viewport `{}` must be <width>x<height>", self.viewport));
        }
        if !SEATS.contains(&self.seat.as_str()) {
            p.push(format!("seat `{}` must be one of {}", self.seat, SEATS.join("|")));
        }
        if self.nodes.len() > MAX_GRAPH_NODES {
            p.push(format!("{} nodes is over the limit of {MAX_GRAPH_NODES}", self.nodes.len()));
            return p;
        }
        let mut ids = BTreeSet::new();
        for n in &self.nodes {
            if n.id.trim().is_empty() {
                p.push("a node has an empty id".into());
            }
            if !ids.insert(n.id.as_str()) {
                p.push(format!("node `{}` appears twice", n.id));
            }
        }
        if !ids.contains(self.start.as_str()) {
            p.push(format!("start `{}` is not a node", self.start));
        }
        for e in &self.edges {
            let who = format!("edge `{}` -> `{}`", e.from, e.to);
            if !ids.contains(e.from.as_str()) {
                p.push(format!("{who}: from is not a node"));
            }
            if !ACTION_KINDS.contains(&e.action.kind.as_str()) {
                p.push(format!("{who}: kind `{}` must be one of {}", e.action.kind, ACTION_KINDS.join("|")));
            }
            if !ACTION_REGIONS.contains(&e.action.region.as_str()) {
                p.push(format!("{who}: region `{}` must be one of {}", e.action.region, ACTION_REGIONS.join("|")));
            }
            if e.action.name.trim().is_empty() {
                p.push(format!("{who}: empty action name"));
            }
            if !EFFECTS.contains(&e.effect.as_str()) {
                p.push(format!("{who}: effect `{}` must be one of {}", e.effect, EFFECTS.join("|")));
            }
            let known_to = ids.contains(e.to.as_str()) || matches!(e.to.as_str(), "unknown" | "external");
            if !known_to && !self.truncated.nodes {
                p.push(format!("{who}: `to` is not a node (and the crawl was not cut by the node cap)"));
            }
            if e.effect == "mutating-not-clicked" && e.to != "unknown" {
                p.push(format!("{who}: a mutating-not-clicked edge leaves its target `unknown`"));
            }
        }
        p
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadAction {
    pub from: String,
    pub name: String,
    pub kind: String,
    pub region: String,
    pub effect: String,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DuplicatePath {
    pub to: String,
    pub sources: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphStats {
    pub nodes: usize,
    pub edges: usize,
    pub reachable: usize,
    pub unreachable: Vec<String>,
    pub orphans: Vec<String>,
    pub max_depth: u32,
    pub mean_depth: f64,
    pub depth: BTreeMap<String, u32>,
    pub dead_actions: Vec<DeadAction>,
    pub mutating_not_clicked: usize,
    pub duplicate_paths: Vec<DuplicatePath>,
    pub cycles: usize,
    pub actions_max: usize,
    pub actions_mean: f64,
    pub actions_per_screen: BTreeMap<String, usize>,
    pub effects: BTreeMap<String, usize>,
}
fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}
fn act_key(e: &GraphEdge) -> String {
    format!("{}|{}|{}", e.action.kind, e.action.name, e.action.region)
}
fn adjacency(g: &UiGraph) -> BTreeMap<&str, Vec<&str>> {
    let ids: BTreeSet<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    let mut adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in &g.edges {
        if TRAVERSE.contains(&e.effect.as_str()) && e.from != e.to && ids.contains(e.to.as_str()) {
            adj.entry(e.from.as_str()).or_default().push(e.to.as_str());
        }
    }
    adj
}
/// Click distance from `start` over the edges that move the person (breadth first, so the first hit is the shortest).
pub fn depths(g: &UiGraph) -> BTreeMap<String, u32> {
    let adj = adjacency(g);
    let mut d: BTreeMap<String, u32> = BTreeMap::new();
    d.insert(g.start.clone(), 0);
    let mut q = VecDeque::from([g.start.as_str()]);
    while let Some(v) = q.pop_front() {
        let dv = d[v];
        for w in adj.get(v).into_iter().flatten() {
            if !d.contains_key(*w) {
                d.insert((*w).to_string(), dv + 1);
                q.push_back(w);
            }
        }
    }
    d
}
/// Strongly connected components with more than one node (Tarjan, recursion bounded by MAX_GRAPH_NODES).
fn cycle_count(g: &UiGraph) -> usize {
    struct T<'a> {
        adj: &'a BTreeMap<&'a str, Vec<&'a str>>,
        idx: usize,
        ix: BTreeMap<&'a str, usize>,
        low: BTreeMap<&'a str, usize>,
        st: Vec<&'a str>,
        on: BTreeSet<&'a str>,
        cycles: usize,
    }
    fn strong<'a>(t: &mut T<'a>, v: &'a str) {
        t.ix.insert(v, t.idx);
        t.low.insert(v, t.idx);
        t.idx += 1;
        t.st.push(v);
        t.on.insert(v);
        for w in t.adj.get(v).into_iter().flatten() {
            if !t.ix.contains_key(w) {
                strong(t, w);
                let m = t.low[v].min(t.low[w]);
                t.low.insert(v, m);
            } else if t.on.contains(w) {
                let m = t.low[v].min(t.ix[w]);
                t.low.insert(v, m);
            }
        }
        if t.low[v] == t.ix[v] {
            let mut n = 0;
            while let Some(w) = t.st.pop() {
                t.on.remove(w);
                n += 1;
                if w == v {
                    break;
                }
            }
            if n > 1 {
                t.cycles += 1;
            }
        }
    }
    let adj = adjacency(g);
    let mut t = T { adj: &adj, idx: 0, ix: BTreeMap::new(), low: BTreeMap::new(), st: vec![], on: BTreeSet::new(), cycles: 0 };
    for n in &g.nodes {
        if !t.ix.contains_key(n.id.as_str()) {
            strong(&mut t, n.id.as_str());
        }
    }
    t.cycles
}
pub fn graph_stats(g: &UiGraph) -> GraphStats {
    let depth = depths(g);
    let ids: BTreeSet<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    let mut per: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut effects: BTreeMap<String, usize> = BTreeMap::new();
    for e in &g.edges {
        per.entry(e.from.clone()).or_default().insert(act_key(e));
        *effects.entry(e.effect.clone()).or_insert(0) += 1;
    }
    let actions_per_screen: BTreeMap<String, usize> = per.iter().map(|(k, v)| (k.clone(), v.len())).collect();
    let counts: Vec<usize> = actions_per_screen.values().copied().collect();
    let mut dead: Vec<DeadAction> = g
        .edges
        .iter()
        .filter(|e| e.effect == "none" || e.effect == "error")
        .map(|e| DeadAction { from: e.from.clone(), name: e.action.name.clone(), kind: e.action.kind.clone(), region: e.action.region.clone(), effect: e.effect.clone() })
        .collect();
    dead.sort_by(|a, b| (&a.from, &a.name, &a.region).cmp(&(&b.from, &b.name, &b.region)));
    let mut by_to: BTreeMap<&str, (BTreeSet<String>, BTreeSet<&str>)> = BTreeMap::new();
    for e in g.edges.iter().filter(|e| TRAVERSE.contains(&e.effect.as_str()) && e.from != e.to && ids.contains(e.to.as_str())) {
        let m = by_to.entry(e.to.as_str()).or_default();
        m.0.insert(format!("{}|{}", e.from, act_key(e)));
        m.1.insert(e.from.as_str());
    }
    let duplicate_paths = by_to
        .into_iter()
        .filter(|(_, (a, _))| a.len() >= 2)
        .map(|(to, (_, from))| DuplicatePath { to: to.to_string(), sources: from.into_iter().map(String::from).collect() })
        .collect();
    let orphans: Vec<String> = g.registry_screens.iter().filter(|s| !ids.contains(s.as_str())).cloned().collect::<BTreeSet<_>>().into_iter().collect();
    let ds: Vec<u32> = depth.values().copied().collect();
    GraphStats {
        nodes: g.nodes.len(),
        edges: g.edges.len(),
        reachable: depth.len(),
        unreachable: g.nodes.iter().filter(|n| !depth.contains_key(&n.id)).map(|n| n.id.clone()).collect::<BTreeSet<_>>().into_iter().collect(),
        orphans,
        max_depth: ds.iter().copied().max().unwrap_or(0),
        mean_depth: if ds.is_empty() { 0.0 } else { round2(ds.iter().map(|d| *d as f64).sum::<f64>() / ds.len() as f64) },
        depth,
        mutating_not_clicked: effects.get("mutating-not-clicked").copied().unwrap_or(0),
        dead_actions: dead,
        duplicate_paths,
        cycles: cycle_count(g),
        actions_max: counts.iter().copied().max().unwrap_or(0),
        actions_mean: if counts.is_empty() { 0.0 } else { round2(counts.iter().sum::<usize>() as f64 / counts.len() as f64) },
        actions_per_screen,
        effects,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PathStep {
    pub from: String,
    pub kind: String,
    pub name: String,
    pub region: String,
    pub effect: String,
    pub to: String,
}
/// The shortest click path `from` -> `to` over the edges that move the person, at most `max_len` clicks. `None` when there is none within the bound.
pub fn shortest_path(g: &UiGraph, from: &str, to: &str, max_len: usize) -> Option<Vec<PathStep>> {
    let ids: BTreeSet<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    if !ids.contains(from) || !ids.contains(to) {
        return None;
    }
    let mut prev: BTreeMap<&str, (&str, &GraphEdge)> = BTreeMap::new();
    let mut seen: BTreeSet<&str> = BTreeSet::from([from]);
    let mut q = VecDeque::from([(from, 0usize)]);
    while let Some((v, d)) = q.pop_front() {
        if v == to {
            break;
        }
        if d >= max_len {
            continue;
        }
        for e in g.edges.iter().filter(|e| e.from == v && TRAVERSE.contains(&e.effect.as_str()) && ids.contains(e.to.as_str())) {
            if seen.insert(e.to.as_str()) {
                prev.insert(e.to.as_str(), (v, e));
                q.push_back((e.to.as_str(), d + 1));
            }
        }
    }
    if from != to && !prev.contains_key(to) {
        return None;
    }
    let mut out = vec![];
    let mut cur = to;
    while cur != from {
        let (p, e) = prev[cur];
        out.push(PathStep { from: p.to_string(), kind: e.action.kind.clone(), name: e.action.name.clone(), region: e.action.region.clone(), effect: e.effect.clone(), to: e.to.clone() });
        cur = p;
    }
    out.reverse();
    Some(out)
}

/// A node that is a real screen of the product (not a dialog or panel of one, not an unregistered route).
pub fn is_screen(id: &str) -> bool {
    !id.contains('#') && !id.starts_with("unregistered:")
}
/// The two graphs in words, `a vs b`, only where they differ.
pub fn graph_diff_words(a: &GraphStats, b: &GraphStats) -> Vec<String> {
    let mut out = vec![];
    let o = &mut out;
    let screens = |s: &GraphStats| s.depth.keys().filter(|k| is_screen(k)).count() as f64;
    vs(o, "reachable screens", screens(a), screens(b));
    vs(o, "max depth", a.max_depth as f64, b.max_depth as f64);
    vs(o, "mean depth", a.mean_depth, b.mean_depth);
    vs(o, "orphans", a.orphans.len() as f64, b.orphans.len() as f64);
    vs(o, "dead actions", a.dead_actions.len() as f64, b.dead_actions.len() as f64);
    vs(o, "mutating actions", a.mutating_not_clicked as f64, b.mutating_not_clicked as f64);
    vs(o, "cycles", a.cycles as f64, b.cycles as f64);
    vs(o, "duplicate paths", a.duplicate_paths.len() as f64, b.duplicate_paths.len() as f64);
    vs(o, "actions per screen (mean)", a.actions_mean, b.actions_mean);
    vs(o, "actions per screen (max)", a.actions_max as f64, b.actions_max as f64);
    out
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DepthRow {
    pub screen: String,
    pub ours: Option<u32>,
    #[serde(rename = "ref")]
    pub refd: Option<u32>,
    pub verdict: String,
}
/// Depth to every screen either side has, in screen order. Verdict: same | ours deeper | ours shallower | ours unreachable | only ours | only ref.
pub fn depth_rows(a: &GraphStats, b: &GraphStats) -> Vec<DepthRow> {
    let keys: BTreeSet<&String> = a.depth.keys().chain(b.depth.keys()).filter(|k| is_screen(k)).collect();
    keys.into_iter()
        .map(|k| {
            let (x, y) = (a.depth.get(k).copied(), b.depth.get(k).copied());
            let verdict = match (x, y) {
                (Some(x), Some(y)) if x == y => "same".to_string(),
                (Some(x), Some(y)) if x > y => format!("ours deeper by {}", fmt_num((x - y) as f64)),
                (Some(x), Some(y)) => format!("ours shallower by {}", fmt_num((y - x) as f64)),
                (Some(_), None) => "only ours".into(),
                (None, Some(_)) => "only ref".into(),
                (None, None) => "unreachable".into(),
            };
            DepthRow { screen: k.to_string(), ours: x, refd: y, verdict }
        })
        .collect()
}

/// One node of a hand-written target menu tree (ui-nav/1 node shape).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TargetNode {
    pub label: String,
    pub kind: String,
    pub level: u32,
    pub target: String,
}
/// The TARGET graph from the approved menu tree: the first level-1 place is the start; every place is one click from it, every deeper item one click
/// from the nearest ancestor that has a screen. These are the intended edges of the places, nothing more.
pub fn target_graph(name: &str, nodes: &[TargetNode]) -> Option<UiGraph> {
    let screen = |t: &str| !matches!(t, "unknown" | "external");
    let start = nodes.iter().find(|n| n.level == 1 && screen(&n.target))?.target.clone();
    let mut stack: Vec<(u32, String)> = vec![];
    let mut ids: BTreeSet<String> = BTreeSet::from([start.clone()]);
    let mut edges: Vec<GraphEdge> = vec![];
    for n in nodes {
        while stack.last().is_some_and(|(l, _)| *l >= n.level) {
            stack.pop();
        }
        let parent = stack.iter().rev().find(|(_, t)| screen(t)).map(|(_, t)| t.clone()).unwrap_or_else(|| start.clone());
        if screen(&n.target) {
            ids.insert(n.target.clone());
            let e = GraphEdge {
                from: parent.clone(),
                action: GraphAction { kind: if n.kind == "tab" { "tab".into() } else { "menu".into() }, name: n.label.clone(), region: if n.kind == "tab" { "main".into() } else { "sidebar".into() } },
                to: n.target.clone(),
                effect: "navigate".into(),
            };
            if parent != n.target && !edges.contains(&e) {
                edges.push(e);
            }
        }
        stack.push((n.level, n.target.clone()));
    }
    Some(UiGraph {
        schema: GRAPH_SCHEMA.into(),
        product: name.into(),
        version: "target".into(),
        viewport: "1440x900".into(),
        seat: "admin-dev".into(),
        start,
        registry_screens: vec![],
        caps: GraphCaps::default(),
        truncated: GraphTruncated::default(),
        nodes: ids.into_iter().map(|id| GraphNode { id, state: "populated".into(), route: String::new(), dom_hash: String::new() }).collect(),
        edges,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn n(id: &str) -> GraphNode {
        GraphNode { id: id.into(), state: "populated".into(), route: String::new(), dom_hash: "0".repeat(64) }
    }
    fn e(from: &str, name: &str, to: &str, effect: &str, kind: &str) -> GraphEdge {
        GraphEdge { from: from.into(), action: GraphAction { kind: kind.into(), name: name.into(), region: "main".into() }, to: to.into(), effect: effect.into() }
    }
    fn g() -> UiGraph {
        UiGraph {
            schema: GRAPH_SCHEMA.into(),
            product: "p".into(),
            version: "v".into(),
            viewport: "1440x900".into(),
            seat: "member".into(),
            start: "a".into(),
            registry_screens: vec!["a".into(), "b".into(), "c".into(), "d".into(), "z".into()],
            caps: GraphCaps::default(),
            truncated: GraphTruncated::default(),
            nodes: ["a", "b", "c", "d"].iter().map(|i| n(i)).collect(),
            edges: vec![
                e("a", "to b", "b", "navigate", "link"),
                e("a", "to c", "c", "navigate", "link"),
                e("b", "to d", "d", "navigate", "link"),
                e("c", "to d", "d", "open-dialog", "dialog-open"),
                e("d", "home", "a", "navigate", "link"),
                e("a", "nothing", "a", "none", "button"),
                e("a", "bad", "a", "error", "button"),
                e("a", "Delete", "unknown", "mutating-not-clicked", "button"),
            ],
        }
    }
    #[test]
    fn stats_are_computed_from_nodes_and_edges() {
        let s = graph_stats(&g());
        assert_eq!(s.depth, BTreeMap::from([("a".into(), 0), ("b".into(), 1), ("c".into(), 1), ("d".into(), 2)]));
        assert_eq!((s.reachable, s.max_depth, s.mean_depth), (4, 2, 1.0));
        assert_eq!(s.orphans, vec!["z".to_string()]);
        assert_eq!(s.dead_actions.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["bad", "nothing"]);
        assert_eq!(s.duplicate_paths, vec![DuplicatePath { to: "d".into(), sources: vec!["b".into(), "c".into()] }]);
        assert_eq!(s.cycles, 1);
        assert_eq!(s.mutating_not_clicked, 1);
        assert_eq!(s.actions_per_screen["a"], 5);
        assert_eq!(s.actions_max, 5);
    }
    #[test]
    fn shortest_path_is_the_fewest_clicks_and_bounded() {
        let p = shortest_path(&g(), "a", "d", 8).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!((p[0].to.as_str(), p[1].to.as_str()), (if p[0].to == "b" { "b" } else { "c" }, "d"));
        assert!(shortest_path(&g(), "a", "d", 1).is_none(), "bound of one click cannot reach depth two");
        assert_eq!(shortest_path(&g(), "a", "a", 3).unwrap().len(), 0);
        assert!(shortest_path(&g(), "a", "nope", 8).is_none());
    }
    #[test]
    fn problems_name_what_is_wrong() {
        let mut bad = g();
        bad.edges.push(e("a", "x", "ghost", "navigate", "link"));
        bad.edges.push(e("a", "y", "b", "explode", "link"));
        bad.edges.push(e("a", "z", "b", "mutating-not-clicked", "button"));
        let p = bad.problems().join(" | ");
        assert!(p.contains("`to` is not a node") && p.contains("effect `explode`") && p.contains("leaves its target `unknown`"), "{p}");
        assert!(g().problems().is_empty());
    }
    #[test]
    fn target_graph_puts_places_one_click_from_the_start() {
        let t = |l: &str, k: &str, lv: u32, tg: &str| TargetNode { label: l.into(), kind: k.into(), level: lv, target: tg.into() };
        let tg = target_graph("target", &[t("Overview", "place", 1, "ov"), t("Customers", "group", 1, "unknown"), t("Workspaces", "item", 2, "ws"), t("People", "item", 2, "pe"), t("Tabs", "tab", 2, "ov")]).unwrap();
        let s = graph_stats(&tg);
        assert_eq!(tg.start, "ov");
        assert_eq!(s.depth, BTreeMap::from([("pe".into(), 1), ("ov".into(), 0), ("ws".into(), 1)]));
        assert!(tg.problems().is_empty(), "{:?}", tg.problems());
    }
}
