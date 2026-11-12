//! REA-CG: every MCP tool advertises a compact `outputSchema`; the real answer of every tool on the fixtures validates against it,
//! its error answers do too, and a tampered answer does not (the check goes red). The schema table lives in `application/output_schema.rs`.
mod common;
use common::Ws;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn fx(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(name)
}
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let (p, q) = (e.path(), to.join(e.file_name()));
        if e.file_type().unwrap().is_dir() {
            copy_dir(&p, &q);
        } else {
            std::fs::copy(&p, &q).unwrap();
        }
    }
}

/// A minimal JSON Schema (2020-12) validator for the keywords the advertised schemas use. An unknown keyword panics: it is never a silent pass.
fn validate(root: &Value, schema: &Value, v: &Value, at: &str, errs: &mut Vec<String>) {
    let Some(s) = schema.as_object() else {
        if schema != &Value::Bool(true) {
            errs.push(format!("{at}: schema is not an object"));
        }
        return;
    };
    for (k, sv) in s {
        match k.as_str() {
            "$schema" | "$defs" | "description" | "title" | "default" => {}
            "$ref" => {
                let name = sv.as_str().unwrap().strip_prefix("#/$defs/").unwrap_or_else(|| panic!("unsupported $ref {sv}"));
                validate(root, &root["$defs"][name], v, at, errs);
            }
            "type" => {
                let ok = |t: &str| match t {
                    "object" => v.is_object(),
                    "array" => v.is_array(),
                    "string" => v.is_string(),
                    "boolean" => v.is_boolean(),
                    "null" => v.is_null(),
                    "integer" => v.is_i64() || v.is_u64(),
                    "number" => v.is_number(),
                    t => panic!("type {t}"),
                };
                let types: Vec<&str> = match sv {
                    Value::String(t) => vec![t.as_str()],
                    Value::Array(a) => a.iter().map(|x| x.as_str().unwrap()).collect(),
                    _ => panic!("type"),
                };
                if !types.iter().any(|t| ok(t)) {
                    errs.push(format!("{at}: expected {types:?}, got {}", short(v)));
                }
            }
            "const" => {
                if v != sv {
                    errs.push(format!("{at}: expected {sv}, got {}", short(v)));
                }
            }
            "enum" => {
                if !sv.as_array().unwrap().contains(v) {
                    errs.push(format!("{at}: {} not in {sv}", short(v)));
                }
            }
            "required" => {
                for r in sv.as_array().unwrap() {
                    if v.is_object() && v.get(r.as_str().unwrap()).is_none() {
                        errs.push(format!("{at}: missing `{}`", r.as_str().unwrap()));
                    }
                }
            }
            "properties" => {
                if let Some(o) = v.as_object() {
                    for (pk, ps) in sv.as_object().unwrap() {
                        if let Some(pv) = o.get(pk) {
                            validate(root, ps, pv, &format!("{at}.{pk}"), errs);
                        }
                    }
                }
            }
            "additionalProperties" => {
                if let (Some(o), Some(props)) = (v.as_object(), s.get("properties")) {
                    if sv == &Value::Bool(false) {
                        for k in o.keys() {
                            if props.get(k).is_none() {
                                errs.push(format!("{at}: unexpected `{k}`"));
                            }
                        }
                    }
                }
            }
            "items" => {
                if let Some(a) = v.as_array() {
                    for (i, x) in a.iter().enumerate() {
                        validate(root, sv, x, &format!("{at}[{i}]"), errs);
                    }
                }
            }
            "anyOf" => {
                let any = sv.as_array().unwrap().iter().any(|alt| {
                    let mut e = vec![];
                    validate(root, alt, v, at, &mut e);
                    e.is_empty()
                });
                if !any {
                    errs.push(format!("{at}: matches none of anyOf"));
                }
            }
            "if" => {
                let mut e = vec![];
                validate(root, sv, v, at, &mut e);
                if e.is_empty() {
                    if let Some(t) = s.get("then") {
                        validate(root, t, v, at, errs);
                    }
                }
            }
            "then" => {}
            "minimum" => {
                if v.as_f64().is_some_and(|n| n < sv.as_f64().unwrap()) {
                    errs.push(format!("{at}: below {sv}"));
                }
            }
            other => panic!("validator does not know the keyword `{other}` at {at}"),
        }
    }
}
fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 80 {
        format!("{}...", &s[..80])
    } else {
        s
    }
}

