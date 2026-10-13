//! The code-review-graph adapter (Plan 6 C2): providers/crg/comparegraph-provider-crg driven through the plugin host with a FAKE
//! `code-review-graph` (a script that writes a crg-shaped SQLite), so no test needs the real engine.
use assert_cmd::Command;
use std::path::{Path, PathBuf};
use std::process::Command as Sys;

fn provider() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../providers/crg/comparegraph-provider-crg").canonicalize().unwrap()
}
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}
/// a fake `code-review-graph`: `--version`, and `build --repo R` writes R/.code-review-graph/graph.db with crg's tables
const FAKE: &str = r#"#!/usr/bin/env python3
import os, sqlite3, sys
if "--version" in sys.argv:
    print("code-review-graph 2.3.9"); sys.exit(0)
if os.environ.get("FAKE_CRG_FAIL"):
    print("boom", file=sys.stderr); sys.exit(3)
repo = sys.argv[sys.argv.index("--repo") + 1]
os.makedirs(os.path.join(repo, ".code-review-graph"), exist_ok=True)
db = sqlite3.connect(os.path.join(repo, ".code-review-graph", "graph.db"))
db.executescript("""
create table nodes (id integer primary key autoincrement, kind text, name text, qualified_name text unique, file_path text,
  line_start integer, line_end integer, language text, parent_name text, is_test integer default 0, file_hash text);
create table edges (id integer primary key autoincrement, kind text, source_qualified text, target_qualified text, file_path text, line integer default 0);
""")
P = lambda f: os.path.join(repo, f)
def node(kind, name, qn, f, a, b, parent=None, test=0):
    db.execute("insert into nodes(kind,name,qualified_name,file_path,line_start,line_end,language,parent_name,is_test,file_hash) values (?,?,?,?,?,?,?,?,?,?)",
               (kind, name, qn, P(f), a, b, "python", parent, test, "h-" + f))
def edge(kind, s, t):
    db.execute("insert into edges(kind,source_qualified,target_qualified,file_path) values (?,?,?,?)", (kind, s, t, ""))
for f, last in (("a.py", 8), ("b.py", 2), ("c.py", 2)):
    node("File", P(f), P(f), f, 1, last)
node("File", P("test_a.py"), P("test_a.py"), "test_a.py", 1, 3, test=1)
node("Function", "fa", P("a.py") + "::fa", "a.py", 2, 5)
node("Function", "fa2", P("a.py") + "::fa2", "a.py", 6, 8)
node("Class", "K", P("a.py") + "::K", "a.py", 9, 12)
node("Function", "m", P("a.py") + "::K.m", "a.py", 10, 11, parent="K")
node("Function", "fb", P("b.py") + "::fb", "b.py", 1, 2)
node("Function", "fc", P("c.py") + "::fc", "c.py", 1, 2)
node("Test", "test_fa", P("test_a.py") + "::test_fa", "test_a.py", 1, 3, test=1)
edge("IMPORTS_FROM", P("a.py"), P("b.py"))
edge("CALLS", P("a.py") + "::fa", P("a.py") + "::fa2")        # same file                  -> 1.0
edge("CALLS", P("a.py") + "::fa", P("b.py") + "::fb")         # a.py imports b.py          -> 1.0
edge("CALLS", P("a.py") + "::fa", P("c.py") + "::fc")         # nothing backs it            -> 0.5
edge("CALLS", P("a.py") + "::fa", "print")                    # not an indexed node         -> dropped
edge("REFERENCES", P("a.py") + "::fa", P("b.py") + "::fb")    # no counterpart              -> counted, not emitted
edge("TESTED_BY", P("a.py") + "::fa", P("test_a.py") + "::test_fa")
db.commit()
print("Full build: 5 files, 8 nodes, 7 edges")
"#;

