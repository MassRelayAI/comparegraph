//! The ONE catalogue of every cap the UI verbs apply (REA lesson 10). A cap is a named constant with a unit and a statement of what is reported
//! when it is hit. `comparegraph ui limits` prints this table; a test fails when a numeric `const` is added to a UI module without being listed here.
use serde_json::{json, Value};

/// Files read per answer when `--max-nodes` is not given.
pub const DEFAULT_MAX_NODES: usize = 200;
/// The largest `--max-nodes`; there is nothing above it to raise.
pub const MAX_NODES: usize = 2000;
/// One evidence, flow, nav or graph file larger than this is not read (it is listed as `invalid_evidence`).
pub const MAX_FILE: u64 = 1 << 20;
/// Invalid files named one by one in an answer; the rest are counted.
pub const MAX_INVALID_LISTED: usize = 20;
/// Blank coverage cells named one by one in an answer.
pub const MAX_BLANK_LISTED: usize = 10;
/// Items per page of a listing when `--page-size` is not given.
pub const DEFAULT_PAGE_SIZE: usize = 50;
/// The largest `--page-size`.
pub const MAX_PAGE_SIZE: usize = 500;
/// Click path length when `--max-len` is not given.
pub const DEFAULT_MAX_LEN: usize = 8;
/// The largest `ui path --max-len`.
pub const MAX_LEN: usize = 20;
/// Viewport widths below this are a phone (the `phone` column of the archetype limits); at or above it, desktop.
pub const PHONE_MAX_WIDTH: u32 = 767;
/// Tablet limits (CMP-UI-1i, section 17) used when the archetype has no `tablet` column: at most this many navigation columns before content (E1).
pub const TABLET_MAX_NAV_COLUMNS: u64 = 1;
/// ... at most this many visible actions in main (E4).
pub const TABLET_MAX_ACTIONS: u64 = 12;
/// ... the content starts within this many percent of the viewport height (E7 <= 0.25).
pub const TABLET_CONTENT_START_PERCENT: u64 = 25;

struct Limit {
    name: &'static str,
    value: u64,
    unit: &'static str,
    control: &'static str,
    when_hit: &'static str,
}
fn table() -> Vec<Limit> {
    use crate::domain::ui::{DHASH_MAX_DISTANCE, METRIC_COUNT};
    use crate::domain::ui_graph::MAX_GRAPH_NODES;
    vec![
        Limit { name: "DEFAULT_MAX_NODES", value: DEFAULT_MAX_NODES as u64, unit: "files", control: "--max-nodes", when_hit: "limitation `truncated` (data: control, value, max); a verify that stopped early FAILS with `incomplete_comparison`" },
        Limit { name: "MAX_NODES", value: MAX_NODES as u64, unit: "files", control: "--max-nodes", when_hit: "--max-nodes above it is refused (invalid_arguments); at it, `truncated` says so and asks for a narrower question, not a raise" },
        Limit { name: "MAX_FILE", value: MAX_FILE, unit: "bytes", control: "none", when_hit: "the file is skipped and listed as `invalid_evidence` (larger than N bytes)" },
        Limit { name: "MAX_INVALID_LISTED", value: MAX_INVALID_LISTED as u64, unit: "entries", control: "none", when_hit: "the rest are counted in one `invalid_evidence` limitation (`and N more invalid files`)" },
        Limit { name: "MAX_BLANK_LISTED", value: MAX_BLANK_LISTED as u64, unit: "cells", control: "none", when_hit: "`incomplete_run` names the first ones; `blankCells` lists them all" },
        Limit { name: "DEFAULT_PAGE_SIZE", value: DEFAULT_PAGE_SIZE as u64, unit: "items", control: "--page-size", when_hit: "`page.next` is the cursor of the next page (pass it as --cursor)" },
        Limit { name: "MAX_PAGE_SIZE", value: MAX_PAGE_SIZE as u64, unit: "items", control: "--page-size", when_hit: "a larger --page-size is clamped to it" },
        Limit { name: "DEFAULT_MAX_LEN", value: DEFAULT_MAX_LEN as u64, unit: "clicks", control: "--max-len", when_hit: "no path within the length: `not_found` with the longest explored" },
        Limit { name: "MAX_LEN", value: MAX_LEN as u64, unit: "clicks", control: "--max-len", when_hit: "a larger --max-len is refused (invalid_arguments)" },
        Limit { name: "MAX_GRAPH_NODES", value: MAX_GRAPH_NODES as u64, unit: "nodes", control: "none (the probe crawl's own cap)", when_hit: "a graph with more nodes is invalid; the probe records `truncated` for depth, time and node caps and `graph_truncated` is reported" },
        Limit { name: "PHONE_MAX_WIDTH", value: PHONE_MAX_WIDTH as u64, unit: "css px", control: "none", when_hit: "not a cap: widths up to it use the `phone` column of the archetype limits" },
        Limit { name: "TABLET_MAX_NAV_COLUMNS", value: TABLET_MAX_NAV_COLUMNS, unit: "columns", control: "archetypes.json `tablet` column", when_hit: "not a cap: E1 of a tablet capture is judged <= N when the archetype has no `tablet` column (limitSource `tablet-default`)" },
        Limit { name: "TABLET_MAX_ACTIONS", value: TABLET_MAX_ACTIONS, unit: "actions", control: "archetypes.json `tablet` column", when_hit: "not a cap: E4 of a tablet capture is judged <= N when the archetype has no `tablet` column (limitSource `tablet-default`)" },
        Limit { name: "TABLET_CONTENT_START_PERCENT", value: TABLET_CONTENT_START_PERCENT, unit: "percent of viewport height", control: "archetypes.json `tablet` column", when_hit: "not a cap: E7 of a tablet capture is judged <= N/100 when the archetype has no `tablet` column (limitSource `tablet-default`)" },
        Limit { name: "DHASH_MAX_DISTANCE", value: DHASH_MAX_DISTANCE as u64, unit: "bits", control: "none", when_hit: "not a cap: image dHash distance above it is a `verify` difference" },
        Limit { name: "METRIC_COUNT", value: METRIC_COUNT as u64, unit: "metrics", control: "none", when_hit: "not a cap: the number of E metrics a capture is scored on" },
    ]
}

/// `comparegraph ui limits`: every cap, its value, unit, control and what is reported when it is hit.
pub fn limits() -> Value {
    let items: Vec<Value> = table().iter().map(|l| json!({ "name": l.name, "value": l.value, "unit": l.unit, "control": l.control, "whenHit": l.when_hit })).collect();
    let total = items.len();
    json!({ "items": items, "page": { "next": null, "total": total }, "probe": "tools/ui/limits.mjs lists the capture-side caps (elements, time, bytes)" })
}
