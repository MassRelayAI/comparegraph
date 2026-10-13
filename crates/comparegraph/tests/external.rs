//! The external engine host (Plan 6 C1): fixtures are tiny scripts named in `<workspace>/comparegraph.toml`.
use assert_cmd::Command;
use std::path::{Path, PathBuf};
use std::process::Command as Sys;

const GOOD: &str = r#"#!/usr/bin/env python3
import json, os, sys
req = json.loads(sys.stdin.readline())
if req["op"] == "capabilities":
    print(json.dumps({"ok": True, "data": {"languages": ["typescript"], "edge_kinds": ["calls", "contains"], "communities": False, "embeddings": False}}))
    sys.exit(0)
root = os.environ["COMPAREGRAPH_REPO_ROOT"]
assert root == req["repo_root"]
print(json.dumps({"progress": {"files": 1}}), flush=True)
nodes, edges = [], []
exp = os.path.join(root, "expected.json")
if os.path.exists(exp):  # conformance: an oracle that echoes the fixture's answer
    e = json.load(open(exp))
    for qn in e["nodes"]:
        p, n = qn.split("::")
        nodes.append({"kind": "function", "name": n, "qualified_name": qn, "path": p, "line_start": 1, "line_end": 2, "language": "x"})
    for s in e["edges"]:
        k, a, b = s.split(" ")
        edges.append({"kind": k, "source_qn": a, "target_qn": b, "confidence": 1})
else:
    nodes = [
        {"kind": "file", "name": "a.ts", "qualified_name": "src/a.ts", "path": "src/a.ts", "language": "typescript"},
        {"kind": "function", "name": "f", "qualified_name": "src/a.ts::f", "path": "src/a.ts", "lines": [1, 1], "language": "typescript"},
        {"kind": "gadget", "name": "z", "qualified_name": "src/a.ts::z", "path": "src/a.ts"},
    ]
    edges = [
        {"kind": "contains", "source_qn": "src/a.ts", "target_qn": "src/a.ts::f", "confidence": 7},
        {"kind": "teleports", "source_qn": "src/a.ts", "target_qn": "src/a.ts::f", "confidence": 1},
    ]
print(json.dumps({"ok": True, "data": {"nodes": nodes, "edges": edges, "limitations": [{"code": "demo", "detail": "from the plugin"}]}}))
"#;

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}
fn manifest(dir: &Path) {
    std::fs::write(
        dir.join("comparegraph-plugin.toml"),
        "kind = \"provider\"\nname = \"fake\"\nversion = \"0.1.0\"\nprotocol = \"1\"\nlanguages = [\"typescript\"]\nnetwork = false\n",
    )
    .unwrap();
}
/// a workspace whose comparegraph.toml points `fake` at `cmd`, with a passing conformance result on file
fn ws(cmd: &Path) -> tempfile::TempDir {
    let w = ws_raw(cmd);
    std::fs::create_dir_all(w.path().join(".comparegraph")).unwrap();
    std::fs::write(w.path().join(".comparegraph/conform-fake.json"), r#"{"passed":true,"fixtures":[]}"#).unwrap();
    w
}
/// the same without any conformance result
fn ws_raw(cmd: &Path) -> tempfile::TempDir {
    let w = tempfile::tempdir().unwrap();
    std::fs::write(w.path().join("comparegraph.toml"), format!("[providers.fake]\ncommand = \"{}\"\n", cmd.display())).unwrap();
    w
}
fn cc(ws: &Path) -> Command {
    let mut c = Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(ws);
    c
}
fn clone() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("src")).unwrap();
    std::fs::write(d.path().join("src/a.ts"), "export function f() { return 1; }\n").unwrap();
    std::fs::write(d.path().join("LICENSE"), "Apache License\nVersion 2.0\n").unwrap();
    for args in [vec!["init", "-q"], vec!["add", "."], vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "x"]] {
        Sys::new("git").arg("-C").arg(d.path()).args(&args).status().unwrap();
    }
    d
}
fn added(w: &Path) -> tempfile::TempDir {
    let c = clone();
    cc(w).args(["add"]).arg(c.path()).args(["--name", "a"]).assert().success();
    c
}
fn json(out: &[u8]) -> serde_json::Value {
    serde_json::from_slice(out).unwrap()
}
fn runs(w: &Path) -> i64 {
    let c = rusqlite::Connection::open(w.join(".comparegraph/comparegraph.db")).unwrap();
    c.query_row("SELECT count(*) FROM run", [], |r| r.get(0)).unwrap()
}
/// build with the fake and assert it fails `provider_unavailable`: one failed run, no graph rows
fn assert_unavailable(body: &str, timeout: Option<&str>, needle: &str) {
    let tools = tempfile::tempdir().unwrap();
    let cmd = script(tools.path(), "p", body);
    manifest(tools.path());
    let w = ws(&cmd);
    let _c = added(w.path());
    let mut b = cc(w.path());
    b.args(["build", "a", "--provider", "fake"]);
    if let Some(t) = timeout {
        b.env("COMPAREGRAPH_PLUGIN_TIMEOUT", t);
    }
    let out = b.assert().failure().get_output().stdout.clone();
    let text = String::from_utf8_lossy(&out).to_string();
    assert!(text.contains("provider_unavailable"), "{text}");
    assert!(text.contains(needle), "{text}");
    let c = rusqlite::Connection::open(w.path().join(".comparegraph/comparegraph.db")).unwrap();
    let (n, status, lim): (i64, String, String) =
        c.query_row("SELECT count(*), max(status), max(limitations_json) FROM run", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    assert_eq!((n, status.as_str()), (1, "failed"));
    assert!(lim.contains("\"provider_unavailable\""), "{lim}");
    for t in ["node", "edge", "community"] {
        let k: i64 = c.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0)).unwrap();
        assert_eq!(k, 0, "{t} rows after an unavailable provider");
    }
}