struct Mcp {
    child: std::process::Child,
    rd: BufReader<std::process::ChildStdout>,
    n: i64,
}
impl Mcp {
    fn start(root: &Path) -> Mcp {
        let mut child = Command::new(assert_cmd::cargo::cargo_bin("comparegraph"))
            .arg("--root")
            .arg(root)
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let rd = BufReader::new(child.stdout.take().unwrap());
        Mcp { child, rd, n: 0 }
    }
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.n += 1;
        let line = json!({"jsonrpc":"2.0","id":self.n,"method":method,"params":params});
        writeln!(self.child.stdin.as_mut().unwrap(), "{line}").unwrap();
        let mut out = String::new();
        self.rd.read_line(&mut out).unwrap();
        serde_json::from_str(&out).unwrap()
    }
    fn call(&mut self, name: &str, args: Value) -> Value {
        self.rpc("tools/call", json!({ "name": name, "arguments": args }))["result"].clone()
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.wait();
    }
}

/// (tool, arguments, which ui root: 0 = fixtures/ui, 1 = fixtures/ui-contract).
fn calls() -> Vec<(&'static str, Value, usize)> {
    let q = "src/net/a.py::retry_with_backoff";
    vec![
        ("comparegraph_repos", json!({}), 0),
        ("comparegraph_search", json!({"query":"retry"}), 0),
        ("comparegraph_show", json!({"repo":"a","qualified_name":q}), 0),
        ("comparegraph_communities", json!({"repo":"a"}), 0),
        ("comparegraph_check", json!({}), 0),
        ("comparegraph_relevance", json!({}), 0),
        ("comparegraph_governance_verify", json!({}), 0),
        ("comparegraph_families", json!({}), 0),
        ("comparegraph_map", json!({"repo":"a","family":"fam"}), 0),
        ("comparegraph_absent_check", json!({"repo":"a","family":"fam","feature":"routing.retry"}), 0),
        ("comparegraph_relate", json!({}), 0),
        ("comparegraph_claims", json!({}), 0),
        ("comparegraph_matrix", json!({"family":"fam"}), 0),
        ("comparegraph_claim", json!({"repo":"b","feature":"memory.recall","verdict":"unknown","author":"t"}), 0),
        ("comparegraph_traverse", json!({"repo":"a","qualified_name":q}), 0),
        ("comparegraph_impact", json!({"repo":"a","qualified_name":q}), 0),
        ("comparegraph_tests_for", json!({"repo":"a","qualified_name":q}), 0),
        ("comparegraph_entry_points", json!({"repo":"a"}), 0),
        ("comparegraph_dead_code", json!({"repo":"a"}), 0),
        ("comparegraph_doctor", json!({}), 0),
        ("comparegraph_ui_limits", json!({}), 0),
        ("comparegraph_ui_products", json!({}), 0),
        ("comparegraph_ui_screens", json!({"product":"cluttered"}), 0),
        ("comparegraph_ui_show", json!({"product":"cluttered","screen":"room"}), 0),
        ("comparegraph_ui_diff", json!({"target":"room","ours":"cluttered","ref":"clean"}), 0),
        ("comparegraph_ui_matrix", json!({"family":"surfaces"}), 0),
        ("comparegraph_ui_system", json!({"product":"cluttered"}), 0),
        ("comparegraph_ui_flow", json!({"product":"cluttered","task":"create-workflow"}), 0),
        ("comparegraph_ui_nav", json!({"product":"cluttered"}), 0),
        ("comparegraph_ui_nav_diff", json!({"ours":"cluttered","ref":"clean"}), 0),
        ("comparegraph_ui_net_diff", json!({"ours":"cluttered","ref":"clean","screen":"room","state":"populated","device":"1440x900"}), 1),
        ("comparegraph_ui_owners", json!({"screen":"room"}), 0),
        ("comparegraph_ui_screens_of", json!({"file":"apps/web/src/App.tsx"}), 0),
        ("comparegraph_ui_graph", json!({"product":"clean"}), 1),
        ("comparegraph_ui_path", json!({"product":"clean","from":"room","to":"inbox"}), 1),
        ("comparegraph_ui_graph_diff", json!({"ours":"cluttered","ref":"clean"}), 1),
        ("comparegraph_ui_journey", json!({"journey":"j1"}), 0),
        ("comparegraph_ui_coverage", json!({"run":"run-a"}), 0),
        ("comparegraph_ui_verify", json!({"run_a":"run-a","run_b":"run-b"}), 0),
    ]
}

