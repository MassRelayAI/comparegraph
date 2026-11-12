//! UI evidence (ui-evidence/1, ui-flow/1, ui-run/1, ui-registry/1, the map row and the `style` block): types, validation and
//! comparison words. Pure - no I/O. Shared format: CMP-UI-1-lanes.md incl. its DETERMINISM PROTOCOL. Navigation, journeys and
//! coverage (section 8) are in `ui_nav`.
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use std::collections::BTreeMap;

pub const EVIDENCE_SCHEMA: &str = "ui-evidence/1";
pub const FLOW_SCHEMA: &str = "ui-flow/1";
pub const RUN_SCHEMA: &str = "ui-run/1";
pub const REGISTRY_SCHEMA: &str = "ui-registry/1";
pub const SEATS: &[&str] = &["guest", "member", "admin", "admin-dev", "operator", "client", "ref"];
/// The locale and theme a capture has unless its file name says otherwise (section 10 path rule).
pub const DEFAULT_LOCALE: &str = "en-US";
pub const DEFAULT_THEME: &str = "light";
pub const STATES: &[&str] = &["empty", "populated", "error", "loading-skeleton", "long"];
pub const REGION_KINDS: &[&str] = &["rail", "sidebar", "list", "main", "panel", "topbar", "bottombar", "composer"];
pub const ARCHETYPES: &[&str] = &[
    "conversation", "list", "detail", "inbox", "dashboard", "settings", "create-flow", "onboarding", "live-session", "public-page",
];
/// Directory names under a product that are not screens.
pub const RESERVED_DIRS: &[&str] = &["flows", "nav"];
pub const METRIC_COUNT: usize = 12;
pub const DEFAULT_STATE: &str = "populated";
pub const DEFAULT_VIEWPORT: &str = "1440x900";
/// Two captures are the same picture when their difference hashes are at most this far apart.
pub const DHASH_MAX_DISTANCE: u32 = 4;

