//! `ui graph`, `ui path`, `ui graph-diff`: the interaction graph (ui-graph/1, CMP-UI-1-lanes.md section 15). Reads `ui/<product>/graph/<viewport>-<seat>.json`.
//! Every answer is bounded (`--max-nodes` files read, `--max-len` clicks) and says so in `limitation_items` when it is cut.
#![allow(clippy::result_large_err)] // serde_json::Value is larger with arbitrary_precision
use super::ui::{need_product_pub, need_token, open, seat_rank, UiSet, Want};
use super::{AppError, Result};
use crate::domain::ui::DEFAULT_VIEWPORT;
use crate::domain::ui_graph::*;
use crate::domain::Limitation;
use serde_json::{json, Value};
use std::path::Path;

pub const DEFAULT_MAX_LEN: usize = 8;
pub const MAX_LEN: usize = 20;
const GRAPHS: Want = Want { evidence: false, flows: false, navs: false, systems: false, graphs: true };

fn select_graph<'a>(set: &'a UiSet, product: &str, viewport: &str, seat: Option<&str>) -> Result<&'a UiGraph> {
    need_product_pub(set, product)?;
    let all: Vec<&UiGraph> = set.graphs.iter().filter(|(_, g)| g.product == product).map(|(_, g)| g).collect();
    let mut c: Vec<&UiGraph> = all.iter().copied().filter(|g| g.viewport == viewport && seat.is_none_or(|s| g.seat == s)).collect();
    c.sort_by_key(|g| (seat_rank(&g.seat), g.seat.clone()));
    c.first().copied().ok_or_else(|| {
        let prefix = format!("{product}/graph/");
        let refused: Vec<String> = set.invalid.iter().filter(|(rel, _)| rel.starts_with(&prefix)).map(|(rel, why)| format!("{rel} refused: {why}")).collect();
        AppError::NotFound(format!(
            "`{product}` has no graph for viewport {viewport} seat {} (graphs: {}){}",
            seat.unwrap_or("any"),
            all.iter().map(|g| g.stem()).collect::<Vec<_>>().join(", "),
            if refused.is_empty() { String::new() } else { format!("; {}", refused.join("; ")) }
        ))
    })
}
fn truncation(g: &UiGraph) -> Vec<Limitation> {
    let mut l = vec![];
    if g.truncated.depth {
        l.push(Limitation::new("graph_truncated", format!("the crawl stopped at depth {}; {} node(s) were not expanded, so depths past it and the orphan list are incomplete", g.caps.depth, g.truncated.unexpanded.len())));
    }
    if g.truncated.nodes {
        l.push(Limitation::new("graph_truncated", format!("the crawl stopped at {} nodes; screens beyond it are missing, so the orphan list is overstated", g.caps.nodes)));
    }
    if g.truncated.time {
        l.push(Limitation::new("graph_truncated", format!("the crawl ran out of its time budget with {} node(s) not expanded, so depths and the orphan list are incomplete", g.truncated.unexpanded.len())));
    }
    l
}
fn summary(s: &GraphStats) -> Value {
    json!({
        "nodes": s.nodes, "edges": s.edges, "reachable": s.reachable, "maxDepth": s.max_depth, "meanDepth": s.mean_depth, "cycles": s.cycles,
        "orphans": s.orphans.len(), "deadActions": s.dead_actions.len(), "mutatingNotClicked": s.mutating_not_clicked, "duplicatePaths": s.duplicate_paths.len(),
        "actionsMax": s.actions_max, "actionsMean": s.actions_mean, "effects": s.effects,
    })
}

pub fn graph(root: &Path, max_nodes: usize, product: &str, viewport: Option<&str>, seat: Option<&str>) -> Result<Value> {
    need_token("product", product)?;
    let set = open(root, max_nodes, Some(&[product]), GRAPHS)?;
    let g = select_graph(&set, product, viewport.unwrap_or(DEFAULT_VIEWPORT), seat)?;
    let s = graph_stats(g);
    let v = json!({
        "product": product, "version": g.version, "viewport": g.viewport, "seat": g.seat, "start": g.start, "caps": g.caps, "truncated": g.truncated,
        "summary": summary(&s), "orphans": s.orphans, "unreachable": s.unreachable, "deadActions": s.dead_actions, "duplicatePaths": s.duplicate_paths,
        "actionsPerScreen": s.actions_per_screen, "depth": s.depth,
    });
    Ok(set.finish(v, &truncation(g)))
}

