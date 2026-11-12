//! Menu structure (ui-nav/1), journeys (ui-journey/1) and coverage (ui-coverage/1) - CMP-UI-1-lanes.md section 8. Pure, no I/O.
use super::ui::{fmt_num, token_ok, viewport_ok, vs, SEATS};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const NAV_SCHEMA: &str = "ui-nav/1";
pub const JOURNEY_SCHEMA: &str = "ui-journey/1";
pub const COVERAGE_SCHEMA: &str = "ui-coverage/1";
pub const NAV_KINDS: &[&str] = &["place", "group", "item", "tab", "menuitem"];
pub const HIDDEN: &[&str] = &["collapsed", "overflow", "devView"];
pub const JOURNEY_STATUS: &[&str] = &["completed", "partial", "n/a"];
pub const CELL_STATUS: &[&str] = &["captured", "n/a", "failed"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavNode {
    pub label: String,
    pub kind: String,
    pub level: u32,
    pub order: u32,
    /// a registry screen id, `external` or `unknown`
    pub target: String,
    pub icon: bool,
    pub badge: bool,
    pub hidden_behind: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiNav {
    pub schema: String,
    pub product: String,
    pub version: String,
    pub viewport: String,
    pub seat: String,
    pub nodes: Vec<NavNode>,
}
impl UiNav {
    pub fn stem(&self) -> String {
        format!("{}-{}", self.viewport, self.seat)
    }
    pub fn problems(&self) -> Vec<String> {
        let mut p = vec![];
        if self.schema != NAV_SCHEMA {
            p.push(format!("schema `{}` is not `{NAV_SCHEMA}`", self.schema));
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
        let mut orders = BTreeSet::new();
        for n in &self.nodes {
            if n.label.trim().is_empty() {
                p.push(format!("node {} has an empty label", n.order));
            }
            if !NAV_KINDS.contains(&n.kind.as_str()) {
                p.push(format!("node `{}` kind `{}` must be one of {}", n.label, n.kind, NAV_KINDS.join("|")));
            }
            if n.level == 0 {
                p.push(format!("node `{}` level must be >= 1", n.label));
            }
            if !orders.insert(n.order) {
                p.push(format!("node order {} appears twice", n.order));
            }
            if !(matches!(n.target.as_str(), "external" | "unknown") || token_ok(&n.target)) {
                p.push(format!("node `{}` target `{}` is not a screen id, `external` or `unknown`", n.label, n.target));
            }
            if n.hidden_behind.as_ref().is_some_and(|h| !HIDDEN.contains(&h.as_str())) {
                p.push(format!("node `{}` hiddenBehind must be {}", n.label, HIDDEN.join("|")));
            }
        }
        p
    }
    /// Nodes in DOM order.
    pub fn ordered(&self) -> Vec<&NavNode> {
        let mut v: Vec<&NavNode> = self.nodes.iter().collect();
        v.sort_by_key(|n| n.order);
        v
    }
}

/// The numbers derived from a tree (section 8a).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavStats {
    pub nodes: usize,
    pub places: usize,
    pub max_depth: u32,
    pub items_per_level: BTreeMap<u32, usize>,
    pub items_per_group: BTreeMap<String, usize>,
    /// distinct registry screens the tree can reach
    pub destinations: usize,
    /// extra nodes that point at a screen another node already points at
    pub duplicate_destinations: usize,
    pub dead_items: usize,
    /// nodes whose screen is reachable only through an overflow menu
    pub overflow_only_items: usize,
    #[serde(skip)]
    pub screens: BTreeSet<String>,
}
pub fn nav_stats(nav: &UiNav) -> NavStats {
    let nodes = nav.ordered();
    let is_screen = |t: &str| !matches!(t, "external" | "unknown");
    let mut per_level = BTreeMap::new();
    let mut per_group: BTreeMap<String, usize> = BTreeMap::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut open: BTreeSet<&str> = BTreeSet::new();
    for (i, n) in nodes.iter().enumerate() {
        *per_level.entry(n.level).or_insert(0) += 1;
        if n.kind == "group" {
            let kids = nodes[i + 1..].iter().take_while(|m| m.level > n.level).filter(|m| m.level == n.level + 1).count();
            *per_group.entry(n.label.clone()).or_insert(0) += kids;
        }
        if is_screen(&n.target) {
            *counts.entry(n.target.as_str()).or_insert(0) += 1;
            if n.hidden_behind.as_deref() != Some("overflow") {
                open.insert(n.target.as_str());
            }
        }
    }
    let overflow_only = nodes.iter().filter(|n| is_screen(&n.target) && n.hidden_behind.as_deref() == Some("overflow") && !open.contains(n.target.as_str())).count();
    NavStats {
        nodes: nodes.len(),
        places: nodes.iter().filter(|n| n.level == 1).count(),
        max_depth: nodes.iter().map(|n| n.level).max().unwrap_or(0),
        items_per_level: per_level,
        items_per_group: per_group,
        destinations: counts.len(),
        duplicate_destinations: counts.values().map(|c| c - 1).sum(),
        dead_items: nodes.iter().filter(|n| n.target == "unknown").count(),
        overflow_only_items: overflow_only,
        screens: counts.keys().map(|s| s.to_string()).collect(),
    }
}
/// The tree difference in words, `a vs b`, fixed order, only where the two differ.
pub fn nav_diff(a: &NavStats, b: &NavStats) -> Vec<String> {
    let mut out = vec![];
    let o = &mut out;
    vs(o, "places", a.places as f64, b.places as f64);
    vs(o, "max depth", a.max_depth as f64, b.max_depth as f64);
    vs(o, "nodes", a.nodes as f64, b.nodes as f64);
    let levels: BTreeSet<u32> = a.items_per_level.keys().chain(b.items_per_level.keys()).copied().collect();
    for l in levels {
        vs(o, &format!("items at level {l}"), *a.items_per_level.get(&l).unwrap_or(&0) as f64, *b.items_per_level.get(&l).unwrap_or(&0) as f64);
    }
    vs(o, "destinations", a.destinations as f64, b.destinations as f64);
    vs(o, "duplicate destinations", a.duplicate_destinations as f64, b.duplicate_destinations as f64);
    vs(o, "dead items", a.dead_items as f64, b.dead_items as f64);
    vs(o, "items only via overflow", a.overflow_only_items as f64, b.overflow_only_items as f64);
    let only_a: Vec<&str> = a.screens.difference(&b.screens).map(String::as_str).collect();
    let only_b: Vec<&str> = b.screens.difference(&a.screens).map(String::as_str).collect();
    if !only_a.is_empty() {
        out.push(format!("screens only in ours: {}", only_a.join(", ")));
    }
    if !only_b.is_empty() {
        out.push(format!("screens only in ref: {}", only_b.join(", ")));
    }
    out
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JourneyResult {
    pub status: String,
    #[serde(default)]
    pub failed_step: Option<u64>,
    #[serde(default)]
    pub why: Option<String>,
    #[serde(default)]
    pub screens: u64,
    #[serde(default)]
    pub clicks: u64,
    #[serde(default)]
    pub decisions: u64,
    #[serde(default)]
    pub waits: u64,
    #[serde(default)]
    pub dead_ends: u64,
    /// steps until the first screen whose archetype is the goal's
    #[serde(default)]
    pub first_useful: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiJourney {
    pub schema: String,
    pub journey: String,
    pub title: String,
    #[serde(default)]
    pub goal_archetype: Option<String>,
    pub results: BTreeMap<String, JourneyResult>,
}
impl UiJourney {
    pub fn problems(&self) -> Vec<String> {
        let mut p = vec![];
        if self.schema != JOURNEY_SCHEMA {
            p.push(format!("schema `{}` is not `{JOURNEY_SCHEMA}`", self.schema));
        }
        if !token_ok(&self.journey) {
            p.push(format!("journey `{}` is not a path-safe token", self.journey));
        }
        for (prod, r) in &self.results {
            if !token_ok(prod) {
                p.push(format!("result product `{prod}` is not a path-safe token"));
            }
            if !JOURNEY_STATUS.contains(&r.status.as_str()) {
                p.push(format!("{prod}: status `{}` must be one of {}", r.status, JOURNEY_STATUS.join("|")));
            }
            let why = r.why.as_deref().is_some_and(|w| !w.trim().is_empty());
            if r.status == "n/a" && !why {
                p.push(format!("{prod}: n/a needs a why"));
            }
            if r.status == "partial" && (r.failed_step.is_none() || !why) {
                p.push(format!("{prod}: partial needs failedStep and why"));
            }
        }
        p
    }
}
/// "completed" | "partial at step N: why" | "n/a: why"
pub fn journey_line(r: &JourneyResult) -> String {
    let why = r.why.as_deref().unwrap_or("");
    match r.status.as_str() {
        "partial" => format!("partial at step {}: {why}", r.failed_step.map(|n| n.to_string()).unwrap_or_default()),
        "n/a" => format!("n/a: {why}"),
        s => s.to_string(),
    }
}
/// The journey difference in words between two results (`a vs b`).
pub fn journey_diff(a: &JourneyResult, b: &JourneyResult) -> Vec<String> {
    let mut out = vec![];
    let o = &mut out;
    vs(o, "screens", a.screens as f64, b.screens as f64);
    vs(o, "clicks", a.clicks as f64, b.clicks as f64);
    vs(o, "decisions", a.decisions as f64, b.decisions as f64);
    vs(o, "waits", a.waits as f64, b.waits as f64);
    vs(o, "dead ends", a.dead_ends as f64, b.dead_ends as f64);
    if let (Some(x), Some(y)) = (a.first_useful, b.first_useful) {
        vs(o, "time to first useful (steps)", x as f64, y as f64);
    }
    out
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverageCell {
    pub product: String,
    pub screen: String,
    pub state: String,
    pub viewport: String,
    pub seat: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub why: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JourneyCell {
    pub product: String,
    pub journey: String,
    pub step: u64,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub why: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiCoverage {
    pub schema: String,
    pub run_id: String,
    pub cells: Vec<CoverageCell>,
    #[serde(default)]
    pub journey_cells: Vec<JourneyCell>,
}
impl UiCoverage {
    pub fn problems(&self, stem: &str) -> Vec<String> {
        let mut p = vec![];
        if self.schema != COVERAGE_SCHEMA {
            p.push(format!("schema `{}` is not `{COVERAGE_SCHEMA}`", self.schema));
        }
        if self.run_id != stem {
            p.push(format!("runId `{}` does not match the file name `{stem}`", self.run_id));
        }
        p
    }
}
/// A cell is blank when its status is not one of captured|n/a|failed, or n/a/failed carries no reason.
pub fn cell_blank(status: &str, why: &str) -> bool {
    !CELL_STATUS.contains(&status) || (status != "captured" && why.trim().is_empty())
}
/// Per-product counts and the sorted list of blank cells.
pub fn coverage_table(c: &UiCoverage) -> (BTreeMap<String, BTreeMap<&'static str, usize>>, Vec<String>) {
    let mut table: BTreeMap<String, BTreeMap<&'static str, usize>> = BTreeMap::new();
    let mut blanks = vec![];
    let mut add = |product: &str, status: &str, why: &str, label: String| {
        let row = table.entry(product.to_string()).or_default();
        if cell_blank(status, why) {
            *row.entry("blank").or_insert(0) += 1;
            blanks.push(label);
        } else {
            let k = match status {
                "captured" => "captured",
                "n/a" => "n/a",
                _ => "failed",
            };
            *row.entry(k).or_insert(0) += 1;
        }
    };
    for x in &c.cells {
        add(&x.product, &x.status, &x.why, format!("{}/{}/{}/{}-{}", x.product, x.screen, x.state, x.viewport, x.seat));
    }
    for x in &c.journey_cells {
        add(&x.product, &x.status, &x.why, format!("{}/journey {}/step {}", x.product, x.journey, x.step));
    }
    blanks.sort();
    (table, blanks)
}
/// Percent as the sweep report writes it.
pub fn share(n: usize, of: usize) -> String {
    if of == 0 {
        "-".into()
    } else {
        format!("{}%", fmt_num((n as f64 * 1000.0 / of as f64).round() / 10.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(label: &str, kind: &str, level: u32, order: u32, target: &str, hidden: Option<&str>) -> NavNode {
        NavNode { label: label.into(), kind: kind.into(), level, order, target: target.into(), icon: false, badge: false, hidden_behind: hidden.map(String::from) }
    }
    fn tree() -> UiNav {
        UiNav {
            schema: NAV_SCHEMA.into(), product: "p".into(), version: "v1".into(), viewport: "1440x900".into(), seat: "member".into(),
            nodes: vec![
                node("Home", "place", 1, 1, "room", None),
                node("Work", "group", 1, 2, "unknown", None),
                node("Flows", "item", 2, 3, "flows", None),
                node("Flows again", "item", 2, 4, "flows", None),
                node("Old", "item", 2, 5, "unknown", Some("collapsed")),
                node("Billing", "menuitem", 2, 6, "billing", Some("overflow")),
            ],
        }
    }
    #[test]
    fn derived_numbers() {
        let s = nav_stats(&tree());
        assert_eq!((s.places, s.max_depth, s.nodes), (2, 2, 6));
        assert_eq!(s.items_per_level[&2], 4);
        assert_eq!(s.items_per_group["Work"], 4);
        assert_eq!((s.destinations, s.duplicate_destinations, s.dead_items, s.overflow_only_items), (3, 1, 2, 1));
    }
    #[test]
    fn nav_rules_go_red() {
        let mut t = tree();
        t.nodes[0].kind = "bogus".into();
        t.nodes[1].order = 1;
        assert_eq!(t.problems().len(), 2);
    }
    #[test]
    fn blank_cells() {
        assert!(cell_blank("", ""));
        assert!(cell_blank("failed", ""));
        assert!(!cell_blank("n/a", "no equivalent"));
        assert!(!cell_blank("captured", ""));
        assert_eq!(share(1, 3), "33.3%");
    }
}