fn fixture() -> (Ws, tempfile::TempDir, tempfile::TempDir) {
    let ws = Ws::new();
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    copy_dir(&fx("ui"), a.path());
    copy_dir(&fx("ui-contract"), b.path());
    (ws, a, b)
}
fn envelope(r: &Value) -> Value {
    serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn every_tool_advertises_an_output_schema_and_its_real_answers_validate() {
    let (ws, a, b) = fixture();
    let mut m = Mcp::start(ws.root());
    let list = m.rpc("tools/list", json!({}))["result"]["tools"].clone();
    let tools = list.as_array().unwrap();
    let covered: std::collections::BTreeSet<&str> = calls().iter().map(|c| c.0).collect();
    for t in tools {
        let n = t["name"].as_str().unwrap();
        assert!(t["outputSchema"].is_object(), "{n} has no outputSchema");
        assert!(covered.contains(n), "{n} has no conformance call in calls()");
    }
    assert_eq!(covered.len(), tools.len());
    let dump = std::env::var_os("CG_DUMP_SHAPES").is_some();
    let mut failures = vec![];
    for (name, mut args, which) in calls() {
        if name.contains("_ui_") {
            args["ui_root"] = json!(if which == 0 { a.path() } else { b.path() }.to_string_lossy());
        }
        let r = m.call(name, args);
        let schema = tools.iter().find(|t| t["name"] == name).unwrap()["outputSchema"].clone();
        let env = envelope(&r);
        if dump {
            let d = &env["data"];
            let keys: Vec<String> = d.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
            eprintln!("SHAPE {name} ok={} data={} keys={keys:?} err={}", env["ok"], if d.is_array() { "array" } else if d.is_object() { "object" } else { "other" }, env["error"]["code"]);
        }
        // the MCP structuredContent is the very same envelope
        assert_eq!(r["structuredContent"], env, "{name}: structuredContent differs from the text envelope");
        let mut errs = vec![];
        validate(&schema, &schema, &env, "$", &mut errs);
        if !errs.is_empty() {
            failures.push(format!("{name}: {}", errs.join("; ")));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn error_answers_validate_too_and_a_tampered_answer_does_not() {
    let (ws, _a, _b) = fixture();
    let mut m = Mcp::start(ws.root());
    let list = m.rpc("tools/list", json!({}))["result"]["tools"].clone();
    let schema = list.as_array().unwrap().iter().find(|t| t["name"] == "comparegraph_show").unwrap()["outputSchema"].clone();
    for r in [m.call("comparegraph_show", json!({"repo":"nope","qualified_name":"x"})), m.call("comparegraph_show", json!({"bogus": 1}))] {
        let env = envelope(&r);
        assert_eq!(env["ok"], false);
        // the schema keeps the code and kind lists off the wire; the test pins them here
        assert!(comparegraph::application::catalog::ERROR_CODES.contains(&env["error"]["code"].as_str().unwrap()), "{env}");
        assert!(comparegraph::application::catalog::RECOVERY_KINDS.contains(&env["error"]["recovery"]["kind"].as_str().unwrap()), "{env}");
        let mut errs = vec![];
        validate(&schema, &schema, &env, "$", &mut errs);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(r["structuredContent"], env);
    }
    // red: break each required part of a good answer and the schema must object
    let good = envelope(&m.call("comparegraph_show", json!({"repo":"a","qualified_name":"src/net/a.py::retry_with_backoff"})));
    assert_eq!(good["ok"], true, "{good}");
    let mutations: Vec<fn(&mut Value)> = vec![
        |e| {
            e.as_object_mut().unwrap().remove("evidence");
        },
        |e| e["ok"] = json!("yes"),
        |e| e["verb"] = json!("search"),
        |e| e["data"] = json!([1]),
        |e| e["evidence"]["confidence"] = json!("sure"),
        |e| e["data"] = json!({}),
    ];
    for mutate in mutations {
        let mut bad = good.clone();
        mutate(&mut bad);
        let mut errs = vec![];
        validate(&schema, &schema, &bad, "$", &mut errs);
        assert!(!errs.is_empty(), "a tampered answer passed: {bad}");
    }
}

#[test]
fn the_advertised_schemas_are_compact_and_share_their_definitions() {
    let cat: Value = serde_json::from_str(include_str!("../../../docs/catalog.json")).unwrap();
    let tools = cat["tools"].as_array().unwrap();
    let mut total = 0;
    for t in tools {
        let s = t["outputSchema"].to_string();
        total += s.len();
        assert!(s.len() <= 1700, "{} output schema is {} bytes", t["name"], s.len());
        assert!(t["outputSchema"]["$defs"].is_object());
    }
    assert!(total <= 55_000, "all output schemas together are {total} bytes");
}