#[allow(clippy::too_many_arguments)]
pub fn path(root: &Path, max_nodes: usize, product: &str, from: &str, to: &str, viewport: Option<&str>, seat: Option<&str>, max_len: usize) -> Result<Value> {
    need_token("product", product)?;
    if !(1..=MAX_LEN).contains(&max_len) {
        return Err(AppError::InvalidArguments(format!("--max-len {max_len} must be 1..{MAX_LEN}")));
    }
    let set = open(root, max_nodes, Some(&[product]), GRAPHS)?;
    let g = select_graph(&set, product, viewport.unwrap_or(DEFAULT_VIEWPORT), seat)?;
    for end in [from, to] {
        if !g.nodes.iter().any(|n| n.id == end) {
            let have: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).take(30).collect();
            return Err(AppError::NotFound(format!("`{end}` is not a node of {product} (nodes: {}{})", have.join(", "), if g.nodes.len() > 30 { ", ..." } else { "" })));
        }
    }
    let mut extra = truncation(g);
    let steps = shortest_path(g, from, to, max_len);
    let v = match &steps {
        Some(p) => json!({ "product": product, "viewport": g.viewport, "seat": g.seat, "from": from, "to": to, "found": true, "clicks": p.len(), "steps": p, "maxLen": max_len }),
        None => {
            extra.push(Limitation::new("no_path", format!("no path from `{from}` to `{to}` within {max_len} click(s) over the edges that move the person")));
            json!({ "product": product, "viewport": g.viewport, "seat": g.seat, "from": from, "to": to, "found": false, "clicks": Value::Null, "steps": [], "maxLen": max_len })
        }
    };
    Ok(set.finish(v, &extra))
}

fn default_tree(product: &str) -> &'static str {
    match product {
        "massrelay-ops" => "console",
        p if p.contains("router") => "router",
        _ => "app",
    }
}
fn target_of(set: &UiSet, tree: &str) -> Result<UiGraph> {
    let f = set.dir.join("target-nav.json");
    let text = std::fs::read_to_string(&f).map_err(|_| AppError::NotFound(format!("no target-nav.json in {} (the approved menu tree; copy tools/ui/target-nav.json there)", set.dir.display())))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| AppError::InvalidArguments(format!("target-nav.json: {e}")))?;
    let nodes = v["trees"][tree]["nodes"].clone();
    if nodes.is_null() {
        let have: Vec<&str> = v["trees"].as_object().map(|o| o.keys().map(|k| k.as_str()).collect()).unwrap_or_default();
        return Err(AppError::NotFound(format!("target-nav.json has no tree `{tree}` (trees: {})", have.join(", "))));
    }
    let nodes: Vec<TargetNode> = serde_json::from_value(nodes).map_err(|e| AppError::InvalidArguments(format!("target-nav.json tree `{tree}`: {e}")))?;
    target_graph("target", &nodes).ok_or_else(|| AppError::InvalidArguments(format!("target-nav.json tree `{tree}` has no place with a screen")))
}

#[allow(clippy::too_many_arguments)]
pub fn graph_diff(root: &Path, max_nodes: usize, ours: &str, refp: &str, viewport: Option<&str>, seat: Option<&str>, ref_seat: Option<&str>, tree: Option<&str>) -> Result<Value> {
    need_token("--ours", ours)?;
    need_token("--ref", refp)?;
    let vp = viewport.unwrap_or(DEFAULT_VIEWPORT);
    let only: Vec<&str> = if refp == "target" { vec![ours] } else { vec![ours, refp] };
    let set = open(root, max_nodes, Some(&only), GRAPHS)?;
    let a = select_graph(&set, ours, vp, seat)?;
    let tg;
    let b: &UiGraph = if refp == "target" {
        tg = target_of(&set, tree.unwrap_or_else(|| default_tree(ours)))?;
        &tg
    } else {
        select_graph(&set, refp, vp, ref_seat)?
    };
    let (sa, sb) = (graph_stats(a), graph_stats(b));
    let mut extra = truncation(a);
    if refp != "target" {
        extra.extend(truncation(b));
    }
    let rows = depth_rows(&sa, &sb);
    let only_ours: Vec<&str> = rows.iter().filter(|r| r.verdict == "only ours").map(|r| r.screen.as_str()).collect();
    let only_ref: Vec<&str> = rows.iter().filter(|r| r.verdict == "only ref").map(|r| r.screen.as_str()).collect();
    let v = json!({
        "viewport": vp,
        "ours": { "product": ours, "version": a.version, "seat": a.seat, "summary": summary(&sa) },
        "ref": { "product": refp, "version": b.version, "seat": b.seat, "summary": summary(&sb) },
        "diff": graph_diff_words(&sa, &sb),
        "depth": rows,
        "onlyOurs": only_ours, "onlyRef": only_ref,
        "orphans": { "ours": sa.orphans, "ref": sb.orphans },
        "deadActions": { "ours": sa.dead_actions, "ref": sb.dead_actions },
    });
    Ok(set.finish(v, &extra))
}