/// A path-safe token (product, screen, task, family, row ids): never a separator, never a leading dot.
pub fn token_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && !s.starts_with('.') && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}
pub fn sha_ok(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn dhash_ok(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn viewport_ok(s: &str) -> bool {
    s.split_once('x').is_some_and(|(w, h)| {
        let n = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) && !p.starts_with('0');
        n(w) && n(h)
    })
}
/// `E1`..`E12` -> 1..=12.
pub fn metric_no(id: &str) -> Option<usize> {
    let n: usize = id.strip_prefix('E')?.parse().ok()?;
    ((1..=METRIC_COUNT).contains(&n) && id == format!("E{n}")).then_some(n)
}
pub fn hamming(a: &str, b: &str) -> Option<u32> {
    Some((u64::from_str_radix(a, 16).ok()? ^ u64::from_str_radix(b, 16).ok()?).count_ones())
}
pub fn safe_rel(f: &str) -> bool {
    !f.starts_with('/') && !f.contains('\\') && !f.split('/').any(|c| c == ".." || c.is_empty())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageRef {
    pub sha256: String,
    pub file: String,
    /// 64-bit difference hash, 16 hex digits
    pub dhash: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub value: Value,
    #[serde(default)]
    pub limit: Value,
    pub pass: bool,
    #[serde(default)]
    pub detail: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub kind: String,
    pub x: Number,
    pub y: Number,
    pub w: Number,
    pub h: Number,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Primary {
    pub name: String,
    pub x: Number,
    pub y: Number,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContentStart {
    pub y: Number,
    pub how: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub regions: Vec<Region>,
    pub primary: Option<Primary>,
    pub content_start: Option<ContentStart>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeStyle {
    pub sizes: BTreeMap<String, u64>,
    pub distinct_sizes: u64,
    pub weights: BTreeMap<String, u64>,
    pub families: Vec<String>,
    pub line_heights: BTreeMap<String, u64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LowContrast {
    pub text: String,
    pub ratio: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorStyle {
    pub text: BTreeMap<String, u64>,
    pub background: BTreeMap<String, u64>,
    pub border: BTreeMap<String, u64>,
    pub distinct_colors: u64,
    pub accent: Option<String>,
    #[serde(default)]
    pub low_contrast: Vec<LowContrast>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceStyle {
    pub values: BTreeMap<String, u64>,
    pub distinct_values: u64,
    pub on_scale4: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeStyle {
    pub radius: BTreeMap<String, u64>,
    pub shadows: u64,
    pub borders: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DensityStyle {
    pub text_per_kpx: f64,
    pub elements_per_kpx: f64,
    pub whitespace_ratio: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MotionStyle {
    pub transitions: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Style {
    #[serde(rename = "type")]
    pub type_: TypeStyle,
    pub color: ColorStyle,
    pub space: SpaceStyle,
    pub shape: ShapeStyle,
    pub density: DensityStyle,
    pub motion: MotionStyle,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiEvidence {
    pub schema: String,
    pub product: String,
    /// container digest (reference apps) or git sha (ours); never "latest"
    pub version: String,
    pub screen: String,
    pub state: String,
    pub route: String,
    pub viewport: String,
    pub seat: String,
    pub locale: String,
    pub theme: String,
    pub dom_hash: String,
    /// section 10 top-level fields: absent in older files, validated when present
    #[serde(default)]
    pub archetype: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub fixture: Value,
    #[serde(default)]
    pub masks: Vec<String>,
    #[serde(default)]
    pub unstable: bool,
    pub image: ImageRef,
    pub metrics: BTreeMap<String, Metric>,
    pub layout: Layout,
    #[serde(default)]
    pub flow: Value,
    #[serde(default)]
    pub probe: Value,
    #[serde(default)]
    pub meta: Value,
    pub style: Style,
}

impl UiEvidence {
    /// The capture key: product@version/screen/state/viewport/seat/locale/theme.
    pub fn capture_key(&self) -> String {
        format!("{}@{}/{}/{}/{}/{}/{}/{}", self.product, self.version, self.screen, self.state, self.viewport, self.seat, self.locale, self.theme)
    }
    /// The file stem under `ui/<product>/<screen>/<state>/`.
    pub fn stem(&self) -> String {
        if self.locale == DEFAULT_LOCALE && self.theme == DEFAULT_THEME {
            format!("{}-{}", self.viewport, self.seat)
        } else {
            format!("{}-{}-{}-{}", self.viewport, self.seat, self.locale, self.theme)
        }
    }
    pub fn pass_count(&self) -> usize {
        self.metrics.values().filter(|m| m.pass).count()
    }
    /// Every problem found, in a fixed order; empty means valid.
    pub fn problems(&self) -> Vec<String> {
        let mut p = vec![];
        if self.schema != EVIDENCE_SCHEMA {
            p.push(format!("schema `{}` is not `{EVIDENCE_SCHEMA}`", self.schema));
        }
        if !token_ok(&self.product) {
            p.push(format!("product `{}` is not a path-safe token", self.product));
        }
        if self.version.is_empty() || self.version == "latest" {
            p.push(format!("version `{}` must be a digest or sha, never empty or `latest`", self.version));
        }
        if !token_ok(&self.screen) || RESERVED_DIRS.contains(&self.screen.as_str()) {
            p.push(format!("screen `{}` is not a valid screen id", self.screen));
        }
        if !STATES.contains(&self.state.as_str()) {
            p.push(format!("state `{}` must be one of {}", self.state, STATES.join("|")));
        }
        if !self.route.starts_with('/') {
            p.push(format!("route `{}` must start with `/`", self.route));
        }
        if !viewport_ok(&self.viewport) {
            p.push(format!("viewport `{}` must be <width>x<height>", self.viewport));
        }
        if !SEATS.contains(&self.seat.as_str()) {
            p.push(format!("seat `{}` must be one of {}", self.seat, SEATS.join("|")));
        }
        if self.locale.is_empty() || self.theme.is_empty() {
            p.push("locale and theme must be set".into());
        }
        if let Some(a) = &self.archetype {
            if !ARCHETYPES.contains(&a.as_str()) {
                p.push(format!("archetype `{a}` must be one of {}", ARCHETYPES.join("|")));
            }
        }
        if let Some(k) = &self.key {
            if *k != self.capture_key() {
                p.push(format!("key `{k}` does not match the fields it joins (`{}`)", self.capture_key()));
            }
        }
        if !sha_ok(&self.dom_hash) {
            p.push("domHash must be 64 lowercase hex digits".into());
        }
        if !sha_ok(&self.image.sha256) {
            p.push("image.sha256 must be 64 lowercase hex digits".into());
        } else if self.image.file != format!("blobs/{}.png", self.image.sha256) {
            p.push(format!("image.file `{}` must be blobs/<sha256>.png", self.image.file));
        }
        if let Some(d) = &self.image.dhash {
            if !dhash_ok(d) {
                p.push("image.dhash must be 16 lowercase hex digits".into());
            }
        }
        if self.metrics.is_empty() {
            p.push("metrics is empty".into());
        }
        for id in self.metrics.keys() {
            if metric_no(id).is_none() {
                p.push(format!("metric id `{id}` is not E1..E{METRIC_COUNT}"));
            }
        }
        for r in &self.layout.regions {
            if !REGION_KINDS.contains(&r.kind.as_str()) {
                p.push(format!("region kind `{}` must be one of {}", r.kind, REGION_KINDS.join("|")));
            }
        }
        let unit = |v: f64| (0.0..=1.0).contains(&v);
        if !unit(self.style.space.on_scale4) || !unit(self.style.density.whitespace_ratio) {
            p.push("style.space.onScale4 and style.density.whitespaceRatio must be within 0..1".into());
        }
        p
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlowImage {
    pub sha256: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlowStep {
    pub n: u64,
    pub action: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub screen: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub image: Option<FlowImage>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiFlow {
    pub schema: String,
    pub product: String,
    pub task: String,
    pub steps: Vec<FlowStep>,
    pub step_count: u64,
    #[serde(default)]
    pub dead_ends: Vec<Value>,
}
impl UiFlow {
    pub fn problems(&self) -> Vec<String> {
        let mut p = vec![];
        if self.schema != FLOW_SCHEMA {
            p.push(format!("schema `{}` is not `{FLOW_SCHEMA}`", self.schema));
        }
        if !token_ok(&self.product) || !token_ok(&self.task) {
            p.push("product and task must be path-safe tokens".into());
        }
        if self.step_count as usize != self.steps.len() {
            p.push(format!("stepCount {} does not match {} steps", self.step_count, self.steps.len()));
        }
        for (i, s) in self.steps.iter().enumerate() {
            if s.n as usize != i + 1 {
                p.push(format!("step {} is numbered {}", i + 1, s.n));
            }
            if s.image.as_ref().is_some_and(|im| !sha_ok(&im.sha256)) {
                p.push(format!("step {} image.sha256 is not 64 hex digits", i + 1));
            }
        }
        p
    }
}

/// `ui/runs/<runId>.json`: the inputs of a run and every capture key -> evidence file (relative to `ui/`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiRun {
    pub schema: String,
    pub run_id: String,
    pub captures: BTreeMap<String, String>,
}
impl UiRun {
    pub fn problems(&self, stem: &str) -> Vec<String> {
        let mut p = vec![];
        if self.schema != RUN_SCHEMA {
            p.push(format!("schema `{}` is not `{RUN_SCHEMA}`", self.schema));
        }
        if self.run_id != stem {
            p.push(format!("runId `{}` does not match the file name `{stem}`", self.run_id));
        }
        for (k, f) in &self.captures {
            if !safe_rel(f) || !f.ends_with(".json") {
                p.push(format!("capture `{k}` names an unsafe file `{f}`"));
            }
        }
        p
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegistryScreen {
    pub id: String,
    pub archetype: String,
    #[serde(default)]
    pub states: Vec<String>,
}
/// `ui/registry.json`: screens x archetype x states (recipes and masks stay in the file) and the per-archetype limits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiRegistry {
    pub schema: String,
    pub version: String,
    pub screens: Vec<RegistryScreen>,
    #[serde(default)]
    pub archetypes: BTreeMap<String, BTreeMap<String, Value>>,
}
impl UiRegistry {
    pub fn problems(&self) -> Vec<String> {
        let mut p = vec![];
        if self.schema != REGISTRY_SCHEMA {
            p.push(format!("schema `{}` is not `{REGISTRY_SCHEMA}`", self.schema));
        }
        for s in &self.screens {
            if !ARCHETYPES.contains(&s.archetype.as_str()) {
                p.push(format!("screen `{}` has unknown archetype `{}`", s.id, s.archetype));
            }
            if let Some(b) = s.states.iter().find(|x| !STATES.contains(&x.as_str())) {
                p.push(format!("screen `{}` has unknown state `{b}`", s.id));
            }
        }
        for (a, lim) in &self.archetypes {
            if !ARCHETYPES.contains(&a.as_str()) {
                p.push(format!("limits for unknown archetype `{a}`"));
            }
            if let Some(b) = lim.keys().find(|k| metric_no(k).is_none()) {
                p.push(format!("archetype `{a}` limits metric `{b}`, not E1..E{METRIC_COUNT}"));
            }
        }
        p
    }
    pub fn archetype_of(&self, screen: &str) -> Option<&str> {
        self.screens.iter().find(|s| s.id == screen).map(|s| s.archetype.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchetypeEntry {
    #[serde(default)]
    pub limits: BTreeMap<String, Value>,
}
/// `ui/archetypes.json` (ui-archetypes/1, section 11): per-archetype limits; judge questions, style targets, overrides and rules are kept in the file, not read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiArchetypes {
    pub schema: String,
    pub archetypes: BTreeMap<String, ArchetypeEntry>,
}
impl UiArchetypes {
    pub fn problems(&self) -> Vec<String> {
        let mut p = vec![];
        if self.schema != "ui-archetypes/1" {
            p.push(format!("schema `{}` is not `ui-archetypes/1`", self.schema));
        }
        for (a, e) in &self.archetypes {
            if !ARCHETYPES.contains(&a.as_str()) {
                p.push(format!("limits for unknown archetype `{a}`"));
            }
            if let Some(b) = e.limits.keys().find(|k| metric_no(k).is_none()) {
                p.push(format!("archetype `{a}` limits metric `{b}`, not E1..E{METRIC_COUNT}"));
            }
        }
        p
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MapRow {
    pub family: String,
    pub row_id: String,
    pub product: String,
    pub screen: String,
    pub route: String,
    pub task: String,
    pub note: String,
}
const MAP_HEADER: &str = "family,row_id,product,screen,route,task";
/// section 11: the design lane adds a seventh column
const MAP_HEADER_NOTE: &str = "family,row_id,product,screen,route,task,note";

fn csv_fields(line: &str) -> Result<Vec<String>, String> {
    let (mut out, mut cur, mut quoted) = (vec![], String::new(), false);
    let mut it = line.chars().peekable();
    while let Some(c) = it.next() {
        match (c, quoted) {
            ('"', true) if it.peek() == Some(&'"') => {
                cur.push('"');
                it.next();
            }
            ('"', true) => quoted = false,
            ('"', false) if cur.is_empty() => quoted = true,
            (',', false) => out.push(std::mem::take(&mut cur)),
            (c, _) => cur.push(c),
        }
    }
    if quoted {
        return Err("unterminated quote".into());
    }
    out.push(cur);
    Ok(out)
}
/// Parse `map.csv` (RFC 4180, header `family,row_id,product,screen,route,task`); rows come back sorted.
pub fn parse_map(text: &str) -> Result<Vec<MapRow>, String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let cols = match lines.next().map(str::trim) {
        Some(MAP_HEADER) => 6,
        Some(MAP_HEADER_NOTE) => 7,
        _ => return Err(format!("map.csv header must be `{MAP_HEADER}` or `{MAP_HEADER_NOTE}`")),
    };
    let mut rows = vec![];
    for (i, l) in lines.enumerate() {
        let f = csv_fields(l).map_err(|e| format!("map.csv row {}: {e}", i + 1))?;
        if f.len() != cols {
            return Err(format!("map.csv row {}: expected {cols} fields, found {}", i + 1, f.len()));
        }
        if !token_ok(&f[0]) || !token_ok(&f[1]) || !token_ok(&f[2]) || !token_ok(&f[3]) {
            return Err(format!("map.csv row {}: family, row_id, product and screen must be path-safe tokens", i + 1));
        }
        rows.push(MapRow { family: f[0].clone(), row_id: f[1].clone(), product: f[2].clone(), screen: f[3].clone(), route: f[4].clone(), task: f[5].clone(), note: f.get(6).cloned().unwrap_or_default() });
    }
    rows.sort_by(|a, b| (&a.family, &a.row_id, &a.product, &a.screen).cmp(&(&b.family, &b.row_id, &b.product, &b.screen)));
    Ok(rows)
}

/// A number the way a person says it: integers plain, others to 2 decimals without trailing zeros.
pub fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 {
        return format!("{}", v as i64);
    }
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}
fn num(n: &Number) -> f64 {
    n.as_f64().unwrap_or(0.0)
}
/// `label a vs b`, only when they differ.
pub fn vs(out: &mut Vec<String>, label: &str, x: f64, y: f64) {
    if x != y {
        out.push(format!("{label} {} vs {}", fmt_num(x), fmt_num(y)));
    }
}

pub fn nav_columns(l: &Layout) -> usize {
    l.regions.iter().filter(|r| matches!(r.kind.as_str(), "rail" | "sidebar" | "list")).count()
}
fn kind_counts(l: &Layout) -> BTreeMap<&str, usize> {
    let mut m = BTreeMap::new();
    for r in &l.regions {
        *m.entry(r.kind.as_str()).or_insert(0) += 1;
    }
    m
}
/// One-line summary of a layout.
pub fn layout_summary(l: &Layout) -> String {
    let kinds: Vec<String> = kind_counts(l).iter().map(|(k, n)| format!("{k} {n}")).collect();
    let primary = match &l.primary {
        Some(p) => format!("primary \"{}\" at {},{}", p.name, fmt_num(num(&p.x)), fmt_num(num(&p.y))),
        None => "no primary".to_string(),
    };
    let start = match &l.content_start {
        Some(c) => format!("content starts y {} ({})", fmt_num(num(&c.y)), c.how),
        None => "no content start".to_string(),
    };
    format!("regions: {}; {primary}; {start}", if kinds.is_empty() { "none".into() } else { kinds.join(", ") })
}
/// The layout difference in words, `a vs b`, fixed order; empty when the layouts match on these terms.
pub fn layout_diff(a: &Layout, b: &Layout) -> Vec<String> {
    let mut out = vec![];
    vs(&mut out, "nav columns", nav_columns(a) as f64, nav_columns(b) as f64);
    vs(&mut out, "primaries", a.primary.is_some() as u8 as f64, b.primary.is_some() as u8 as f64);
    let (ka, kb) = (kind_counts(a), kind_counts(b));
    let mut kinds: Vec<&str> = ka.keys().chain(kb.keys()).copied().collect();
    kinds.sort();
    kinds.dedup();
    for k in kinds {
        if matches!(k, "rail" | "sidebar" | "list") {
            continue; // already said as nav columns
        }
        vs(&mut out, &format!("{k} regions"), *ka.get(k).unwrap_or(&0) as f64, *kb.get(k).unwrap_or(&0) as f64);
    }
    let cs = |l: &Layout| l.content_start.as_ref().map(|c| num(&c.y));
    if let (Some(x), Some(y)) = (cs(a), cs(b)) {
        vs(&mut out, "content start y", x, y);
    }
    out
}
/// The style difference in words, fixed order, only where the two differ.
pub fn style_diff(a: &Style, b: &Style) -> Vec<String> {
    let mut out = vec![];
    let o = &mut out;
    vs(o, "type sizes", a.type_.distinct_sizes as f64, b.type_.distinct_sizes as f64);
    vs(o, "font families", a.type_.families.len() as f64, b.type_.families.len() as f64);
    vs(o, "weights", a.type_.weights.len() as f64, b.type_.weights.len() as f64);
    vs(o, "colours", a.color.distinct_colors as f64, b.color.distinct_colors as f64);
    vs(o, "low-contrast text", a.color.low_contrast.len() as f64, b.color.low_contrast.len() as f64);
    vs(o, "space values", a.space.distinct_values as f64, b.space.distinct_values as f64);
    vs(o, "onScale4", a.space.on_scale4, b.space.on_scale4);
    vs(o, "radii", a.shape.radius.len() as f64, b.shape.radius.len() as f64);
    vs(o, "shadows", a.shape.shadows as f64, b.shape.shadows as f64);
    vs(o, "borders", a.shape.borders as f64, b.shape.borders as f64);
    vs(o, "elements per kpx", a.density.elements_per_kpx, b.density.elements_per_kpx);
    vs(o, "whitespace", a.density.whitespace_ratio, b.density.whitespace_ratio);
    vs(o, "transitions", a.motion.transitions as f64, b.motion.transitions as f64);
    out
}
/// One-line style summary.
pub fn style_summary(s: &Style) -> String {
    format!(
        "type sizes {}, weights {}, colours {}, low-contrast {}, space values {}, onScale4 {}, radii {}, shadows {}, density {} elements/kpx, whitespace {}",
        s.type_.distinct_sizes, s.type_.weights.len(), s.color.distinct_colors, s.color.low_contrast.len(), s.space.distinct_values,
        fmt_num(s.space.on_scale4), s.shape.radius.len(), s.shape.shadows, fmt_num(s.density.elements_per_kpx), fmt_num(s.density.whitespace_ratio)
    )
}

fn sum_into(into: &mut BTreeMap<String, u64>, from: &BTreeMap<String, u64>) {
    for (k, v) in from {
        *into.entry(k.clone()).or_insert(0) += v;
    }
}
/// The design system of a product as the union of its screens' styles (counts summed, distinct counts from the union).
pub fn union_styles(all: &[&Style]) -> Option<Style> {
    let first = all.first()?;
    let mut u = (*first).clone();
    u.type_ = TypeStyle { sizes: BTreeMap::new(), distinct_sizes: 0, weights: BTreeMap::new(), families: vec![], line_heights: BTreeMap::new() };
    u.color = ColorStyle { text: BTreeMap::new(), background: BTreeMap::new(), border: BTreeMap::new(), distinct_colors: 0, accent: None, low_contrast: vec![] };
    u.space.values = BTreeMap::new();
    u.shape = ShapeStyle { radius: BTreeMap::new(), shadows: 0, borders: 0 };
    u.motion.transitions = 0;
    let mut accents: BTreeMap<String, u64> = BTreeMap::new();
    let n = all.len() as f64;
    let (mut tp, mut ep, mut wr) = (0.0, 0.0, 0.0);
    for s in all {
        sum_into(&mut u.type_.sizes, &s.type_.sizes);
        sum_into(&mut u.type_.weights, &s.type_.weights);
        sum_into(&mut u.type_.line_heights, &s.type_.line_heights);
        u.type_.families.extend(s.type_.families.iter().cloned());
        sum_into(&mut u.color.text, &s.color.text);
        sum_into(&mut u.color.background, &s.color.background);
        sum_into(&mut u.color.border, &s.color.border);
        u.color.low_contrast.extend(s.color.low_contrast.iter().cloned());
        if let Some(a) = &s.color.accent {
            *accents.entry(a.clone()).or_insert(0) += 1;
        }
        sum_into(&mut u.space.values, &s.space.values);
        sum_into(&mut u.shape.radius, &s.shape.radius);
        u.shape.shadows += s.shape.shadows;
        u.shape.borders += s.shape.borders;
        u.motion.transitions += s.motion.transitions;
        tp += s.density.text_per_kpx;
        ep += s.density.elements_per_kpx;
        wr += s.density.whitespace_ratio;
    }
    u.type_.families.sort();
    u.type_.families.dedup();
    u.type_.distinct_sizes = u.type_.sizes.len() as u64;
    let mut colours: Vec<&String> = u.color.text.keys().chain(u.color.background.keys()).chain(u.color.border.keys()).collect();
    colours.sort();
    colours.dedup();
    u.color.distinct_colors = colours.len() as u64;
    u.color.low_contrast.sort_by(|a, b| a.text.cmp(&b.text).then(a.ratio.total_cmp(&b.ratio)));
    u.color.accent = accents.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))).map(|(k, _)| k);
    u.space.distinct_values = u.space.values.len() as u64;
    let (mut total, mut on) = (0u64, 0u64);
    for (k, c) in &u.space.values {
        total += c;
        if k.trim_end_matches("px").parse::<f64>().is_ok_and(|v| v % 4.0 == 0.0) {
            on += c;
        }
    }
    u.space.on_scale4 = if total == 0 { 0.0 } else { ((on as f64 / total as f64) * 100.0).round() / 100.0 };
    let r = |v: f64| (v / n * 100.0).round() / 100.0;
    u.density = DensityStyle { text_per_kpx: r(tp), elements_per_kpx: r(ep), whitespace_ratio: r(wr) };
    Some(u)
}
/// The default design-system targets of the lanes file: <= 6 type sizes, <= 12 colours, >= 0.95 on the 4px scale.
pub fn system_targets(s: &Style) -> Vec<Value> {
    let t = |name: &str, value: f64, limit: &str, pass: bool| serde_json::json!({ "target": name, "value": fmt_num(value), "limit": limit, "pass": pass });
    vec![
        t("type sizes", s.type_.distinct_sizes as f64, "<= 6", s.type_.distinct_sizes <= 6),
        t("colours", s.color.distinct_colors as f64, "<= 12", s.color.distinct_colors <= 12),
        t("onScale4", s.space.on_scale4, ">= 0.95", s.space.on_scale4 >= 0.95),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> UiEvidence {
        let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/ui/ui/cluttered/room/populated/1440x900-member.json");
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }
    #[test]
    fn sample_is_valid() {
        assert_eq!(sample().problems(), Vec::<String>::new());
    }
    #[test]
    fn each_rule_goes_red() {
        type Break = Box<dyn Fn(&mut UiEvidence)>;
        let cases: Vec<(&str, Break)> = vec![
            ("schema", Box::new(|e| e.schema = "ui-evidence/2".into())),
            ("viewport", Box::new(|e| e.viewport = "1440".into())),
            ("seat", Box::new(|e| e.seat = "root".into())),
            ("state", Box::new(|e| e.state = "ready".into())),
            ("image.sha256", Box::new(|e| e.image.sha256 = "ABC".into())),
            ("domHash", Box::new(|e| e.dom_hash = "xyz".into())),
            ("dhash", Box::new(|e| e.image.dhash = Some("12".into()))),
            ("E13", Box::new(|e| {
                let m = e.metrics["E1"].clone();
                e.metrics.insert("E13".into(), m);
            })),
            ("latest", Box::new(|e| e.version = "latest".into())),
        ];
        for (needle, f) in cases {
            let mut e = sample();
            f(&mut e);
            assert!(e.problems().iter().any(|p| p.contains(needle)), "{needle}: {:?}", e.problems());
        }
    }
    #[test]
    fn map_parses_sorted_and_refuses_bad_rows() {
        let r = parse_map("family,row_id,product,screen,route,task\nb,x,p,s,\"/a,b\",t\na,x,p,s,/r,\n").unwrap();
        assert_eq!(r[0].family, "a");
        assert_eq!(r[1].route, "/a,b");
        assert!(parse_map("a,b\n").is_err());
        assert!(parse_map("family,row_id,product,screen,route,task\na,x,p,../s,/r,\n").is_err());
    }
    #[test]
    fn hamming_and_numbers() {
        assert_eq!(hamming("0000000000000000", "000000000000000f"), Some(4));
        assert_eq!(fmt_num(0.710), "0.71");
        assert_eq!(fmt_num(11.0), "11");
        assert_eq!(metric_no("E12"), Some(12));
        assert_eq!(metric_no("E012"), None);
    }
}
