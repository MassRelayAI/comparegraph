//! The generated tool catalog: every MCP tool (name, description, inputSchema) plus the envelope error codes.
//! `docs/catalog.json` is its committed rendering; `comparegraph catalog --check` fails when it is stale.
use serde_json::{json, Value};

/// Every code an envelope `error.code` can carry (`AppError::code` plus `catalog_stale`).
pub const ERROR_CODES: &[&str] = &[
    "below_floor",
    "catalog_stale",
    "check_failed",
    "empty",
    "invalid_arguments",
    "io_error",
    "not_found",
    "provider_unavailable",
    "store_error",
];

/// The catalog as pretty JSON (2-space indent, trailing newline), tools sorted by name.
pub fn render(mut tools: Vec<Value>) -> String {
    tools.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    let v = json!({ "schema": 1, "version": env!("CARGO_PKG_VERSION"), "tools": tools, "error_codes": ERROR_CODES });
    let mut s = serde_json::to_string_pretty(&v).expect("catalog serialises");
    s.push('\n');
    s
}
