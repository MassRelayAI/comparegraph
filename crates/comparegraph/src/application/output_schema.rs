//! Compact output schemas for the MCP tools: every tool advertises, next to its `inputSchema`, the JSON Schema of the envelope it answers with.
//!
//! Approach after REA (MIT, github.com/morluto/rea, commit a016743, `src/contracts/compactOutputSchema.ts`, `compactJsonSchema.ts`): an advertised output
//! schema is bounded in bytes and always a WIDENING of the real contract - it names the keys and types a caller can rely on and leaves the rest open
//! (no `additionalProperties: false` on `data`), so a valid answer never fails it and a schema never promises more than the verb always returns.
//! Independent Rust implementation; no REA code is copied (docs/THIRD_PARTY_NOTICES.md).
//!
//! One definition per shared part (`defs`), referenced by `$ref` from every tool: the envelope, the evidence, a page, the cost and the error with its
//! `recovery`. The per-tool part is one line of [`DATA`]. The confidence classes come from the one constant below; `data` is a hand-kept one-liner per verb, and
//! `tests/output_schema.rs` runs every verb on the fixtures and validates the real answer against the advertised schema (a drifted line goes red).
use serde_json::{json, Map, Value};

/// The confidence classes of `evidence.confidence`.
pub const CONFIDENCE: &[&str] = &["observed", "derived", "inferred", "unknown"];

/// `(tool, data keys)`. A key is `name:T` (optional) or `name:T!` (always present in a successful answer); `T` is one of
/// `s` string, `i` integer, `b` boolean, `a` array, `o` object, with a trailing `?` for "or null". Keys not listed are allowed and untyped.
const DATA: &[(&str, &str)] = &[
    ("comparegraph_repos", "items:a!"),
    ("comparegraph_search", "items:a!"),
    ("comparegraph_show", "repo:s! licence:s lane:s node:o! community:s? callers:a! callers_total:i! callees:a! callees_total:i! stale:b confidence:s"),
    ("comparegraph_communities", "items:a!"),
    ("comparegraph_check", "repos:i! schema_ok:b! chain_ok:b problems:a! notes:a clone_unavailable:a"),
    ("comparegraph_relevance", "items:a! classes:o as_of:s inputs:o"),
    ("comparegraph_governance_verify", "actions:i! chain_ok:b! problems:a! head:s? store_head:s? broken_at:i? file:s archived:i"),
    ("comparegraph_families", "items:a!"),
    ("comparegraph_map", "repo:s! family:s! features:a! entry_points:i"),
    ("comparegraph_absent_check", "repo:s! family:s! feature:s! absent:b! reason:s! read_set:o! channels:o terms:a"),
    ("comparegraph_relate", "items:a!"),
    ("comparegraph_claims", "items:a!"),
    ("comparegraph_matrix", "family:s! columns:a! rows:a! comparators:i"),
    ("comparegraph_claim", "id:i! repo:s! feature:s! verdict:s! author:s! confidence:s family:s run_id:i created_at:s supersedes_id:i? note:s? locations:a"),
    ("comparegraph_traverse", "repo:s! start:s! summary:o! items:a! truncated:b! total:i!"),
    ("comparegraph_impact", "repo:s! summary:o! items:a! truncated:b! total:i! targets:a tests:a!"),
    ("comparegraph_tests_for", "repo:s! symbol:s! summary:o! items:a! truncated:b! total:i!"),
    ("comparegraph_entry_points", "repo:s! items:a!"),
    ("comparegraph_dead_code", "repo:s! items:a!"),
    ("comparegraph_doctor", "store:s! schema_version:i! repos:i providers:a! governance:o clients:a"),
    ("comparegraph_ui_limits", "items:a! probe:s"),
    ("comparegraph_ui_products", "items:a!"),
    ("comparegraph_ui_screens", "product:s! items:a!"),
    ("comparegraph_ui_show", "key:s product:s! screen:s! state:s! viewport:s! seat:s pass:i! of:i! metrics:a! layout:o style:o image:o captures:a"),
    ("comparegraph_ui_diff", "target:s! screen:s! state:s! viewport:s! ours:o! ref:o! pass:o! metrics:a! layoutDiff:a styleDiff:a images:o"),
    ("comparegraph_ui_matrix", "family:s! state:s! viewport:s products:a! items:a!"),
    ("comparegraph_ui_system", "product:s! source:s! style:o targets:a summary:s"),
    ("comparegraph_ui_flow", "product:s! task:s! stepCount:i! clicks:i steps:a! deadEnds:a"),
    ("comparegraph_ui_nav", "product:s! viewport:s! seat:s stats:o! tree:a!"),
    ("comparegraph_ui_nav_diff", "viewport:s! ours:o! ref:o! diff:a!"),
    ("comparegraph_ui_net_diff", "ours:s! ref:s! source:s! status:s! normalisation:s summary:o! captures:a!"),
    ("comparegraph_ui_graph", "product:s! viewport:s! seat:s start:s? summary:o! orphans:a unreachable:a deadActions:a depth:o"),
    ("comparegraph_ui_path", "product:s! from:s! to:s! found:b! clicks:i? steps:a maxLen:i"),
    ("comparegraph_ui_graph_diff", "viewport:s! ours:o! ref:o! diff:a!"),
    ("comparegraph_ui_journey", "journey:s! items:a! pairwise:a title:s goalArchetype:s?"),
    ("comparegraph_ui_coverage", "runId:s! complete:b! table:a! blankCells:a"),
    ("comparegraph_ui_verify", "compared:i! identical:b! differences:a! total:i onlyInA:a onlyInB:a unstable:a"),
];

