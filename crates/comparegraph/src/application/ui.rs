//! UI evidence verbs: read a `ui/` tree of evidence files, validate, sort and compare. Reads files only - no store, no network.
//! Layout and formats: CMP-UI-1-lanes.md (ui-evidence/1, ui-flow/1, ui-run/1, ui-registry/1, ui-nav/1, ui-journey/1, ui-coverage/1).
#![allow(clippy::result_large_err)] // serde_json::Value is larger with arbitrary_precision
use super::{AppError, Result};
use crate::domain::ui::*;
use crate::domain::ui_graph::UiGraph;
use crate::domain::ui_nav::*;
use crate::domain::Limitation;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const DEFAULT_MAX_NODES: usize = 200;
pub const MAX_NODES: usize = 2000;
const MAX_FILE: u64 = 1 << 20;
/// Top-level directories of `ui/` that are not products.
const TOP_RESERVED: &[&str] = &["blobs", "snapshots", "runs", "journeys", "coverage"];
const SEAT_PREFERENCE: &[&str] = &["member", "admin", "ref", "operator", "admin-dev", "guest"];
const SNAPSHOT_NOTE: &str = "metrics, layout and style are read from the evidence JSON as written; recomputing them from ui/snapshots/<domHash>.json.gz is pending a dependency ruling (no gzip reader in Cargo.lock)";

pub struct Loaded {
    pub rel: String,
    pub ev: UiEvidence,
}
#[derive(Clone, Copy, Default)]
pub struct Want {
    pub evidence: bool,
    pub flows: bool,
    pub navs: bool,
    pub systems: bool,
    pub graphs: bool,
}
pub struct UiSet {
    /// the `ui-root` as given, for the paths we print
    pub shown: String,
    pub dir: PathBuf,
    /// true when the files live in `<ui-root>/ui/`
    pub nested: bool,
    pub products: BTreeSet<String>,
    pub evidence: Vec<Loaded>,
    pub flows: Vec<(String, UiFlow)>,
    pub navs: Vec<(String, UiNav)>,
    pub graphs: Vec<(String, UiGraph)>,
    pub systems: BTreeMap<String, std::result::Result<Style, String>>,
    pub map: Vec<MapRow>,
    pub registry: Option<UiRegistry>,
    pub invalid: Vec<(String, String)>,
    pub truncated: bool,
    max_nodes: usize,
    count: usize,
}

fn bad(m: String) -> AppError {
    AppError::InvalidArguments(m)
}
pub(super) fn need_token(what: &str, v: &str) -> Result<()> {
    if token_ok(v) {
        Ok(())
    } else {
        Err(bad(format!("{what} `{v}` must be letters, digits, `-`, `_` or `.` (no separators)")))
    }
}
pub fn check_max(max_nodes: usize) -> Result<usize> {
    if (1..=MAX_NODES).contains(&max_nodes) {
        Ok(max_nodes)
    } else {
        Err(bad(format!("--max-nodes {max_nodes} must be 1..{MAX_NODES}")))
    }
}
fn read_text(path: &Path) -> std::result::Result<String, String> {
    let m = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if m.file_type().is_symlink() {
        return Err("symlink skipped".into());
    }
    if m.len() > MAX_FILE {
        return Err(format!("larger than {MAX_FILE} bytes"));
    }
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}
fn read_json<T: DeserializeOwned>(path: &Path) -> std::result::Result<T, String> {
    serde_json::from_str(&read_text(path)?).map_err(|e| e.to_string())
}
/// Sorted (name, path, is_dir) of a directory; symlinks are left out.
fn entries(dir: &Path) -> Result<Vec<(String, PathBuf, bool)>> {
    let rd = std::fs::read_dir(dir).map_err(|e| AppError::Io(dir.to_path_buf(), e))?;
    let mut v = vec![];
    for e in rd.flatten() {
        let Ok(t) = e.file_type() else { continue };
        if t.is_symlink() {
            continue;
        }
        if let Some(n) = e.file_name().to_str() {
            v.push((n.to_string(), e.path(), t.is_dir()));
        }
    }
    v.sort();
    Ok(v)
}
fn slash(p: &str) -> String {
    p.replace('\\', "/")
}

impl UiSet {
    fn take(&mut self) -> bool {
        if self.count >= self.max_nodes {
            self.truncated = true;
            return false;
        }
        self.count += 1;
        true
    }
    /// The path of a blob as the caller can open it: `<ui-root>[/ui]/blobs/<sha>.png`.
    pub fn blob_path(&self, file: &str) -> String {
        slash(&if self.nested { format!("{}/ui/{file}", self.shown.trim_end_matches(['/', '\\'])) } else { format!("{}/{file}", self.shown.trim_end_matches(['/', '\\'])) })
    }
    pub fn blob_present(&self, file: &str) -> bool {
        self.dir.join(file).is_file()
    }
    fn image(&self, im: &ImageRef) -> Value {
        json!({ "sha256": im.sha256, "dhash": im.dhash, "path": self.blob_path(&im.file), "present": self.blob_present(&im.file) })
    }
    pub fn evidence_of<'a>(&'a self, product: &str) -> impl Iterator<Item = &'a Loaded> + 'a {
        let p = product.to_string();
        self.evidence.iter().filter(move |l| l.ev.product == p)
    }
    fn limitations(&self, extra: &[Limitation]) -> Value {
        let mut l: Vec<Limitation> = vec![];
        if self.truncated {
            l.push(Limitation::new("truncated", format!("stopped after {} files (--max-nodes); raise it to read more", self.max_nodes)));
        }
        for (rel, why) in self.invalid.iter().take(20) {
            l.push(Limitation::new("invalid_evidence", format!("{rel}: {why}")));
        }
        if self.invalid.len() > 20 {
            l.push(Limitation::new("invalid_evidence", format!("and {} more invalid files", self.invalid.len() - 20)));
        }
        l.extend(extra.iter().cloned());
        serde_json::to_value(l).unwrap()
    }
    /// Attach the evidence limitations to an answer.
    pub(super) fn finish(&self, mut v: Value, extra: &[Limitation]) -> Value {
        v["limitation_items"] = self.limitations(extra);
        v
    }
}