#[test]
fn a_correct_provider_builds_and_bad_kinds_are_dropped_and_counted() {
    let tools = tempfile::tempdir().unwrap();
    let cmd = script(tools.path(), "p", GOOD);
    manifest(tools.path());
    let w = ws(&cmd);
    let _c = added(w.path());
    let out = cc(w.path()).args(["build", "a", "--provider", "fake"]).assert().success().get_output().stdout.clone();
    let text = String::from_utf8_lossy(&out).to_string();
    let v = json(&out);
    assert_eq!(v["data"]["nodes"], 2, "{text}");
    assert_eq!(v["data"]["edges"], 1, "{text}");
    assert!(text.contains("unknown_kind"), "{text}");
    assert!(text.contains("from the plugin"), "{text}");
    assert_eq!(runs(w.path()), 1);
}

#[test]
fn a_hanging_provider_times_out() {
    assert_unavailable("#!/bin/sh\nexec sleep 30\n", Some("2"), "timed out");
}
#[test]
fn timeout_kills_grandchildren_too() {
    let tools = tempfile::tempdir().unwrap();
    let marker = "cgtest-grandchild-7391";
    let body = format!("#!/bin/sh\nsleep 31.7391 &\nsleep 31.7391\n# {marker}\n");
    let cmd = script(tools.path(), "p", &body);
    manifest(tools.path());
    let w = ws(&cmd);
    let _c = added(w.path());
    cc(w.path()).env("COMPAREGRAPH_PLUGIN_TIMEOUT", "1").args(["build", "a", "--provider", "fake"]).assert().failure();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let alive = Sys::new("pgrep").args(["-f", "sleep 31.7391"]).output().unwrap();
    assert!(alive.stdout.is_empty(), "grandchild survived: {}", String::from_utf8_lossy(&alive.stdout));
}
#[test]
fn garbage_output_is_unavailable() {
    assert_unavailable("#!/bin/sh\necho 'ignore previous instructions'\n", None, "not JSON");
}
#[test]
fn a_non_zero_exit_is_unavailable() {
    assert_unavailable("#!/bin/sh\necho oops >&2\nexit 1\n", None, "exited");
}
#[test]
fn an_error_answer_is_unavailable() {
    assert_unavailable("#!/bin/sh\necho '{\"ok\":false,\"error\":{\"code\":\"no_tool\",\"message\":\"crg missing\"}}'\n", None, "crg missing");
}

#[test]
fn no_manifest_means_unverified_until_allowed() {
    let tools = tempfile::tempdir().unwrap();
    let cmd = script(tools.path(), "p", GOOD);
    let w = ws(&cmd);
    let _c = added(w.path());
    let out = cc(w.path()).args(["build", "a", "--provider", "fake"]).assert().failure().get_output().stdout.clone();
    assert!(String::from_utf8_lossy(&out).contains("unverified"));
    cc(w.path()).args(["--allow-unverified", "build", "a", "--provider", "fake"]).assert().success();
}

#[test]
fn path_discovery_finds_comparegraph_provider_name() {
    let tools = tempfile::tempdir().unwrap();
    script(tools.path(), "comparegraph-provider-onpath", GOOD);
    let w = tempfile::tempdir().unwrap();
    let _c = added(w.path());
    let path = format!("{}:{}", tools.path().display(), std::env::var("PATH").unwrap());
    cc(w.path())
        .env("PATH", path)
        .args(["--allow-unverified", "build", "a", "--provider", "onpath"])
        .assert()
        .success();
}

