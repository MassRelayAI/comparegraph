//! The generated tool catalog: every MCP tool (name, description, inputSchema) plus the envelope error codes.
//! `docs/catalog.json` is its committed rendering; `comparegraph catalog --check` fails when it is stale.
use serde_json::{json, Value};

/// Every code an envelope `error.code` can carry (`AppError::code` plus `catalog_stale`).
pub const ERROR_CODES: &[&str] = &[
    "already_archived",
    "archive_refused",
    "backup_mismatch",
    "below_floor",
    "cannot_archive_root",
    "catalog_stale",
    "chain_broken",
    "check_failed",
    "clone_missing",
    "clone_unavailable",
    "empty",
    "head_mismatch",
    "insufficient_space",
    "invalid_arguments",
    "invalid_policy",
    "io_error",
    "manifest_mismatch",
    "none_refused",
    "not_archived",
    "not_found",
    "provider_unavailable",
    "repo_archived",
    "store_busy",
    "store_error",
];

/// Next-step kinds a caller can branch on without reading prose (bounded, like REA's `AnalysisErrorRecovery`):
/// `fix_arguments` (the call was wrong: re-read the tool's inputSchema), `retry` (same call later), `run_command` (a write verb must run first),
/// `inspect` (a read verb shows what is wrong), `choose_candidate` (`error.data.candidates` lists the valid values), `none` (nothing to do; the refusal is the answer).
pub const RECOVERY_KINDS: &[&str] = &["fix_arguments", "retry", "run_command", "inspect", "choose_candidate", "none"];

/// `(code, kind, operation, retryable)`: the machine-readable recovery of every error code. `operation` is a CLI verb line (`governance verify`);
/// its first word is a `comparegraph` subcommand, and every read verb is also the MCP tool `comparegraph_<verb with _>` (a test checks both).
/// `retryable` is true only when the SAME call can succeed later with no change by the caller. The per-error prose stays in `error.hint`.
pub const RECOVERY: &[(&str, &str, &str, bool)] = &[
    ("already_archived", "inspect", "repos --archived", false),
    ("archive_refused", "inspect", "relevance", false),
    ("backup_mismatch", "run_command", "backup", false),
    ("below_floor", "inspect", "relate", false),
    ("cannot_archive_root", "none", "", false),
    ("catalog_stale", "run_command", "catalog", false),
    ("chain_broken", "inspect", "governance verify", false),
    ("check_failed", "inspect", "check", false),
    ("clone_missing", "inspect", "doctor", false),
    ("clone_unavailable", "inspect", "check", false),
    ("empty", "run_command", "build", false),
    ("head_mismatch", "run_command", "build", false),
    ("insufficient_space", "retry", "gc", true),
    ("invalid_arguments", "fix_arguments", "catalog", false),
    ("invalid_policy", "fix_arguments", "relevance", false),
    ("io_error", "inspect", "doctor", false),
    ("manifest_mismatch", "inspect", "governance verify", false),
    ("none_refused", "inspect", "absent-check", false),
    ("not_archived", "inspect", "repos --archived", false),
    ("not_found", "choose_candidate", "search", false),
    ("provider_unavailable", "inspect", "doctor", false),
    ("repo_archived", "run_command", "restore", false),
    ("store_busy", "retry", "", true),
    ("store_error", "inspect", "check", false),
];

/// The recovery of an error code (`error.recovery`); an unknown code gets `inspect` `doctor`, never nothing.
pub fn recovery(code: &str) -> Value {
    let (kind, operation, retryable) = RECOVERY.iter().find(|r| r.0 == code).map_or(("inspect", "doctor", false), |r| (r.1, r.2, r.3));
    let mut v = json!({ "kind": kind, "retryable": retryable });
    if !operation.is_empty() {
        v["operation"] = json!(operation);
    }
    v
}

/// The catalog as pretty JSON (2-space indent, trailing newline), tools sorted by name.
pub fn render(mut tools: Vec<Value>) -> String {
    tools.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    let v = json!({ "schema": 1, "version": env!("CARGO_PKG_VERSION"), "tools": tools, "error_codes": ERROR_CODES, "error_recovery": RECOVERY.iter().map(|r| (r.0.to_string(), recovery(r.0))).collect::<serde_json::Map<String, Value>>() });
    let mut s = serde_json::to_string_pretty(&v).expect("catalog serialises");
    s.push('\n');
    s
}