/// Open a ui root: `<root>/ui` when it exists, else the root itself when it looks like a ui dir.
pub fn open(root: &Path, max_nodes: usize, only: Option<&[&str]>, want: Want) -> Result<UiSet> {
    let max_nodes = check_max(max_nodes)?;
    let nested = root.join("ui").is_dir();
    let dir = if nested { root.join("ui") } else { root.to_path_buf() };
    if !dir.is_dir() {
        return Err(AppError::NotFound(format!("no ui directory at {}", root.display())));
    }
    let mut set = UiSet {
        shown: slash(&root.display().to_string()), dir: dir.clone(), nested, products: BTreeSet::new(), evidence: vec![], flows: vec![], navs: vec![], graphs: vec![],
        systems: BTreeMap::new(), map: vec![], registry: None, invalid: vec![], truncated: false, max_nodes, count: 0,
    };
    if std::fs::symlink_metadata(dir.join("map.csv")).is_ok() {
        match read_text(&dir.join("map.csv")).and_then(|t| parse_map(&t)) {
            Ok(m) => set.map = m,
            Err(e) => set.invalid.push(("map.csv".into(), e)),
        }
    }
    if dir.join("registry.json").exists() {
        match read_json::<UiRegistry>(&dir.join("registry.json")) {
            Ok(r) if r.problems().is_empty() => set.registry = Some(r),
            Ok(r) => set.invalid.push(("registry.json".into(), r.problems().join("; "))),
            Err(e) => set.invalid.push(("registry.json".into(), e)),
        }
    }
    // section 11: the design lane's per-archetype limits (ui-archetypes/1) fill the registry's; other keys (rowHeightMaxPx, overrides, rules) stay in the file
    if dir.join("archetypes.json").exists() {
        match read_json::<UiArchetypes>(&dir.join("archetypes.json")) {
            Ok(a) if a.problems().is_empty() => {
                if let Some(r) = set.registry.as_mut() {
                    for (name, e) in a.archetypes {
                        r.archetypes.entry(name).or_insert(e.limits);
                    }
                }
            }
            Ok(a) => set.invalid.push(("archetypes.json".into(), a.problems().join("; "))),
            Err(e) => set.invalid.push(("archetypes.json".into(), e)),
        }
    }
    for (product, ppath, is_dir) in entries(&dir)? {
        if !is_dir || TOP_RESERVED.contains(&product.as_str()) || !token_ok(&product) {
            continue;
        }
        set.products.insert(product.clone());
        if only.is_some_and(|o| !o.contains(&product.as_str())) {
            continue;
        }
        for (name, path, is_dir) in entries(&ppath)? {
            if !is_dir {
                if name == "system.json" && want.systems && set.take() {
                    let r = read_json::<Value>(&path).and_then(|v| {
                        let s = v.get("style").cloned().unwrap_or(v);
                        serde_json::from_value::<Style>(s).map_err(|e| e.to_string())
                    });
                    set.systems.insert(product.clone(), r);
                }
                continue;
            }
            match name.as_str() {
                "flows" if want.flows => {
                    for (f, fp, d) in entries(&path)? {
                        if d || !f.ends_with(".json") || !set.take() {
                            continue;
                        }
                        let rel = format!("{product}/flows/{f}");
                        match read_json::<UiFlow>(&fp) {
                            Ok(fl) => {
                                let mut p = fl.problems();
                                if fl.product != product || format!("{}.json", fl.task) != f {
                                    p.push("product or task does not match the path".into());
                                }
                                if p.is_empty() { set.flows.push((rel, fl)) } else { set.invalid.push((rel, p.join("; "))) }
                            }
                            Err(e) => set.invalid.push((rel, e)),
                        }
                    }
                }
                "nav" if want.navs => {
                    for (f, fp, d) in entries(&path)? {
                        if d || !f.ends_with(".json") || !set.take() {
                            continue;
                        }
                        let rel = format!("{product}/nav/{f}");
                        match read_json::<UiNav>(&fp) {
                            Ok(n) => {
                                let mut p = n.problems();
                                if n.product != product || format!("{}.json", n.stem()) != f {
                                    p.push("product, viewport or seat does not match the path".into());
                                }
                                if p.is_empty() { set.navs.push((rel, n)) } else { set.invalid.push((rel, p.join("; "))) }
                            }
                            Err(e) => set.invalid.push((rel, e)),
                        }
                    }
                }
                "graph" if want.graphs => {
                    for (f, fp, d) in entries(&path)? {
                        if d || !f.ends_with(".json") || !set.take() {
                            continue;
                        }
                        let rel = format!("{product}/graph/{f}");
                        match read_json::<UiGraph>(&fp) {
                            Ok(g) => {
                                let mut p = g.problems();
                                if g.product != product || format!("{}.json", g.stem()) != f {
                                    p.push("product, viewport or seat does not match the path".into());
                                }
                                if p.is_empty() { set.graphs.push((rel, g)) } else { set.invalid.push((rel, p.join("; "))) }
                            }
                            Err(e) => set.invalid.push((rel, e)),
                        }
                    }
                }
                "flows" | "nav" | "graph" | "graph-log" => {}
                _ if want.evidence => {
                    for (state, spath, sd) in entries(&path)? {
                        if !sd {
                            continue;
                        }
                        for (f, fp, d) in entries(&spath)? {
                            if d || !f.ends_with(".json") || !set.take() {
                                continue;
                            }
                            let rel = format!("{product}/{name}/{state}/{f}");
                            match read_json::<UiEvidence>(&fp) {
                                Ok(ev) => {
                                    let mut p = ev.problems();
                                    if ev.product != product || ev.screen != name || ev.state != state || format!("{}.json", ev.stem()) != f {
                                        p.push("product, screen, state, viewport or seat does not match the path".into());
                                    }
                                    if p.is_empty() { set.evidence.push(Loaded { rel, ev }) } else { set.invalid.push((rel, p.join("; "))) }
                                }
                                Err(e) => set.invalid.push((rel, e)),
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(set)
}
const EV: Want = Want { evidence: true, flows: false, navs: false, systems: false, graphs: false };

fn screens_of<'a>(set: &'a UiSet, product: &str) -> BTreeSet<&'a str> {
    set.evidence_of(product).map(|l| l.ev.screen.as_str()).collect()
}
pub(super) fn need_product_pub(set: &UiSet, product: &str) -> Result<()> {
    if set.products.contains(product) {
        Ok(())
    } else {
        Err(AppError::NotFound(format!("no product `{product}` (products: {})", set.products.iter().cloned().collect::<Vec<_>>().join(", "))))
    }
}
fn available(cands: &[&Loaded]) -> String {
    cands.iter().map(|l| format!("{}/{}", l.ev.state, l.ev.stem())).collect::<Vec<_>>().join(", ")
}
pub(super) fn seat_rank(seat: &str) -> usize {
    SEAT_PREFERENCE.iter().position(|s| *s == seat).unwrap_or(SEAT_PREFERENCE.len())
}
/// The one evidence file for product/screen/state; `viewport` None prefers 1440x900. Never crosses state.
fn select<'a>(set: &'a UiSet, product: &str, screen: &str, state: &str, viewport: Option<&str>, seat: Option<&str>) -> Result<&'a Loaded> {
    need_product_pub(set, product)?;
    let of_screen: Vec<&Loaded> = set.evidence_of(product).filter(|l| l.ev.screen == screen).collect();
    if of_screen.is_empty() {
        return Err(AppError::NotFound(format!("product `{product}` has no screen `{screen}` (screens: {})", screens_of(set, product).into_iter().collect::<Vec<_>>().join(", "))));
    }
    let in_state: Vec<&Loaded> = of_screen.iter().copied().filter(|l| l.ev.state == state).collect();
    if in_state.is_empty() {
        return Err(AppError::NotFound(format!("`{product}/{screen}` has no state `{state}` (captured: {})", available(&of_screen))));
    }
    let mut c: Vec<&Loaded> = in_state.iter().copied().filter(|l| viewport.is_none_or(|v| l.ev.viewport == v) && seat.is_none_or(|s| l.ev.seat == s)).collect();
    if c.is_empty() {
        return Err(AppError::NotFound(format!("`{product}/{screen}/{state}` has no capture for viewport {} seat {} (captured: {})", viewport.unwrap_or("any"), seat.unwrap_or("any"), available(&in_state))));
    }
    if viewport.is_none() && c.iter().any(|l| l.ev.viewport == DEFAULT_VIEWPORT) {
        c.retain(|l| l.ev.viewport == DEFAULT_VIEWPORT);
    }
    c.sort_by(|a, b| (seat_rank(&a.ev.seat), &a.ev.viewport, &a.ev.seat).cmp(&(seat_rank(&b.ev.seat), &b.ev.viewport, &b.ev.seat)));
    Ok(c[0])
}

fn archetype_limits(set: &UiSet, screen: &str) -> (Option<String>, BTreeMap<String, Value>) {
    match set.registry.as_ref().and_then(|r| r.archetype_of(screen).map(|a| (a.to_string(), r.archetypes.get(a).cloned().unwrap_or_default()))) {
        Some((a, l)) => (Some(a), l),
        None => (None, BTreeMap::new()),
    }
}
fn registry_limits(set: &UiSet, screen: &str) -> Vec<Limitation> {
    let mut v = vec![];
    if set.registry.is_none() {
        v.push(Limitation::new("registry_missing", "ui/registry.json is absent or invalid: no archetype and no per-archetype limits"));
    } else if archetype_limits(set, screen).0.is_none() {
        v.push(Limitation::new("screen_not_in_registry", format!("screen `{screen}` is not in ui/registry.json")));
    }
    v
}
fn metric_ids_sorted(a: &UiEvidence, b: Option<&UiEvidence>) -> Vec<String> {
    let mut ids: BTreeSet<(usize, String)> = BTreeSet::new();
    for m in a.metrics.keys().chain(b.iter().flat_map(|e| e.metrics.keys())) {
        ids.insert((metric_no(m).unwrap_or(99), m.clone()));
    }
    ids.into_iter().map(|(_, m)| m).collect()
}

pub fn products(root: &Path, max_nodes: usize) -> Result<Value> {
    let set = open(root, max_nodes, None, Want { evidence: true, flows: true, navs: true, systems: true, graphs: false })?;
    let items: Vec<Value> = set
        .products
        .iter()
        .map(|p| {
            let ev: Vec<&Loaded> = set.evidence_of(p).collect();
            let states: BTreeSet<&str> = ev.iter().map(|l| l.ev.state.as_str()).collect();
            json!({
                "product": p, "screens": screens_of(&set, p).len(), "evidenceFiles": ev.len(), "states": states,
                "flows": set.flows.iter().filter(|(_, f)| &f.product == p).count(),
                "navs": set.navs.iter().filter(|(_, n)| &n.product == p).count(),
                "system": set.systems.contains_key(p),
            })
        })
        .collect();
    let total = items.len();
    Ok(set.finish(json!({ "items": items, "page": { "next": null, "total": total } }), &[]))
}

pub fn screens(root: &Path, max_nodes: usize, product: &str) -> Result<Value> {
    need_token("product", product)?;
    let set = open(root, max_nodes, Some(&[product]), EV)?;
    need_product_pub(&set, product)?;
    let mut items = vec![];
    for s in screens_of(&set, product) {
        let files: Vec<&Loaded> = set.evidence_of(product).filter(|l| l.ev.screen == s).collect();
        let (arch, _) = archetype_limits(&set, s);
        let states: BTreeSet<&str> = files.iter().map(|l| l.ev.state.as_str()).collect();
        let best = files.iter().map(|l| l.ev.pass_count()).max().unwrap_or(0);
        items.push(json!({
            "screen": s, "archetype": arch, "route": files[0].ev.route, "states": states,
            "captures": files.iter().map(|l| format!("{}/{}", l.ev.state, l.ev.stem())).collect::<Vec<_>>(), "bestPass": best, "of": METRIC_COUNT,
        }));
    }
    let total = items.len();
    Ok(set.finish(json!({ "product": product, "items": items, "page": { "next": null, "total": total } }), &[Limitation::new("snapshot_recompute_pending", SNAPSHOT_NOTE)]))
}

fn one_side(set: &UiSet, l: &Loaded) -> Value {
    let e = &l.ev;
    json!({ "product": e.product, "version": e.version, "seat": e.seat, "route": e.route, "domHash": e.dom_hash, "pass": e.pass_count(), "image": set.image(&e.image) })
}

pub fn show(root: &Path, max_nodes: usize, product: &str, screen: &str, state: &str, viewport: Option<&str>, seat: Option<&str>) -> Result<Value> {
    need_token("product", product)?;
    need_token("screen", screen)?;
    let set = open(root, max_nodes, Some(&[product]), EV)?;
    let l = select(&set, product, screen, state, viewport, seat)?;
    let e = &l.ev;
    let (arch, lim) = archetype_limits(&set, screen);
    let metrics: Vec<Value> = metric_ids_sorted(e, None)
        .iter()
        .map(|id| {
            let m = &e.metrics[id];
            json!({ "id": id, "value": m.value, "limit": m.limit, "pass": m.pass, "detail": m.detail, "archetypeLimit": lim.get(id) })
        })
        .collect();
    let others: Vec<String> = set.evidence_of(product).filter(|x| x.ev.screen == screen).map(|x| format!("{}/{}", x.ev.state, x.ev.stem())).collect();
    let v = json!({
        "key": e.capture_key(), "product": e.product, "version": e.version, "screen": e.screen, "archetype": arch, "state": e.state, "route": e.route,
        "viewport": e.viewport, "seat": e.seat, "locale": e.locale, "theme": e.theme, "domHash": e.dom_hash,
        "pass": e.pass_count(), "of": METRIC_COUNT, "metrics": metrics,
        "layout": { "summary": layout_summary(&e.layout), "navColumns": nav_columns(&e.layout), "regions": e.layout.regions, "primary": e.layout.primary, "contentStart": e.layout.content_start },
        "style": { "summary": style_summary(&e.style), "style": e.style },
        "image": set.image(&e.image), "captures": others,
    });
    let mut v = v;
    if let Some(n) = &e.net {
        v["net"] = json!(n);
        v["netLine"] = json!(n.line());
    }
    let mut lims = registry_limits(&set, screen);
    lims.push(Limitation::new("snapshot_recompute_pending", SNAPSHOT_NOTE));
    Ok(set.finish(v, &lims))
}

/// Resolve a diff target (a screen id, or a map.csv row id) to the screen of one product.
fn resolve_screen(set: &UiSet, product: &str, target: &str) -> Result<String> {
    if screens_of(set, product).contains(target) {
        return Ok(target.to_string());
    }
    set.map
        .iter()
        .find(|m| m.row_id == target && m.product == product)
        .map(|m| m.screen.clone())
        .ok_or_else(|| AppError::NotFound(format!("`{target}` is neither a screen of `{product}` nor a map.csv row mapped for it")))
}

#[allow(clippy::too_many_arguments)]
pub fn diff(root: &Path, max_nodes: usize, target: &str, ours: &str, refp: &str, state: &str, viewport: Option<&str>, seat: Option<&str>, ref_seat: Option<&str>) -> Result<Value> {
    for (w, v) in [("target", target), ("--ours", ours), ("--ref", refp)] {
        need_token(w, v)?;
    }
    let set = open(root, max_nodes, Some(&[ours, refp]), EV)?;
    need_product_pub(&set, ours)?;
    need_product_pub(&set, refp)?;
    let (so, sr) = (resolve_screen(&set, ours, target)?, resolve_screen(&set, refp, target)?);
    if so != sr {
        return Err(bad(format!("refused: `{target}` is screen `{so}` for {ours} but `{sr}` for {refp}; a comparison is only within the same screen, state and viewport")));
    }
    let vp = viewport.unwrap_or(DEFAULT_VIEWPORT);
    let (a, b) = (select(&set, ours, &so, state, Some(vp), seat)?, select(&set, refp, &sr, state, Some(vp), ref_seat)?);
    let (ea, eb) = (&a.ev, &b.ev);
    let (arch, lim) = archetype_limits(&set, &so);
    let metrics: Vec<Value> = metric_ids_sorted(ea, Some(eb))
        .iter()
        .map(|id| {
            let side = |e: &UiEvidence| e.metrics.get(id).map(|m| json!({ "value": m.value, "limit": m.limit, "pass": m.pass }));
            json!({ "id": id, "ours": side(ea), "ref": side(eb), "archetypeLimit": lim.get(id) })
        })
        .collect();
    let v = json!({
        "target": target, "screen": so, "archetype": arch, "state": state, "viewport": vp,
        "ours": one_side(&set, a), "ref": one_side(&set, b),
        "pass": { "ours": ea.pass_count(), "ref": eb.pass_count(), "of": METRIC_COUNT },
        "metrics": metrics,
        "layoutDiff": layout_diff(&ea.layout, &eb.layout),
        "styleDiff": style_diff(&ea.style, &eb.style),
        "images": { "ours": set.blob_path(&ea.image.file), "ref": set.blob_path(&eb.image.file) },
    });
    let mut v = v;
    if ea.net.is_some() || eb.net.is_some() {
        let side = |e: &UiEvidence| e.net.as_ref().map_or("no net recorded".to_string(), |n| n.line());
        v["net"] = json!({ "ours": ea.net, "ref": eb.net });
        v["netLine"] = json!(format!("ours: {} | ref: {}", side(ea), side(eb)));
    }
    let mut lims = registry_limits(&set, &so);
    lims.push(Limitation::new("snapshot_recompute_pending", SNAPSHOT_NOTE));
    Ok(set.finish(v, &lims))
}

/// Family rows: `<ui-root>/families/<family>/rows.md`, else `<root>/families/<family>/rows.md`.
fn family_rows(ui_root: &Path, root: &Path, family: &str) -> Option<Vec<String>> {
    for base in [ui_root, root] {
        if let Ok(t) = std::fs::read_to_string(base.join("families").join(family).join("rows.md")) {
            if let Ok(p) = crate::application::family::parse_pack(&t) {
                return Some(p.rows.into_iter().map(|r| r.id).collect());
            }
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub fn matrix(root: &Path, ui_root: &Path, max_nodes: usize, family: &str, state: &str, viewport: Option<&str>, net: bool) -> Result<Value> {
    need_token("family", family)?;
    let set = open(ui_root, max_nodes, None, EV)?;
    let vp = viewport.unwrap_or(DEFAULT_VIEWPORT);
    let mapped: Vec<&MapRow> = set.map.iter().filter(|m| m.family == family).collect();
    let mut lims = vec![];
    let ids = match family_rows(ui_root, root, family) {
        Some(ids) => ids,
        None if !mapped.is_empty() => {
            lims.push(Limitation::new("family_pack_missing", format!("families/{family}/rows.md not found: rows are the ids in map.csv only")));
            mapped.iter().map(|m| m.row_id.clone()).collect::<BTreeSet<_>>().into_iter().collect()
        }
        None => return Err(AppError::NotFound(format!("family `{family}` has no rows.md and no map.csv rows"))),
    };
    let products: Vec<&String> = set.products.iter().collect();
    let mut rows = vec![];
    for id in &ids {
        let mut cells = BTreeMap::new();
        for p in &products {
            let best = mapped
                .iter()
                .filter(|m| &m.row_id == id && &&m.product == p)
                .filter_map(|m| set.evidence_of(p).filter(|l| l.ev.screen == m.screen && l.ev.state == state && l.ev.viewport == vp).map(|l| l.ev.pass_count()).max())
                .max();
            cells.insert(p.to_string(), best.map_or("-".to_string(), |n| format!("{n}/{METRIC_COUNT}")));
        }
        rows.push(json!({ "row": id, "cells": cells }));
    }
    let total = rows.len();
    let mut out = json!({ "family": family, "state": state, "viewport": vp, "products": products, "items": rows, "page": { "next": null, "total": total } });
    if net {
        // `--net`: every capture of the products with failed calls (all states and viewports), so a 5xx on a screen cannot hide in a green cell
        let mut failing: Vec<Value> = vec![];
        for p in &products {
            for l in set.evidence_of(p) {
                if let Some(n) = l.ev.net.as_ref().filter(|n| !n.failed.is_empty()) {
                    failing.push(json!({ "product": l.ev.product, "screen": l.ev.screen, "state": l.ev.state, "viewport": l.ev.viewport, "seat": l.ev.seat, "failed": n.failed }));
                }
            }
        }
        let with_net = products.iter().map(|p| set.evidence_of(p).filter(|l| l.ev.net.is_some()).count()).sum::<usize>();
        if with_net == 0 {
            lims.push(Limitation::new("net_missing", "no capture records `net`: nothing to list (captured before CMP-UI-1e?)"));
        }
        out["failedCalls"] = json!(failing);
    }
    Ok(set.finish(out, &lims))
}

pub fn system(root: &Path, max_nodes: usize, product: &str) -> Result<Value> {
    need_token("product", product)?;
    let set = open(root, max_nodes, Some(&[product]), Want { evidence: true, systems: true, ..Default::default() })?;
    need_product_pub(&set, product)?;
    let mut lims = vec![];
    let (source, style, unioned) = match set.systems.get(product) {
        Some(Ok(s)) => ("system.json", s.clone(), 0),
        Some(Err(e)) => return Err(bad(format!("{product}/system.json: {e}"))),
        None => {
            let all: Vec<&Style> = set.evidence_of(product).filter(|l| l.ev.viewport == DEFAULT_VIEWPORT).map(|l| &l.ev.style).collect();
            let s = union_styles(&all).ok_or_else(|| AppError::NotFound(format!("`{product}` has no system.json and no {DEFAULT_VIEWPORT} evidence to derive one from")))?;
            lims.push(Limitation::new("system_derived", format!("no {product}/system.json: derived as the union of {} {DEFAULT_VIEWPORT} captures", all.len())));
            ("derived", s, all.len())
        }
    };
    Ok(set.finish(json!({ "product": product, "source": source, "capturesUnioned": unioned, "summary": style_summary(&style), "targets": system_targets(&style), "style": style }), &lims))
}

pub fn flow(root: &Path, max_nodes: usize, product: &str, task: &str) -> Result<Value> {
    need_token("product", product)?;
    need_token("task", task)?;
    let set = open(root, max_nodes, Some(&[product]), Want { flows: true, ..Default::default() })?;
    need_product_pub(&set, product)?;
    let (_, f) = set.flows.iter().find(|(_, f)| f.task == task).ok_or_else(|| {
        AppError::NotFound(format!("`{product}` has no flow `{task}` (flows: {})", set.flows.iter().map(|(_, f)| f.task.clone()).collect::<Vec<_>>().join(", ")))
    })?;
    let steps: Vec<Value> = f
        .steps
        .iter()
        .map(|s| json!({ "n": s.n, "action": s.action, "target": s.target, "screen": s.screen, "state": s.state, "image": s.image.as_ref().map(|i| set.blob_path(&format!("blobs/{}.png", i.sha256))) }))
        .collect();
    let v = json!({ "product": product, "task": task, "stepCount": f.step_count, "clicks": f.steps.iter().filter(|s| s.action == "click").count(), "steps": steps, "deadEnds": f.dead_ends });
    Ok(set.finish(v, &[]))
}

fn select_nav<'a>(set: &'a UiSet, product: &str, viewport: &str, seat: Option<&str>) -> Result<&'a UiNav> {
    need_product_pub(set, product)?;
    let all: Vec<&UiNav> = set.navs.iter().filter(|(_, n)| n.product == product).map(|(_, n)| n).collect();
    let mut c: Vec<&UiNav> = all.iter().copied().filter(|n| n.viewport == viewport && seat.is_none_or(|s| n.seat == s)).collect();
    c.sort_by_key(|n| (seat_rank(&n.seat), n.seat.clone()));
    c.first().copied().ok_or_else(|| {
        AppError::NotFound(format!("`{product}` has no nav for viewport {viewport} seat {} (navs: {})", seat.unwrap_or("any"), all.iter().map(|n| n.stem()).collect::<Vec<_>>().join(", ")))
    })
}
const NAVS: Want = Want { evidence: false, flows: false, navs: true, systems: false, graphs: false };

pub fn nav(root: &Path, max_nodes: usize, product: &str, viewport: Option<&str>, seat: Option<&str>) -> Result<Value> {
    need_token("product", product)?;
    let set = open(root, max_nodes, Some(&[product]), NAVS)?;
    let n = select_nav(&set, product, viewport.unwrap_or(DEFAULT_VIEWPORT), seat)?;
    let tree: Vec<Value> = n
        .ordered()
        .iter()
        .map(|x| json!({ "order": x.order, "level": x.level, "kind": x.kind, "label": x.label, "target": x.target, "hiddenBehind": x.hidden_behind, "icon": x.icon, "badge": x.badge }))
        .collect();
    let v = json!({ "product": product, "version": n.version, "viewport": n.viewport, "seat": n.seat, "stats": nav_stats(n), "tree": tree });
    Ok(set.finish(v, &[]))
}

pub fn nav_diff_verb(root: &Path, max_nodes: usize, ours: &str, refp: &str, viewport: Option<&str>, seat: Option<&str>, ref_seat: Option<&str>) -> Result<Value> {
    need_token("--ours", ours)?;
    need_token("--ref", refp)?;
    let set = open(root, max_nodes, Some(&[ours, refp]), NAVS)?;
    let vp = viewport.unwrap_or(DEFAULT_VIEWPORT);
    let (a, b) = (select_nav(&set, ours, vp, seat)?, select_nav(&set, refp, vp, ref_seat)?);
    let (sa, sb) = (nav_stats(a), nav_stats(b));
    let v = json!({
        "viewport": vp,
        "ours": { "product": ours, "version": a.version, "seat": a.seat, "stats": sa },
        "ref": { "product": refp, "version": b.version, "seat": b.seat, "stats": sb },
        "diff": nav_diff(&sa, &sb),
    });
    Ok(set.finish(v, &[]))
}

pub fn journey(root: &Path, journey: &str, products: &[String]) -> Result<Value> {
    need_token("journey", journey)?;
    for p in products {
        need_token("product", p)?;
    }
    let dir = if root.join("ui").is_dir() { root.join("ui") } else { root.to_path_buf() };
    let path = dir.join("journeys").join(format!("{journey}.json"));
    let j: UiJourney = read_json(&path).map_err(|e| AppError::NotFound(format!("journey `{journey}`: {e}")))?;
    let p = j.problems();
    if !p.is_empty() {
        return Err(bad(format!("journeys/{journey}.json: {}", p.join("; "))));
    }
    let mut rows = vec![];
    for (prod, r) in &j.results {
        if !products.is_empty() && !products.contains(prod) {
            continue;
        }
        rows.push(json!({ "product": prod, "outcome": journey_line(r), "result": r }));
    }
    let missing: Vec<&String> = products.iter().filter(|p| !j.results.contains_key(*p)).collect();
    let lims: Vec<Limitation> = missing.iter().map(|p| Limitation::new("no_result", format!("journey `{journey}` has no result for `{p}`"))).collect();
    // pairwise words against the first product with a completed run, in product order
    let base = j.results.iter().find(|(p, r)| r.status == "completed" && (products.is_empty() || products.contains(p)));
    let words: Vec<Value> = match base {
        Some((bp, br)) => j
            .results
            .iter()
            .filter(|(p, r)| p != &bp && r.status == "completed" && (products.is_empty() || products.contains(p)))
            .map(|(p, r)| json!({ "product": p, "against": bp, "diff": journey_diff(r, br) }))
            .collect(),
        None => vec![],
    };
    let total = rows.len();
    let v = json!({ "journey": journey, "title": j.title, "goalArchetype": j.goal_archetype, "items": rows, "pairwise": words, "page": { "next": null, "total": total }, "limitation_items": lims });
    Ok(v)
}

pub fn coverage(root: &Path, run: &str) -> Result<Value> {
    need_token("run id", run)?;
    let dir = if root.join("ui").is_dir() { root.join("ui") } else { root.to_path_buf() };
    let c: UiCoverage = read_json(&dir.join("coverage").join(format!("{run}.json"))).map_err(|e| AppError::NotFound(format!("coverage of run `{run}`: {e}")))?;
    let p = c.problems(run);
    if !p.is_empty() {
        return Err(bad(format!("coverage/{run}.json: {}", p.join("; "))));
    }
    let (table, blanks) = coverage_table(&c);
    let rows: Vec<Value> = table
        .iter()
        .map(|(prod, t)| {
            let g = |k: &str| *t.get(k).unwrap_or(&0);
            let total: usize = t.values().sum();
            json!({ "product": prod, "captured": g("captured"), "n/a": g("n/a"), "failed": g("failed"), "blank": g("blank"), "cells": total, "capturedShare": share(g("captured"), total) })
        })
        .collect();
    let complete = blanks.is_empty();
    let mut lims = vec![];
    if !complete {
        lims.push(Limitation::new("incomplete_run", format!("{} blank cells (a missing cell must be explained): {}", blanks.len(), blanks.iter().take(10).cloned().collect::<Vec<_>>().join(", "))));
    }
    Ok(json!({ "runId": run, "complete": complete, "table": rows, "blankCells": blanks, "limitation_items": lims }))
}

type Failure = (AppError, Option<Value>);

/// A run manifest read from `<dir>/runs/<id>.json`.
fn load_run(dir: &Path, id: &str) -> Result<UiRun> {
    let r: UiRun = read_json(&dir.join("runs").join(format!("{id}.json"))).map_err(|e| AppError::NotFound(format!("run `{id}`: {e}")))?;
    let p = r.problems(id);
    if p.is_empty() { Ok(r) } else { Err(bad(format!("runs/{id}.json: {}", p.join("; ")))) }
}
fn ui_dir(root: &Path) -> PathBuf {
    if root.join("ui").is_dir() { root.join("ui") } else { root.to_path_buf() }
}
/// `product@version/rest` -> `product/rest`: two roots may hold different versions of the same product, and that is what they are compared for.
fn key_without_version(k: &str) -> String {
    match (k.find('@'), k.find('/')) {
        (Some(at), Some(sl)) if at < sl => format!("{}{}", &k[..at], &k[sl..]),
        _ => k.to_string(),
    }
}
/// The one run a root holds (a root given to `--root-a/--root-b` must hold exactly one).
fn only_run(root: &Path, which: &str) -> Result<(PathBuf, String)> {
    let dir = ui_dir(root);
    if !dir.is_dir() {
        return Err(AppError::NotFound(format!("{which}: no ui directory at {}", root.display())));
    }
    let mut ids = vec![];
    if let Ok(rd) = std::fs::read_dir(dir.join("runs")) {
        for e in rd.flatten() {
            if let Some(stem) = e.file_name().to_str().and_then(|n| n.strip_suffix(".json")) {
                if e.file_type().is_ok_and(|t| t.is_file()) {
                    ids.push(stem.to_string());
                }
            }
        }
    }
    ids.sort();
    match ids.len() {
        1 => Ok((dir, ids.remove(0))),
        0 => Err(AppError::NotFound(format!("{which}: {} holds no run (runs/<runId>.json)", root.display()))),
        n => Err(bad(format!("{which}: {} holds {n} runs; give each run its own ui root (probe: --out <dir>/<runLabel>) so a verify cannot compare a run with itself", root.display()))),
    }
}

/// The determinism gate across two ui roots, each holding ONE run. Captures are matched on the key without its version.
pub fn verify_roots(root_a: &Path, root_b: &Path, max_nodes: usize) -> std::result::Result<Value, Failure> {
    let f = |e: AppError| (e, None);
    check_max(max_nodes).map_err(f)?;
    if let (Ok(a), Ok(b)) = (std::fs::canonicalize(root_a), std::fs::canonicalize(root_b)) {
        if a == b {
            return Err(f(bad("--root-a and --root-b are the same folder: that compares a run with itself".into())));
        }
    }
    let ((da, ia), (db, ib)) = (only_run(root_a, "--root-a").map_err(f)?, only_run(root_b, "--root-b").map_err(f)?);
    let (ra, rb) = (load_run(&da, &ia).map_err(f)?, load_run(&db, &ib).map_err(f)?);
    compare(&da, &ra, &db, &rb, (&ia, &ib), max_nodes, true)
}

/// The determinism gate between two runs of ONE root. Only correct when the two runs wrote different evidence files; a shared file is refused.
pub fn verify(root: &Path, max_nodes: usize, run_a: &str, run_b: &str) -> std::result::Result<Value, Failure> {
    let f = |e: AppError| (e, None);
    check_max(max_nodes).map_err(f)?;
    need_token("run id", run_a).map_err(f)?;
    need_token("run id", run_b).map_err(f)?;
    let dir = ui_dir(root);
    let (ra, rb) = (load_run(&dir, run_a).map_err(f)?, load_run(&dir, run_b).map_err(f)?);
    let same: Vec<String> = ra.captures.iter().filter(|(k, file)| rb.captures.get(*k) == Some(*file)).map(|(k, file)| format!("{k} -> {file}")).collect();
    if !same.is_empty() {
        return Err(f(bad(format!(
            "runs {run_a} and {run_b} point at the same evidence file for {} key(s) (e.g. {}): one root keeps one file per capture, so this would compare the file with itself. Write each run into its own root and use `ui verify --root-a <dirA> --root-b <dirB>`",
            same.len(),
            same[0]
        ))));
    }
    compare(&dir, &ra, &dir, &rb, (run_a, run_b), max_nodes, false)
}

/// domHash equal, metrics equal, image dHash within 4 bits and `net.failed` equal for every shared capture key. Timings and byte counts are never compared.
#[allow(clippy::too_many_arguments)]
fn compare(dir_a: &Path, ra: &UiRun, dir_b: &Path, rb: &UiRun, ids: (&str, &str), max_nodes: usize, ignore_version: bool) -> std::result::Result<Value, Failure> {
    let (run_a, run_b) = ids;
    let load_ev = |dir: &Path, rel: &str| -> std::result::Result<UiEvidence, String> {
        let e: UiEvidence = read_json(&dir.join(rel))?;
        let p = e.problems();
        if p.is_empty() { Ok(e) } else { Err(p.join("; ")) }
    };
    let norm = |k: &String| if ignore_version { key_without_version(k) } else { k.clone() };
    let map_a: BTreeMap<String, &String> = ra.captures.iter().map(|(k, f)| (norm(k), f)).collect();
    let map_b: BTreeMap<String, &String> = rb.captures.iter().map(|(k, f)| (norm(k), f)).collect();
    let keys_a: BTreeSet<&String> = map_a.keys().collect();
    let keys_b: BTreeSet<&String> = map_b.keys().collect();
    let both: Vec<&String> = keys_a.intersection(&keys_b).copied().take(max_nodes).collect();
    let truncated = keys_a.intersection(&keys_b).count() > both.len();
    let mut diffs: Vec<Value> = vec![];
    let mut net_missing = 0usize;
    for k in &both {
        let (ea, eb) = (load_ev(dir_a, map_a[*k]), load_ev(dir_b, map_b[*k]));
        let (ea, eb) = match (ea, eb) {
            (Ok(a), Ok(b)) => (a, b),
            (a, b) => {
                diffs.push(json!({ "key": k, "kind": "unreadable", "a": a.err(), "b": b.err() }));
                continue;
            }
        };
        if ea.dom_hash != eb.dom_hash {
            diffs.push(json!({ "key": k, "kind": "domHash", "a": ea.dom_hash, "b": eb.dom_hash }));
        }
        let ids: BTreeSet<&String> = ea.metrics.keys().chain(eb.metrics.keys()).collect();
        let mut ids: Vec<&String> = ids.into_iter().collect();
        ids.sort_by_key(|i| (metric_no(i).unwrap_or(99), (*i).clone()));
        for id in ids {
            if ea.metrics.get(id) != eb.metrics.get(id) {
                diffs.push(json!({ "key": k, "kind": "metric", "metric": id, "a": ea.metrics.get(id), "b": eb.metrics.get(id) }));
            }
        }
        match (&ea.image.dhash, &eb.image.dhash) {
            (Some(x), Some(y)) => {
                let d = hamming(x, y).unwrap_or(64);
                if d > DHASH_MAX_DISTANCE {
                    diffs.push(json!({ "key": k, "kind": "image", "distance": d, "limit": DHASH_MAX_DISTANCE, "a": x, "b": y }));
                }
            }
            (x, y) => diffs.push(json!({ "key": k, "kind": "image", "detail": "image.dhash missing", "a": x, "b": y })),
        }
        match (&ea.net, &eb.net) {
            (Some(x), Some(y)) => {
                let (mut fa, mut fb) = (x.failed.clone(), y.failed.clone());
                fa.sort();
                fb.sort();
                if fa != fb {
                    diffs.push(json!({ "key": k, "kind": "netFailed", "a": fa, "b": fb }));
                }
            }
            (None, None) => {}
            _ => net_missing += 1,
        }
    }
    let only_a: Vec<&String> = keys_a.difference(&keys_b).copied().collect();
    let only_b: Vec<&String> = keys_b.difference(&keys_a).copied().collect();
    let mut lims = vec![];
    if truncated {
        lims.push(Limitation::new("truncated", format!("compared the first {} shared keys (--max-nodes)", both.len())));
    }
    if net_missing > 0 {
        lims.push(Limitation::new("net_missing", format!("{net_missing} shared captures have `net` on one side only: their failed calls were not compared")));
    }
    if ignore_version {
        lims.push(Limitation::new("versions_ignored", "captures are matched on product/screen/state/viewport/seat/locale/theme; the version in the key may differ between the roots"));
    }
    lims.push(Limitation::new("snapshot_recompute_pending", SNAPSHOT_NOTE));
    let data = json!({
        "runA": run_a, "runB": run_b, "compared": both.len(), "identical": diffs.is_empty(), "differences": diffs,
        "onlyInA": only_a, "onlyInB": only_b,
        "limitation_items": lims,
    });
    if both.is_empty() && ignore_version {
        return Err((AppError::CheckFailed(format!("roots share no capture key: {} only in A, {} only in B (nothing was compared)", only_a.len(), only_b.len())), Some(data)));
    }
    if data["identical"] == Value::Bool(true) {
        Ok(data)
    } else {
        let n = data["differences"].as_array().map_or(0, |a| a.len());
        Err((AppError::CheckFailed(format!("runs {run_a} and {run_b} differ: {n} differences over {} shared keys", both.len())), Some(data)))
    }
}