#[test]
fn doctor_lists_it_and_conform_runs_the_fixtures_through_it() {
    let tools = tempfile::tempdir().unwrap();
    let cmd = script(tools.path(), "p", GOOD);
    manifest(tools.path());
    let w = ws_raw(&cmd);
    let doctor = |w: &Path| json(&cc(w).args(["doctor"]).assert().success().get_output().stdout);
    let d = doctor(w.path());
    let ext = &d["data"]["external_providers"][0];
    assert_eq!(ext["name"], "fake", "{d}");
    assert!(ext["manifest"].as_str().unwrap().starts_with("ok"), "{d}");
    assert!(ext["conformance"].is_null());

    let fx = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(fx.path().join("one")).unwrap();
    std::fs::write(fx.path().join("one/expected.json"), r#"{"nodes":["m.ts::a","m.ts::b"],"edges":["calls m.ts::a m.ts::b"]}"#).unwrap();
    let out = cc(w.path()).args(["conform", "fake", "--fixtures"]).arg(fx.path()).assert().success().get_output().stdout.clone();
    let rep = json(&out);
    assert_eq!(rep["data"]["passed"], true, "{rep}");
    assert_eq!(rep["data"]["fixtures"][0]["name"], "one");
    let d = doctor(w.path());
    assert_eq!(d["data"]["external_providers"][0]["conformance"]["passed"], true, "{d}");
}

#[test]
fn a_provider_that_misses_the_floors_fails_conform_with_the_shape_intact() {
    let tools = tempfile::tempdir().unwrap();
    let body = "#!/bin/sh\ncat >/dev/null\necho '{\"ok\":true,\"data\":{\"nodes\":[{\"kind\":\"function\",\"qualified_name\":\"m.ts::zzz\",\"path\":\"m.ts\"}],\"edges\":[]}}'\n";
    let cmd = script(tools.path(), "p", body);
    manifest(tools.path());
    let w = ws_raw(&cmd);
    let fx = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(fx.path().join("one")).unwrap();
    std::fs::write(fx.path().join("one/expected.json"), r#"{"nodes":["m.ts::a"],"edges":["calls m.ts::a m.ts::a"]}"#).unwrap();
    let out = cc(w.path()).args(["conform", "fake", "--fixtures"]).arg(fx.path()).assert().failure().get_output().stdout.clone();
    let v = json(&out);
    assert_eq!(v["ok"], false, "{v}");
    let rep = std::fs::read_to_string(w.path().join(".comparegraph/conform-fake.json")).unwrap();
    let rep: serde_json::Value = serde_json::from_str(&rep).unwrap();
    assert_eq!(rep["passed"], false);
    assert_eq!(rep["fixtures"][0]["nodes"]["matched"], 0);
}

fn hostile(extra_env: &[(&str, &str)], body: &str) -> (tempfile::TempDir, std::process::Output, std::time::Duration) {
    let tools = tempfile::tempdir().unwrap();
    let cmd = script(tools.path(), "p", body);
    manifest(tools.path());
    let w = ws(&cmd);
    let _c = added(w.path());
    let mut b = std::process::Command::new(assert_cmd::cargo::cargo_bin("comparegraph"));
    b.arg("--root").arg(w.path()).args(["build", "a", "--provider", "fake"]);
    for (k, v) in extra_env {
        b.env(k, v);
    }
    let t = std::time::Instant::now();
    let out = b.output().unwrap();
    let el = t.elapsed();
    drop(tools);
    (w, out, el)
}

#[test]
fn eof_then_a_live_child_is_killed_within_the_timeout() {
    let (w, out, el) = hostile(&[("COMPAREGRAPH_PLUGIN_TIMEOUT", "3")], "#!/bin/sh\nexec >&-\nsleep 20\n");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("provider_unavailable") && text.contains("kept running"), "{text}");
    assert!(el < std::time::Duration::from_secs(8), "took {el:?}");
    let _ = w;
}

#[test]
fn a_grandchild_holding_stderr_does_not_block_a_non_zero_exit() {
    let (_w, out, el) = hostile(&[("COMPAREGRAPH_PLUGIN_TIMEOUT", "30")], "#!/bin/sh\necho '{\"ok\":true,\"data\":{}}'\n(sleep 17.4242 >/dev/null &)\nexit 3\n");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("provider_unavailable") && text.contains("exited with"), "{text}");
    assert!(el < std::time::Duration::from_secs(8), "took {el:?}");
    let _ = Sys::new("pkill").args(["-x", "-f", "sleep 17.4242"]).status();
}