/// The `verb` an envelope of this tool carries: the tool name minus the prefix with `_` as `-`, except `families` (the CLI verb is `family`).
pub fn verb_of_tool(tool: &str) -> String {
    match tool.strip_prefix("comparegraph_").unwrap_or(tool).replace('_', "-").as_str() {
        "families" => "family".into(),
        v => v.into(),
    }
}

fn ty(t: &str) -> Value {
    let (base, null) = match t.strip_suffix('?') {
        Some(b) => (b, true),
        None => (t, false),
    };
    let name = match base {
        "s" => "string",
        "i" => "integer",
        "b" => "boolean",
        "a" => "array",
        "o" => "object",
        other => panic!("output_schema: unknown type letter `{other}`"),
    };
    if null {
        json!({ "type": [name, "null"] })
    } else {
        json!({ "type": name })
    }
}

/// The `data` schema of one tool, parsed from its [`DATA`] line.
fn data_shape(tool: &str) -> Value {
    let spec = DATA.iter().find(|d| d.0 == tool).unwrap_or_else(|| panic!("output_schema: no DATA line for {tool}")).1;
    let (mut props, mut required) = (Map::new(), vec![]);
    for k in spec.split(' ') {
        let (name, t) = k.split_once(':').unwrap_or_else(|| panic!("output_schema: bad key `{k}`"));
        let (t, req) = t.strip_suffix('!').map_or((t, false), |t| (t, true));
        props.insert(name.to_string(), ty(t));
        if req {
            required.push(json!(name));
        }
    }
    json!({ "type": "object", "required": required, "properties": props })
}

/// The shared definitions (the same in every tool's schema, defined once here). Kept small on purpose: `tools/list` carries 37 of them.
/// The code and kind lists are not repeated as enums on the wire (24 + 6 names per tool); `tests/error_recovery.rs` and `tests/output_schema.rs` pin them.
fn defs() -> Value {
    json!({
        "evidence": { "type": "object", "required": ["confidence", "limitations"], "properties": {
            "confidence": {"enum": CONFIDENCE}, "limitations": {"type":"array","items":{"type":"object","required":["code","detail"]}} } },
        "recovery": { "type": "object", "required": ["kind", "retryable"], "properties": {
            "kind": {"type":"string"}, "operation": {"type":"string"}, "retryable": {"type":"boolean"} } },
        "error": { "type": "object", "required": ["code", "message", "recovery"], "properties": {
            "code": {"type":"string"}, "message": {"type":"string"}, "hint": {"type":"string"}, "recovery": {"$ref":"#/$defs/recovery"} } },
    })
}

/// The advertised output schema of a tool: the envelope, with this tool's `verb` and `data` shape.
pub fn output_schema(tool: &str) -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["ok", "verb", "schema", "data", "evidence", "cost"],
        "properties": {
            "ok": {"type":"boolean"},
            "verb": {"const": verb_of_tool(tool)},
            "schema": {"const": 1},
            "data": {"type":["object","null"]},
            "evidence": {"$ref":"#/$defs/evidence"},
            "page": {"type":"object","required":["next","total"]},
            "cost": {"type":"object","required":["rows","bytes"]},
            "error": {"$ref":"#/$defs/error"},
        },
        "anyOf": [
            { "properties": { "ok": {"const": true}, "data": data_shape(tool) } },
            { "properties": { "ok": {"const": false} }, "required": ["error"] },
        ],
        "$defs": defs(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_tool_has_one_data_line_and_the_lines_parse() {
        for t in crate::adapters::mcp::tools() {
            let n = t["name"].as_str().unwrap();
            assert_eq!(DATA.iter().filter(|d| d.0 == n).count(), 1, "{n}");
            let s = output_schema(n);
            assert!(s["anyOf"][0]["properties"]["data"]["required"].is_array());
        }
        assert_eq!(DATA.len(), crate::adapters::mcp::tools().len(), "a DATA line for a tool that does not exist");
    }
}
