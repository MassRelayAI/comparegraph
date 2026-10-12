//! MCP stdio server: JSON-RPC 2.0, one object per line. Tools run the same verbs as the CLI and return the same envelope.
use crate::adapters::cli::{envelope_for, Cli, Cmd, FamilyCmd};
use crate::adapters::envelope::Envelope;
use crate::application::AppError;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

const PROTOCOL: &str = "2025-06-18";

fn schema(props: Value, required: &[&str]) -> Value {
    json!({ "$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object",
            "properties": props, "required": required, "additionalProperties": false })
}
fn tool(name: &str, desc: &str, input: Value) -> Value {
    json!({ "name": name, "description": desc, "inputSchema": input })
}
fn page_props() -> Value {
    json!({ "limit": {"type":"integer","minimum":1,"maximum":500,"description":"page size, default 50"},
            "cursor": {"type":"string","description":"opaque cursor from a previous page"} })
}

pub fn tools() -> Vec<Value> {
    let mut search_props = json!({
        "query": {"type":"string","description":"words to find in symbol names and paths"},
        "repo": {"type":"string"}, "kind": {"type":"string","description":"node kind, e.g. function, class, file"} });
    search_props
        .as_object_mut()
        .unwrap()
        .extend(page_props().as_object().unwrap().clone());
    let mut comm_props = json!({ "repo": {"type":"string"} });
    comm_props
        .as_object_mut()
        .unwrap()
        .extend(page_props().as_object().unwrap().clone());
    vec![
        tool("comparegraph_repos", "List registered repos (name, licence, lane, role, head) with the latest run summary (status, nodes, communities, store_hash). Returns a compact envelope.", schema(json!({}), &[])),
        tool("comparegraph_search", "Full-text search over symbols of each repo's latest run. Returns a compact, paged envelope of hits with licence-lane labels.", schema(search_props, &["query"])),
        tool("comparegraph_show", "One symbol with callers, callees, community and licence lane. Compact by default; detail=full adds its current source lines.", schema(json!({
            "repo": {"type":"string"}, "qualified_name": {"type":"string"},
            "detail": {"type":"string","enum":["compact","full"]},
            "max_callers": {"type":"integer","minimum":1,"maximum":2000,"description":"cap on callers and callees listed, default 50; a truncated list is reported in limitations"} }), &["repo", "qualified_name"])),
        tool("comparegraph_communities", "Module clusters of a repo's latest run (label, keywords, cohesion, size). Returns a compact, paged envelope.", schema(comm_props, &["repo"])),
        tool("comparegraph_check", "Verify the store: schema, determinism, locations. Returns a compact envelope listing problems, if any.", schema(json!({}), &[])),
        tool("comparegraph_families", "Registered taxonomy families with feature counts. Returns a compact envelope.", schema(json!({}), &[])),
        tool("comparegraph_map", "Candidate symbols and communities per feature of a family in one clone. Candidates are never verdicts; writes nothing.", schema(json!({
            "repo": {"type":"string"}, "family": {"type":"string"}, "feature": {"type":"string","description":"one feature id"},
            "limit": {"type":"integer","minimum":1,"maximum":50,"description":"symbols per feature, default 5"} }), &["repo", "family"])),
        tool("comparegraph_relate", "Latest stored relatedness band per (clone, family). Reads only: it does not compute; run `comparegraph relate` to refresh.", schema(json!({
            "repo": {"type":"string"}, "family": {"type":"string"} }), &[])),
        tool("comparegraph_claims", "Latest claim per (clone, feature) with its citations; history=true lists every row with its supersede chain.", schema(json!({
            "repo": {"type":"string"}, "family": {"type":"string"}, "feature": {"type":"string"}, "history": {"type":"boolean"} }), &[])),
        tool("comparegraph_matrix", "Active features of a family against every clone: verdicts, prevalence, gaps and differentiators.", schema(json!({
            "family": {"type":"string"} }), &["family"])),
        tool("comparegraph_claim", "WRITE: record a claim (has|partial|none|unknown) about a feature in a clone. author is required; has/partial need at least one citation in `at` (path:start-end) that resolves now; none needs `searched` terms that find nothing.", schema(json!({
            "repo": {"type":"string"}, "feature": {"type":"string"}, "family": {"type":"string","description":"disambiguates a feature id that exists in several families"},
            "verdict": {"type":"string","enum":["has","partial","none","unknown"]},
            "author": {"type":"string","description":"who makes the claim; required"},
            "at": {"type":"array","items":{"type":"string"},"description":"citations, path:start-end or path:line, relative to the clone"},
            "searched": {"type":"string","description":"terms the absence was checked with (none)"},
            "note": {"type":"string"} }), &["repo", "feature", "verdict", "author"])),
        tool("comparegraph_traverse", "Nodes reachable from a symbol over calls/extends/implements (kinds), with hop distance, path_confidence and the path to each. Bounded: depth<=10, max_nodes<=2000; truncation is recorded in evidence.limitations.", schema(json!({
            "repo": {"type":"string"}, "qualified_name": {"type":"string"},
            "direction": {"type":"string","enum":["callers","callees","both"]},
            "depth": {"type":"integer","minimum":1,"maximum":10}, "max_nodes": {"type":"integer","minimum":1,"maximum":2000},
            "kinds": {"type":"string","description":"comma list of calls,extends,implements,imports,tests"},
            "min_confidence": {"type":"number","enum":[0.5,1.0]} }), &["repo", "qualified_name"])),
        tool("comparegraph_impact", "Blast radius: everything that transitively depends on a symbol or on file(s), grouped by hop with its community, plus the tests that reach it.", schema(json!({
            "repo": {"type":"string"}, "qualified_name": {"type":"string"}, "files": {"type":"array","items":{"type":"string"}},
            "depth": {"type":"integer","minimum":1,"maximum":10}, "max_nodes": {"type":"integer","minimum":1,"maximum":2000},
            "min_confidence": {"type":"number","enum":[0.5,1.0]} }), &["repo"])),
        tool("comparegraph_tests_for", "Tests linked to a symbol directly or through at most 3 call hops.", schema(json!({
            "repo": {"type":"string"}, "qualified_name": {"type":"string"}, "max_nodes": {"type":"integer","minimum":1,"maximum":2000},
            "min_confidence": {"type":"number","enum":[0.5,1.0]} }), &["repo", "qualified_name"])),
        tool("comparegraph_entry_points", "Symbols nothing calls that are exported, handler-named or main. Paged.", schema(json!({
            "repo": {"type":"string"}, "limit": {"type":"integer","minimum":1,"maximum":500}, "cursor": {"type":"string"},
            "min_confidence": {"type":"number","enum":[0.5,1.0]} }), &["repo"])),
        tool("comparegraph_dead_code", "Dead-code CANDIDATES (confidence unknown: dynamic dispatch is invisible). Paged.", schema(json!({
            "repo": {"type":"string"}, "limit": {"type":"integer","minimum":1,"maximum":500}, "cursor": {"type":"string"},
            "min_confidence": {"type":"number","enum":[0.5,1.0]} }), &["repo"])),
        tool("comparegraph_doctor", "Report the store and every provider. Returns a compact envelope.", schema(json!({}), &[])),
    ]
}