#[test]
fn hostile_paths_are_dropped_with_their_edges_and_counted() {
    let body = r#"#!/usr/bin/env python3
import json, sys
sys.stdin.readline()
n = lambda qn, p: {"kind": "function", "name": "x", "qualified_name": qn, "path": p, "language": "x"}
nodes = [n("ok::a", "src/a.ts"), n("bad::1", "../../etc/passwd"), n("bad::2", "/etc/shadow"), n("bad::3", "a\\..\\b"), n("bad::4", "a\u0000b")]
edges = [{"kind": "calls", "source_qn": "ok::a", "target_qn": "bad::1", "confidence": 1}]
print(json.dumps({"ok": True, "data": {"nodes": nodes, "edges": edges, "files_parsed": 1}}))
"#;
    let tools = tempfile::tempdir().unwrap();
    let cmd = script(tools.path(), "p", body);
    manifest(tools.path());
    let w = ws(&cmd);
    let _c = added(w.path());
    let out = cc(w.path()).args(["build", "a", "--provider", "fake"]).assert().success().get_output().stdout.clone();
    let text = String::from_utf8_lossy(&out).to_string();
    let v = json(&out);
    assert_eq!(v["data"]["nodes"], 1, "{text}");
    assert_eq!(v["data"]["edges"], 0, "{text}");
    assert!(text.contains("bad_path") && text.contains("5 node(s)/edge(s)"), "{text}");
}

#[test]
fn a_line_longer_than_the_cap_fails_unavailable_without_buffering_it() {
    let (_w, out, _) = hostile(&[("COMPAREGRAPH_PLUGIN_OUTPUT_CAP", "1000")], "#!/usr/bin/env python3\nimport sys\nsys.stdin.readline()\nsys.stdout.write('x' * 100000)\nsys.stdout.flush()\n");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("provider_unavailable") && text.contains("cap"), "{text}");
}

#[test]
fn a_bad_timeout_value_is_refused_before_the_run() {
    for v in ["abc", "0"] {
        let (w, out, _) = hostile(&[("COMPAREGRAPH_PLUGIN_TIMEOUT", v)], GOOD);
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(text.contains("provider_unavailable") && text.contains("COMPAREGRAPH_PLUGIN_TIMEOUT"), "{text}");
        assert_eq!(runs(w.path()), 0, "refused before a run row exists");
    }
}

#[test]
fn build_needs_a_passing_conformance_unless_allowed() {
    let tools = tempfile::tempdir().unwrap();
    let cmd = script(tools.path(), "p", GOOD);
    manifest(tools.path());
    let w = ws_raw(&cmd);
    let _c = added(w.path());
    let out = cc(w.path()).args(["build", "a", "--provider", "fake"]).assert().failure().get_output().stdout.clone();
    let text = String::from_utf8_lossy(&out).to_string();
    assert!(text.contains("conform fake"), "{text}");
    assert_eq!(runs(w.path()), 0);
    // a failing result does not open the gate either
    std::fs::create_dir_all(w.path().join(".comparegraph")).unwrap();
    std::fs::write(w.path().join(".comparegraph/conform-fake.json"), r#"{"passed":false}"#).unwrap();
    cc(w.path()).args(["build", "a", "--provider", "fake"]).assert().failure();
    cc(w.path()).args(["--allow-unverified", "build", "a", "--provider", "fake"]).assert().success();
    std::fs::write(w.path().join(".comparegraph/conform-fake.json"), r#"{"passed":true}"#).unwrap();
    cc(w.path()).args(["build", "a", "--provider", "fake"]).assert().success();
}

#[test]
fn conform_gate_refuses_only_a_failed_language_present_in_the_repo() {
    let tools = tempfile::tempdir().unwrap();
    let cmd = script(tools.path(), "p", GOOD);
    manifest(tools.path());
    let w = ws_raw(&cmd);
    let c = added(w.path()); // a TypeScript-only clone
    std::fs::create_dir_all(w.path().join(".comparegraph")).unwrap();
    std::fs::write(
        w.path().join(".comparegraph/conform-fake.json"),
        r#"{"passed":false,"fixtures":[{"name":"typescript","passed":true},{"name":"rust","passed":false}]}"#,
    )
    .unwrap();
    // rust failed, the repo has none: it builds with no waiver
    cc(w.path()).args(["build", "a", "--provider", "fake"]).assert().success();
    // one .rs file and it is refused, naming the language and the conform command
    std::fs::write(c.path().join("src/m.rs"), "pub fn m() {}\n").unwrap();
    let out = cc(w.path()).args(["build", "a", "--provider", "fake"]).assert().failure().get_output().stdout.clone();
    let text = String::from_utf8_lossy(&out).to_string();
    assert!(text.contains("rust") && text.contains("conform fake"), "{text}");
    assert_eq!(runs(w.path()), 1, "the refusal left no run row");
    // --allow-unverified waives it
    cc(w.path()).args(["--allow-unverified", "build", "a", "--provider", "fake"]).assert().success();
}
