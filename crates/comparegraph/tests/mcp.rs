use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn clone() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("src")).unwrap();
    std::fs::write(
        d.path().join("src/a.ts"),
        "export function f() { return g(); }\nexport function g() { return 1; }\n",
    )
    .unwrap();
    std::fs::write(d.path().join("LICENSE"), "Apache License\nVersion 2.0\n").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "."],
        vec![
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "x",
        ],
    ] {
        Command::new("git")
            .arg("-C")
            .arg(d.path())
            .args(&args)
            .status()
            .unwrap();
    }
    d
}
fn bin() -> String {
    assert_cmd::cargo::cargo_bin("comparegraph")
        .to_string_lossy()
        .into_owned()
}
fn built_ws() -> (tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let c = clone();
    for args in [
        vec![
            "add".to_string(),
            c.path().to_string_lossy().into_owned(),
            "--name".into(),
            "a".into(),
        ],
        vec!["build".into(), "a".into()],
    ] {
        assert!(Command::new(bin())
            .arg("--root")
            .arg(ws.path())
            .args(args)
            .output()
            .unwrap()
            .status
            .success());
    }
    (ws, c)
}
/// Feed raw lines, close stdin, return (exit success, stdout lines parsed).
fn session(ws: &std::path::Path, lines: &[String]) -> (bool, Vec<Value>) {
    let mut ch = Command::new(bin())
        .arg("--root")
        .arg(ws)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = ch.stdin.take().unwrap();
    for l in lines {
        writeln!(stdin, "{l}").unwrap();
    }
    drop(stdin);
    let rd = BufReader::new(ch.stdout.take().unwrap());
    let out: Vec<Value> = rd
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).expect("stdout line is JSON"))
        .collect();
    (ch.wait().unwrap().success(), out)
}
fn req(id: i64, method: &str, params: Value) -> String {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string()
}

#[test]
fn initialize_and_list_tools() {
    let ws = tempfile::tempdir().unwrap();
    let (ok, out) = session(
        ws.path(),
        &[
            req(1, "initialize", json!({})),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string(),
            req(2, "tools/list", json!({})),
            req(3, "ping", json!({})),
        ],
    );
    assert!(ok);
    assert_eq!(out.len(), 3, "the notification gets no reply");
    assert_eq!(out[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(out[0]["result"]["serverInfo"]["name"], "comparegraph");
    assert!(out[0]["result"]["capabilities"]["tools"].is_object());
    let tools = out[1]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 12);
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object");
        assert_eq!(t["inputSchema"]["additionalProperties"], false);
        assert!(
            t["inputSchema"]["properties"].is_object() && t["inputSchema"]["required"].is_array()
        );
    }
    assert_eq!(out[2]["id"], 3);
}

