use serde_json::Value;
use std::process::Command;

fn clone_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let f = d.path().join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, c).unwrap();
    }
    std::fs::write(
        d.path().join("LICENSE"),
        "MIT License\n\nCopyright (c) 2026 S\n",
    )
    .unwrap();
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
        assert!(Command::new("git")
            .arg("-C")
            .arg(d.path())
            .args(&args)
            .status()
            .unwrap()
            .success());
    }
    d
}
fn cc(root: &std::path::Path) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(root);
    c
}
fn json(a: assert_cmd::assert::Assert) -> serde_json::Value {
    serde_json::from_slice(&a.get_output().stdout).unwrap()
}

fn ws_with(files: &[(&str, &str)]) -> (tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let c = clone_with(files);
    cc(ws.path())
        .args(["add", c.path().to_str().unwrap(), "--name", "r"])
        .assert()
        .success();
    cc(ws.path()).args(["build", "r"]).assert().success();
    (ws, c)
}
fn qns(v: &Value) -> Vec<String> {
    v["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["qualified_name"].as_str().unwrap().to_string())
        .collect()
}
fn codes(v: &Value) -> Vec<String> {
    v["evidence"]["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["code"].as_str().unwrap().to_string())
        .collect()
}
const APP: &str = "def main():\n    helper()\n\ndef helper():\n    leaf()\n\ndef leaf():\n    helper()\n    leaf()\n\ndef _unused():\n    pass\n\nclass Base:\n    def m(self):\n        pass\n";
const TEST_APP: &str = "from src.app import helper\n\ndef test_helper():\n    helper()\n";

#[test]
fn a_call_cycle_and_self_recursion_terminate_with_each_node_once() {
    let (ws, _c) = ws_with(&[("src/app.py", APP), ("tests/test_app.py", TEST_APP)]);
    let v = json(
        cc(ws.path())
            .args(["traverse", "r", "src/app.py::helper", "--direction", "callees", "--depth", "10"])
            .assert()
            .success(),
    );
    let q = qns(&v);
    assert_eq!(q, vec!["src/app.py::leaf".to_string()], "helper -> leaf -> helper(start) -> leaf(seen)");
    assert_eq!(v["data"]["items"][0]["hop"], 1);
    assert_eq!(v["data"]["items"][0]["via"][0], "src/app.py::helper");
    assert_eq!(v["data"]["items"][0]["path_confidence"], 1.0);
    assert_eq!(v["evidence"]["confidence"], "derived");
    let both = json(
        cc(ws.path())
            .args(["traverse", "r", "src/app.py::helper", "--depth", "10"])
            .assert()
            .success(),
    );
    let mut q = qns(&both);
    let n = q.len();
    q.sort();
    q.dedup();
    assert_eq!(n, q.len(), "no node twice");
    assert!(q.contains(&"src/app.py::main".to_string()));
}

#[test]
fn a_wide_impact_is_cut_at_max_nodes_and_says_so() {
    let mut app = String::from("def target():\n    pass\n");
    for i in 0..30 {
        app.push_str(&format!("\ndef caller{i}():\n    target()\n"));
    }
    let (ws, _c) = ws_with(&[("src/wide.py", &app)]);
    let v = json(
        cc(ws.path())
            .args(["impact", "r", "src/wide.py::target", "--max-nodes", "5"])
            .assert()
            .success(),
    );
    assert_eq!(v["data"]["total"], 5);
    assert_eq!(v["data"]["truncated"], true);
    assert!(codes(&v).contains(&"truncated".to_string()));
    let full = json(cc(ws.path()).args(["impact", "r", "src/wide.py::target"]).assert().success());
    assert_eq!(full["data"]["total"], 30);
    assert_eq!(full["data"]["truncated"], false);
    assert!(!codes(&full).contains(&"truncated".to_string()));
    cc(ws.path())
        .args(["impact", "r", "src/wide.py::target", "--max-nodes", "9999"])
        .assert()
        .failure();
}

#[test]
fn impact_by_file_follows_imports_and_lists_the_tests() {
    let (ws, _c) = ws_with(&[("src/app.py", APP), ("tests/test_app.py", TEST_APP)]);
    let v = json(
        cc(ws.path())
            .args(["impact", "r", "--file", "src/app.py"])
            .assert()
            .success(),
    );
    let all: Vec<String> = v["data"]["by_hop"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|h| h["nodes"].as_array().unwrap().clone())
        .map(|n| n["qualified_name"].as_str().unwrap().to_string())
        .collect();
    assert!(all.contains(&"tests/test_app.py".to_string()), "{all:?}");
    let tests: Vec<&str> = v["data"]["tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["qualified_name"].as_str().unwrap())
        .collect();
    assert!(tests.contains(&"tests/test_app.py::test_helper"), "{tests:?}");
    cc(ws.path()).args(["impact", "r"]).assert().failure();
    cc(ws.path()).args(["impact", "r", "--file", "nope.py"]).assert().failure();
}

#[test]
fn tests_for_reaches_through_two_call_hops() {
    let lib = "def leaf():\n    pass\n\ndef mid():\n    leaf()\n";
    let t = "from src.lib import mid\n\ndef test_mid():\n    mid()\n";
    let (ws, _c) = ws_with(&[("src/lib.py", lib), ("tests/test_lib.py", t)]);
    let v = json(cc(ws.path()).args(["tests-for", "r", "src/lib.py::leaf"]).assert().success());
    let hit = v["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["qualified_name"] == "tests/test_lib.py::test_mid")
        .unwrap_or_else(|| panic!("{v}"));
    assert_eq!(hit["hop"], 2);
    let direct = json(cc(ws.path()).args(["tests-for", "r", "src/lib.py::mid"]).assert().success());
    assert!(qns(&direct).contains(&"tests/test_lib.py::test_mid".to_string()));
}

#[test]
fn entry_points_and_dead_code_on_a_small_fixture() {
    let (ws, _c) = ws_with(&[("src/app.py", APP), ("tests/test_app.py", TEST_APP)]);
    let e = json(cc(ws.path()).args(["entry-points", "r"]).assert().success());
    let q = qns(&e);
    assert!(q.contains(&"src/app.py::main".to_string()), "{q:?}");
    assert!(!q.contains(&"src/app.py::helper".to_string()), "called");
    assert!(!q.contains(&"src/app.py::_unused".to_string()), "private");
    assert!(!q.iter().any(|n| n.starts_with("tests/")), "tests are not entry points");
    let d = json(cc(ws.path()).args(["dead-code", "r"]).assert().success());
    let q = qns(&d);
    assert!(q.contains(&"src/app.py::_unused".to_string()), "{q:?}");
    assert!(!q.contains(&"src/app.py::main".to_string()), "entry point");
    assert!(!q.contains(&"src/app.py::helper".to_string()), "called");
    assert!(!q.contains(&"src/app.py::leaf".to_string()), "called by helper; self-call ignored but helper calls it");
    assert!(!q.iter().any(|n| n.starts_with("tests/")));
    assert_eq!(d["evidence"]["confidence"], "unknown");
    assert!(codes(&d).contains(&"dynamic_dispatch_invisible".to_string()));
    assert_eq!(d["data"]["items"][0]["label"], "candidate");
}

#[test]
fn min_confidence_is_validated_and_unknown_symbols_are_not_found() {
    let (ws, _c) = ws_with(&[("src/app.py", APP)]);
    for bad in ["0.7", "0.0"] {
        cc(ws.path())
            .args(["traverse", "r", "src/app.py::main", "--min-confidence", bad])
            .assert()
            .failure();
    }
    let ok = json(
        cc(ws.path())
            .args(["traverse", "r", "src/app.py::main", "--min-confidence", "1.0", "--direction", "callees"])
            .assert()
            .success(),
    );
    assert_eq!(ok["verb"], "traverse");
    assert!(qns(&ok).contains(&"src/app.py::helper".to_string()));
    let nf = cc(ws.path()).args(["traverse", "r", "src/app.py::zzz"]).assert().failure();
    let v: Value = serde_json::from_slice(&nf.get_output().stdout).unwrap();
    assert_eq!(v["error"]["code"], "not_found");
    cc(ws.path())
        .args(["traverse", "r", "src/app.py::main", "--direction", "sideways"])
        .assert()
        .failure();
}

#[test]
fn name_guessed_hops_are_labelled_observed_with_a_count() {
    // two `run`s, no import: the call is a name-only guess (0.5)
    let a = "def go():\n    shared_name()\n";
    let b = "def shared_name():\n    pass\n";
    let (ws, _c) = ws_with(&[("a.py", a), ("b.py", b), ("c.py", b)]);
    let v = json(cc(ws.path()).args(["traverse", "r", "a.py::go", "--direction", "callees"]).assert().success());
    let it = &v["data"]["items"][0];
    assert!(it["qualified_name"].as_str().unwrap().ends_with("::shared_name"));
    assert_eq!(it["path_confidence"], 0.5);
    assert_eq!(v["evidence"]["confidence"], "observed");
    assert!(codes(&v).contains(&"name_guessed_hops".to_string()));
    let strict = json(
        cc(ws.path())
            .args(["traverse", "r", "a.py::go", "--direction", "callees", "--min-confidence", "1.0"])
            .assert()
            .success(),
    );
    assert!(qns(&strict).is_empty());
    assert_eq!(strict["evidence"]["confidence"], "derived");
}

#[test]
fn a_node_reached_by_a_full_path_and_a_shorter_guessed_one_reports_full_confidence() {
    // go -> shared_name is a name-only guess (0.5, two candidates); go -> mid -> shared_name is 1.0 all the way
    let a = "from b import mid\n\ndef go():\n    mid()\n    shared_name()\n";
    let b = "def mid():\n    shared_name()\n\ndef shared_name():\n    pass\n";
    let (ws, _c) = ws_with(&[("a.py", a), ("b.py", b), ("c.py", "def shared_name():\n    pass\n")]);
    let v = json(cc(ws.path()).args(["traverse", "r", "a.py::go", "--direction", "callees"]).assert().success());
    let items = v["data"]["items"].as_array().unwrap();
    let it = items.iter().find(|i| i["qualified_name"] == "b.py::shared_name").unwrap_or_else(|| panic!("{v}"));
    assert_eq!(it["path_confidence"], 1.0, "{v}");
}