fn setup() -> (tempfile::TempDir, tempfile::TempDir, PathBuf) {
    let tools = tempfile::tempdir().unwrap();
    let fake = script(tools.path(), "code-review-graph", FAKE);
    let ws = tempfile::tempdir().unwrap();
    std::fs::write(ws.path().join("comparegraph.toml"), format!("[providers.crg]\ncommand = \"{}\"\n", provider().display())).unwrap();
    (tools, ws, fake)
}
fn cc(ws: &Path, fake: &Path) -> Command {
    let mut c = Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(ws).arg("--allow-unverified").env("COMPAREGRAPH_CRG_BIN", fake);
    c
}
fn clone() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let w = |f: &str, s: &str| std::fs::write(d.path().join(f), s).unwrap();
    w("a.py", "from b import fb\n\ndef fa():\n    fb()\n    fa2()\n\ndef fa2():\n    pass\n");
    w("b.py", "def fb():\n    pass\n");
    w("c.py", "def fc():\n    pass\n");
    w("LICENSE", "Apache License\nVersion 2.0\n");
    for args in [vec!["init", "-q"], vec!["add", "."], vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "x"]] {
        Sys::new("git").arg("-C").arg(d.path()).args(&args).status().unwrap();
    }
    d
}
fn added(ws: &Path, fake: &Path) -> tempfile::TempDir {
    let c = clone();
    cc(ws, fake).args(["add"]).arg(c.path()).args(["--name", "a"]).assert().success();
    c
}
fn db(ws: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open(ws.join(".comparegraph/comparegraph.db")).unwrap()
}
fn json(out: &[u8]) -> serde_json::Value {
    serde_json::from_slice(out).unwrap()
}