fn tool_env(out: &Value) -> Value {
    serde_json::from_str(out["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn tools_call_on_a_built_workspace() {
    let (ws, _c) = built_ws();
    let (_, out) = session(
        ws.path(),
        &[
            req(
                1,
                "tools/call",
                json!({"name":"comparegraph_search","arguments":{"query":"f"}}),
            ),
            req(
                2,
                "tools/call",
                json!({"name":"comparegraph_repos","arguments":{}}),
            ),
            req(
                3,
                "tools/call",
                json!({"name":"comparegraph_communities","arguments":{"repo":"a"}}),
            ),
            req(
                4,
                "tools/call",
                json!({"name":"comparegraph_show","arguments":{"repo":"a","qualified_name":"src/a.ts::f"}}),
            ),
            req(
                5,
                "tools/call",
                json!({"name":"comparegraph_doctor","arguments":{}}),
            ),
            req(6, "tools/call", json!({"name":"comparegraph_check"})),
        ],
    );
    let s = tool_env(&out[0]);
    assert_eq!(out[0]["result"]["isError"], false);
    assert_eq!(s["verb"], "search");
    assert!(!s["data"]["items"].as_array().unwrap().is_empty());
    let r = tool_env(&out[1]);
    let repo = &r["data"]["items"][0];
    assert_eq!(repo["name"], "a");
    assert_eq!(repo["lane"], "take");
    assert_eq!(repo["latest_run"]["status"], "done");
    assert!(repo["latest_run"]["nodes"].as_u64().unwrap() > 0);
    assert!(repo["latest_run"]["communities"].as_u64().unwrap() > 0);
    assert!(repo["latest_run"]["store_hash"].is_string());
    assert_eq!(tool_env(&out[2])["verb"], "communities");
    assert_eq!(out[2]["result"]["isError"], false);
    assert_eq!(tool_env(&out[3])["verb"], "show");
    assert_eq!(tool_env(&out[4])["verb"], "doctor");
    assert_eq!(tool_env(&out[5])["verb"], "check");
}

#[test]
fn invalid_arguments_and_domain_errors_are_is_error() {
    let (ws, _c) = built_ws();
    let (_, out) = session(
        ws.path(),
        &[
            req(
                1,
                "tools/call",
                json!({"name":"comparegraph_search","arguments":{}}),
            ),
            req(
                2,
                "tools/call",
                json!({"name":"comparegraph_search","arguments":{"query":"f","bogus":1}}),
            ),
            req(
                3,
                "tools/call",
                json!({"name":"comparegraph_show","arguments":{"repo":"nope","qualified_name":"x"}}),
            ),
        ],
    );
    for o in out.iter().take(2) {
        assert_eq!(o["result"]["isError"], true);
        assert_eq!(tool_env(o)["error"]["code"], "invalid_arguments");
    }
    assert_eq!(out[2]["result"]["isError"], true);
    assert_eq!(tool_env(&out[2])["error"]["code"], "not_found");
}

#[test]
fn garbage_and_bad_requests_do_not_stop_the_server() {
    let ws = tempfile::tempdir().unwrap();
    let (ok, out) = session(
        ws.path(),
        &[
            "this is not json".to_string(),
            json!({"id":5,"method":"ping"}).to_string(),
            json!({"jsonrpc":"2.0","id":6}).to_string(),
            req(7, "no/such", json!({})),
            req(8, "ping", json!({})),
        ],
    );
    assert!(ok, "EOF exits 0");
    assert_eq!(out[0]["error"]["code"], -32700);
    assert!(out[0]["id"].is_null());
    assert_eq!(out[1]["error"]["code"], -32600);
    assert_eq!(out[1]["id"], 5);
    assert_eq!(out[2]["error"]["code"], -32600);
    assert_eq!(out[3]["error"]["code"], -32601);
    assert_eq!(out[4]["id"], 8);
    assert!(out[4]["result"].is_object());
}

#[test]
fn invalid_utf8_line_gets_parse_error_and_session_survives() {
    let ws = tempfile::tempdir().unwrap();
    let mut ch = Command::new(bin())
        .arg("--root")
        .arg(ws.path())
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = ch.stdin.take().unwrap();
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n")
        .unwrap();
    stdin
        .write_all(&[0xff, 0xfe, b' ', b'b', b'a', b'd', b'\n'])
        .unwrap();
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n")
        .unwrap();
    drop(stdin);
    let out: Vec<Value> = BufReader::new(ch.stdout.take().unwrap())
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect();
    assert!(ch.wait().unwrap().success());
    assert_eq!(out.len(), 3);
    assert_eq!(out[0]["id"], 1);
    assert_eq!(out[1]["error"]["code"], -32700);
    assert!(out[1]["id"].is_null());
    assert_eq!(out[2]["id"], 2);
}

#[test]
fn invalid_arguments_use_the_envelope_with_the_tool_verb() {
    let (ws, _c) = built_ws();
    let (_, out) = session(
        ws.path(),
        &[
            req(
                1,
                "tools/call",
                json!({"name":"comparegraph_search","arguments":{"query":"f","cursor":"abc"}}),
            ),
            req(
                2,
                "tools/call",
                json!({"name":"comparegraph_show","arguments":{"repo":"a"}}),
            ),
        ],
    );
    for (i, verb) in [(0, "search"), (1, "show")] {
        let text = out[i]["result"]["content"][0]["text"].as_str().unwrap();
        let env: Value = serde_json::from_str(text).unwrap();
        assert_eq!(out[i]["result"]["isError"], true);
        assert_eq!(env["error"]["code"], "invalid_arguments", "{env}");
        assert_eq!(env["verb"], verb);
        assert_eq!(env["ok"], false);
    }
}

#[test]
fn map_limit_out_of_range_is_invalid_arguments() {
    let (ws, _c) = built_ws();
    let lines: Vec<String> = [0, 51]
        .iter()
        .enumerate()
        .map(|(i, l)| {
            req(
                i as i64 + 1,
                "tools/call",
                json!({"name":"comparegraph_map","arguments":{"repo":"a","family":"f","limit":l}}),
            )
        })
        .collect();
    let (_, out) = session(ws.path(), &lines);
    for o in out.iter().take(2) {
        assert_eq!(o["result"]["isError"], true, "{o}");
        assert_eq!(tool_env(o)["error"]["code"], "invalid_arguments");
    }
}