fn str_arg(a: &Value, k: &str) -> Result<Option<String>, String> {
    match a.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("{k} must be a string")),
    }
}
fn req(a: &Value, k: &str) -> Result<String, String> {
    str_arg(a, k)?.ok_or_else(|| format!("missing required argument {k}"))
}
fn limit_arg(a: &Value) -> Result<usize, String> {
    match a.get("limit") {
        None | Some(Value::Null) => Ok(50),
        Some(v) => v
            .as_u64()
            .map(|n| n as usize)
            .ok_or_else(|| "limit must be a non-negative integer".to_string()),
    }
}

fn uint_arg(a: &Value, k: &str, default: usize) -> Result<usize, String> {
    match a.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_u64()
            .map(|n| n as usize)
            .ok_or_else(|| format!("{k} must be a non-negative integer")),
    }
}
fn conf_arg(a: &Value) -> Result<f64, String> {
    match a.get("min_confidence") {
        None | Some(Value::Null) => Ok(0.5),
        Some(v) => v
            .as_f64()
            .ok_or_else(|| "min_confidence must be a number".to_string()),
    }
}

fn command(name: &str, a: &Value) -> Result<Cmd, String> {
    if !a.is_object() {
        return Err("arguments must be an object".into());
    }
    let known: &[&str] = match name {
        "comparegraph_repos" | "comparegraph_check" | "comparegraph_doctor" => &[],
        "comparegraph_search" => &["query", "repo", "kind", "limit", "cursor"],
        "comparegraph_show" => &["repo", "qualified_name", "detail", "max_callers"],
        "comparegraph_communities" => &["repo", "limit", "cursor"],
        "comparegraph_traverse" => &[
            "repo", "qualified_name", "direction", "depth", "max_nodes", "kinds", "min_confidence",
        ],
        "comparegraph_impact" => &[
            "repo", "qualified_name", "files", "depth", "max_nodes", "min_confidence",
        ],
        "comparegraph_tests_for" => &["repo", "qualified_name", "max_nodes", "min_confidence"],
        "comparegraph_entry_points" | "comparegraph_dead_code" => {
            &["repo", "limit", "cursor", "min_confidence"]
        }
        "comparegraph_families" => &[],
        "comparegraph_map" => &["repo", "family", "feature", "limit"],
        "comparegraph_relate" => &["repo", "family"],
        "comparegraph_claims" => &["repo", "family", "feature", "history"],
        "comparegraph_matrix" => &["family"],
        "comparegraph_claim" => &[
            "repo", "feature", "family", "verdict", "author", "at", "searched", "note",
        ],
        _ => return Err(format!("unknown tool {name}")),
    };
    if let Some(k) = a
        .as_object()
        .unwrap()
        .keys()
        .find(|k| !known.contains(&k.as_str()))
    {
        return Err(format!("unknown argument {k}"));
    }
    Ok(match name {
        "comparegraph_repos" => Cmd::Repos,
        "comparegraph_check" => Cmd::Check,
        "comparegraph_doctor" => Cmd::Doctor { bin: None },
        "comparegraph_search" => Cmd::Search {
            query: req(a, "query")?,
            repo: str_arg(a, "repo")?,
            kind: str_arg(a, "kind")?,
            limit: limit_arg(a)?,
            cursor: str_arg(a, "cursor")?,
        },
        "comparegraph_show" => Cmd::Show {
            repo: req(a, "repo")?,
            qualified_name: req(a, "qualified_name")?,
            detail: str_arg(a, "detail")?,
            max_callers: uint_arg(a, "max_callers", 50)?,
        },
        "comparegraph_traverse" => Cmd::Traverse {
            repo: req(a, "repo")?,
            qualified_name: req(a, "qualified_name")?,
            direction: str_arg(a, "direction")?.unwrap_or_else(|| "both".into()),
            depth: uint_arg(a, "depth", 3)?,
            max_nodes: uint_arg(a, "max_nodes", 200)?,
            kinds: str_arg(a, "kinds")?.unwrap_or_else(|| "calls,extends,implements".into()),
            min_confidence: conf_arg(a)?,
        },
        "comparegraph_impact" => Cmd::Impact {
            repo: req(a, "repo")?,
            qualified_name: str_arg(a, "qualified_name")?,
            file: match a.get("files") {
                None | Some(Value::Null) => vec![],
                Some(Value::Array(v)) => v
                    .iter()
                    .map(|x| {
                        x.as_str()
                            .map(String::from)
                            .ok_or_else(|| "files must be an array of strings".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                Some(_) => return Err("files must be an array of strings".into()),
            },
            depth: uint_arg(a, "depth", 5)?,
            max_nodes: uint_arg(a, "max_nodes", 500)?,
            min_confidence: conf_arg(a)?,
        },
        "comparegraph_tests_for" => Cmd::TestsFor {
            repo: req(a, "repo")?,
            qualified_name: req(a, "qualified_name")?,
            max_nodes: uint_arg(a, "max_nodes", 200)?,
            min_confidence: conf_arg(a)?,
        },
        "comparegraph_entry_points" => Cmd::EntryPoints {
            repo: req(a, "repo")?,
            limit: limit_arg(a)?,
            cursor: str_arg(a, "cursor")?,
            min_confidence: conf_arg(a)?,
        },
        "comparegraph_dead_code" => Cmd::DeadCode {
            repo: req(a, "repo")?,
            limit: limit_arg(a)?,
            cursor: str_arg(a, "cursor")?,
            min_confidence: conf_arg(a)?,
        },
        "comparegraph_families" => Cmd::Family {
            what: FamilyCmd::List,
        },
        "comparegraph_map" => Cmd::Map {
            repo: req(a, "repo")?,
            family: req(a, "family")?,
            feature: str_arg(a, "feature")?,
            limit: match a.get("limit") {
                None | Some(Value::Null) => 5,
                Some(v) => match v.as_u64() {
                    Some(n @ 1..=50) => n as usize,
                    _ => return Err("limit must be an integer in 1..50".to_string()),
                },
            },
        },
        "comparegraph_relate" => Cmd::Relate {
            repo: str_arg(a, "repo")?,
            family: str_arg(a, "family")?,
            stored: true,
        },
        "comparegraph_claims" => Cmd::Claims {
            repo: str_arg(a, "repo")?,
            family: str_arg(a, "family")?,
            feature: str_arg(a, "feature")?,
            history: match a.get("history") {
                None | Some(Value::Null) => false,
                Some(Value::Bool(b)) => *b,
                Some(_) => return Err("history must be a boolean".into()),
            },
        },
        "comparegraph_matrix" => Cmd::Matrix {
            family: req(a, "family")?,
        },
        "comparegraph_claim" => {
            let author = req(a, "author")?;
            if author.trim().is_empty() {
                return Err("author must not be empty".into());
            }
            let at = match a.get("at") {
                None | Some(Value::Null) => vec![],
                Some(Value::Array(v)) => v
                    .iter()
                    .map(|x| {
                        x.as_str()
                            .map(String::from)
                            .ok_or_else(|| "at must be an array of strings".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                Some(_) => return Err("at must be an array of strings".into()),
            };
            Cmd::Claim {
                repo: req(a, "repo")?,
                feature: req(a, "feature")?,
                verdict: req(a, "verdict")?,
                author,
                family: str_arg(a, "family")?,
                at,
                searched: str_arg(a, "searched")?,
                note: str_arg(a, "note")?,
            }
        }
        _ => Cmd::Communities {
            repo: req(a, "repo")?,
            limit: limit_arg(a)?,
            cursor: str_arg(a, "cursor")?,
        },
    })
}

fn call(root: &Path, params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let empty = json!({});
    let args = match params.get("arguments") {
        None | Some(Value::Null) => &empty,
        Some(v) => v,
    };
    let (text, is_error) = match command(name, args) {
        Err(m) => {
            let verb = name.strip_prefix("comparegraph_").unwrap_or("mcp");
            (
                Envelope::err(verb, &AppError::InvalidArguments(m)).to_json(),
                true,
            )
        }
        Ok(cmd) => {
            let env = envelope_for(&Cli {
                root: PathBuf::from(root),
                pretty: false,
                allow_unverified: false,
                cmd,
            });
            (env.to_json(), !env.ok)
        }
    };
    json!({ "content": [{"type":"text","text":text}], "isError": is_error })
}

fn err(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc":"2.0", "id": id, "error": {"code":code,"message":message} })
}

/// Handle one line; None means no reply (notification).
fn handle(root: &Path, line: &str) -> Option<Value> {
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return Some(err(Value::Null, -32700, "parse error")),
    };
    let id = v.get("id").cloned();
    let method = v.get("method").and_then(Value::as_str);
    if v.get("jsonrpc").and_then(Value::as_str) != Some("2.0") || method.is_none() || !v.is_object()
    {
        return Some(err(id.unwrap_or(Value::Null), -32600, "invalid request"));
    }
    let id = id?; // notification: no reply
    let method = method.unwrap();
    let params = v.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => json!({ "protocolVersion": PROTOCOL, "capabilities": {"tools": {}},
                                "serverInfo": {"name":"comparegraph","version":env!("CARGO_PKG_VERSION")} }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => call(root, &params),
        _ => return Some(err(id, -32601, "method not found")),
    };
    Some(json!({ "jsonrpc":"2.0", "id": id, "result": result }))
}

pub fn serve<R: BufRead, W: Write>(root: &Path, mut input: R, mut out: W) {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match input.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let line = String::from_utf8_lossy(&buf);
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle(root, &line) {
            if writeln!(out, "{reply}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
    }
}