#[test]
fn the_fake_crg_yields_mapped_kinds_and_the_confidence_rule() {
    let (_t, ws, fake) = setup();
    let _c = added(ws.path(), &fake);
    let out = cc(ws.path(), &fake).args(["build", "a", "--provider", "crg"]).assert().success().get_output().stdout.clone();
    let text = String::from_utf8_lossy(&out).to_string();
    let c = db(ws.path());
    let kinds: Vec<(String, String)> = c
        .prepare("SELECT qualified_name, kind FROM node ORDER BY qualified_name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(|x| x.unwrap())
        .collect();
    let k = |qn: &str| kinds.iter().find(|(q, _)| q == qn).map(|(_, k)| k.as_str()).unwrap_or("MISSING");
    assert_eq!(k("a.py"), "file");
    assert_eq!(k("test_a.py"), "test", "a crg test File is our test kind");
    assert_eq!(k("a.py::fa"), "function");
    assert_eq!(k("a.py::K"), "class");
    assert_eq!(k("a.py::m"), "method", "a Function with a parent is a method");
    assert_eq!(k("test_a.py::test_fa"), "function", "a crg Test is a function");
    let edge = |kind: &str, s: &str, t: &str| -> Option<f64> {
        c.query_row(
            "SELECT e.confidence FROM edge e JOIN node a ON a.id=e.source_node_id JOIN node b ON b.id=e.target_node_id WHERE e.kind=?1 AND a.qualified_name=?2 AND b.qualified_name=?3",
            [kind, s, t],
            |r| r.get(0),
        )
        .ok()
    };
    assert_eq!(edge("calls", "a.py::fa", "a.py::fa2"), Some(1.0), "same-file definition");
    assert_eq!(edge("calls", "a.py::fa", "b.py::fb"), Some(1.0), "the caller's file imports the callee's");
    assert_eq!(edge("calls", "a.py::fa", "c.py::fc"), Some(0.5), "crg resolved it, nothing here backs it");
    assert_eq!(edge("imports", "a.py", "b.py"), Some(1.0));
    assert_eq!(edge("contains", "a.py", "a.py::m"), Some(1.0), "containment is file to symbol");
    assert_eq!(edge("tests", "test_a.py::test_fa", "a.py::fa"), Some(0.5), "TESTED_BY reversed; the test file does not import a.py");
    let n: i64 = c.query_row("SELECT count(*) FROM edge e JOIN node b ON b.id=e.target_node_id WHERE b.qualified_name='print'", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 0);
    assert!(text.contains("crg_unresolved_dropped"), "{text}");
    assert!(text.contains("crg_name_only"), "{text}");
    assert!(text.contains("crg_unmapped_kinds") && text.contains("REFERENCES"), "{text}");
    assert_eq!(json(&out)["data"]["files_parsed"], 5, "crg's own count, not ours");
}

#[test]
fn a_failing_crg_is_a_failed_run_and_leaves_no_graph() {
    let (_t, ws, fake) = setup();
    let _c = added(ws.path(), &fake);
    let out = cc(ws.path(), &fake).env("FAKE_CRG_FAIL", "1").args(["build", "a", "--provider", "crg"]).assert().failure().get_output().stdout.clone();
    let text = String::from_utf8_lossy(&out).to_string();
    assert!(text.contains("provider_unavailable") && text.contains("exited 3"), "{text}");
    let n: i64 = db(ws.path()).query_row("SELECT count(*) FROM node", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn a_missing_crg_shows_uninstalled_in_doctor_with_the_install_hint() {
    let (_t, ws, _fake) = setup();
    let gone = ws.path().join("no-such-code-review-graph");
    let out = cc(ws.path(), &gone).arg("doctor").assert().success().get_output().stdout.clone();
    let v = json(&out);
    let ext = v["data"]["external_providers"].as_array().unwrap();
    let crg = ext.iter().find(|p| p["name"] == "crg").unwrap_or_else(|| panic!("crg not listed: {v}"));
    let why = crg["unavailable"].as_str().unwrap_or_else(|| panic!("no unavailable reason: {crg}"));
    assert!(why.contains("install code-review-graph"), "{why}");
    // and a build refuses before it starts a run
    let _c = added(ws.path(), &gone);
    let out = cc(ws.path(), &gone).args(["build", "a", "--provider", "crg"]).assert().failure().get_output().stdout.clone();
    let text = String::from_utf8_lossy(&out).to_string();
    assert!(text.contains("provider_unavailable") && text.contains("install code-review-graph"), "{text}");
    let n: i64 = db(ws.path()).query_row("SELECT count(*) FROM run", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 0, "no run row for a provider that is not installed");
}

#[test]
fn providers_writes_agreement_limitations_below_point_eight_and_none_above() {
    let (_t, ws, fake) = setup();
    let _c = added(ws.path(), &fake);
    let out = cc(ws.path(), &fake).args(["build", "a", "--providers", "reference,crg"]).assert().success().get_output().stdout.clone();
    let v = json(&out);
    let codes: Vec<String> = v["evidence"]["limitations"].as_array().unwrap().iter().map(|l| l["code"].as_str().unwrap().to_string()).collect();
    let rows = v["data"]["agreement"].as_array().unwrap();
    let j = |scope: &str, kind: &str| rows.iter().find(|r| r["scope"] == scope && r["kind"] == kind).map(|r| r["jaccard"].as_f64().unwrap());
    // files agree at 1.0, imports at 1.0 and functions at exactly 0.8 (4 of 5): at or above the floor writes nothing
    assert_eq!(j("nodes", "file"), Some(1.0), "{rows:?}");
    assert_eq!(j("edges", "imports"), Some(1.0), "{rows:?}");
    assert_eq!(j("nodes", "function"), Some(0.8), "{rows:?}");
    for ok in ["agreement_file", "agreement_imports", "agreement_function"] {
        assert!(!codes.iter().any(|c| c == ok), "{ok} must not be written at or above 0.8: {codes:?}");
    }
    // below 0.8: the fake crg has a class, a method and a test the reference does not draw, a third call, more containment
    for below in ["calls", "contains", "class"] {
        let scope = if below == "class" { "nodes" } else { "edges" };
        assert!(j(scope, below).unwrap() < 0.8, "{below} {rows:?}");
        assert!(codes.iter().any(|c| *c == format!("agreement_{below}")), "agreement_{below} missing: {codes:?}");
    }
    // the limitation sits on the later (crg) run only
    let c = db(ws.path());
    let lim = |provider: &str| -> String {
        c.query_row("SELECT limitations_json FROM run WHERE provider=?1", [provider], |r| r.get(0)).unwrap()
    };
    assert!(lim("crg").contains("agreement_calls"));
    assert!(!lim("reference").contains("agreement_"));
    // a pair that agrees everywhere writes nothing: crg against itself
    let ws2 = setup().1;
    let _c2 = added(ws2.path(), &fake);
    let out = cc(ws2.path(), &fake).args(["build", "a", "--providers", "crg,crg"]).assert().success().get_output().stdout.clone();
    let v = json(&out);
    assert!(v["data"]["agreement"].as_array().unwrap().iter().all(|r| r["jaccard"] == 1.0), "{v}");
    assert!(!String::from_utf8_lossy(&out).contains("agreement_"), "no agreement limitation at 1.0");
}

#[test]
fn an_unknown_provider_in_the_list_fails_cleanly_before_any_build() {
    let (_t, ws, fake) = setup();
    let _c = added(ws.path(), &fake);
    let out = cc(ws.path(), &fake).args(["build", "a", "--providers", "reference,nonesuch"]).assert().failure().get_output().stdout.clone();
    let text = String::from_utf8_lossy(&out).to_string();
    assert!(text.contains("not_found") && text.contains("nonesuch"), "{text}");
    let n: i64 = db(ws.path()).query_row("SELECT count(*) FROM run", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 0, "the valid name was not built either");
    // and --provider still works on its own
    cc(ws.path(), &fake).args(["build", "a", "--provider", "reference"]).assert().success();
}
