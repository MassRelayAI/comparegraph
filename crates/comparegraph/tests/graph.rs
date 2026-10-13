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
fn tests_for_honours_max_nodes_and_says_so() {
    let mut t = String::from("from src.wide import target\n");
    for i in 0..30 {
        t.push_str(&format!("\ndef test_c{i}():\n    target()\n"));
    }
    let (ws, _c) = ws_with(&[("src/wide.py", "def target():\n    pass\n"), ("tests/test_wide.py", &t)]);
    let v = json(cc(ws.path()).args(["tests-for", "r", "src/wide.py::target", "--max-nodes", "5"]).assert().success());
    assert_eq!(v["data"]["truncated"], true);
    assert!(codes(&v).contains(&"truncated".to_string()));
    let full = json(cc(ws.path()).args(["tests-for", "r", "src/wide.py::target"]).assert().success());
    assert_eq!(full["data"]["truncated"], false);
    assert!(!codes(&full).contains(&"truncated".to_string()));
    cc(ws.path()).args(["tests-for", "r", "src/wide.py::target", "--max-nodes", "9999"]).assert().failure();
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
    let all = qns(&v);
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

// ---- bounded-adjacency identity: every query on one generated fixture must equal the answer recorded from the
// whole-run-in-memory implementation (f3259a6). Regenerate only with GOLDEN_UPDATE=1 and a reason in the commit. ----
fn golden_fixture() -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = vec![];
    for m in 0..6 {
        let mut s = String::new();
        if m > 0 {
            s.push_str(&format!("from src.m{} import f{}_0, f{}_1\n\n", m - 1, m - 1, m - 1));
        }
        for i in 0..12 {
            s.push_str(&format!("def f{m}_{i}():\n"));
            if i + 1 < 12 {
                s.push_str(&format!("    f{m}_{}()\n", i + 1));
            }
            if i % 3 == 0 && m > 0 {
                s.push_str(&format!("    f{}_0()\n", m - 1));
            }
            if i % 4 == 1 && m > 0 {
                s.push_str(&format!("    f{}_1()\n", m - 1));
            }
            // a name that exists in two other modules: a guessed (0.5) edge
            if i == 5 {
                s.push_str("    dup_name()\n");
            }
            if i == 11 {
                s.push_str(&format!("    f{m}_3()\n    f{m}_11()\n"));
            }
            if i < 11 {
                s.push_str("    pass\n");
            }
            s.push('\n');
        }
        s.push_str(&format!("class K{m}:\n    def meth(self):\n        f{m}_0()\n\nclass Sub{m}(K{m}):\n    def other(self):\n        self.meth()\n\n"));
        s.push_str(&format!("def _private{m}():\n    pass\n\ndef orphan{m}():\n    pass\n"));
        files.push((format!("src/m{m}.py"), s));
    }
    files.push(("src/dups_a.py".into(), "def dup_name():\n    pass\n".into()));
    files.push(("src/dups_b.py".into(), "def dup_name():\n    pass\n".into()));
    files.push((
        "src/server.py".into(),
        "from src.m3 import f3_0\n\ndef handle_request():\n    f3_0()\n\ndef main():\n    handle_request()\n\nif __name__ == \"__main__\":\n    main()\n".into(),
    ));
    let mut t = String::from("from src.m1 import f1_0\nfrom src.m4 import f4_2\n\n");
    for i in 0..4 {
        t.push_str(&format!("def test_a{i}():\n    f1_{i}()\n    f4_2()\n\n"));
    }
    files.push(("tests/test_a.py".into(), t));
    files.push((
        "tests/test_b.py".into(),
        "from src.m2 import f2_0\n\ndef test_b():\n    f2_0()\n\ndef helper_in_test():\n    f2_0()\n".into(),
    ));
    files
}

fn golden_queries() -> Vec<Vec<String>> {
    let q = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let mut v = vec![];
    for sym in ["src/m3.py::f3_5", "src/m0.py::f0_0", "src/m5.py::f5_11", "src/m2.py::K2", "src/m4.py::other", "src/dups_a.py::dup_name", "src/server.py::main"] {
        for dir in ["callers", "callees", "both"] {
            for (depth, max, conf) in [("1", "200", "0.5"), ("3", "200", "0.5"), ("10", "2000", "0.5"), ("10", "7", "0.5"), ("4", "200", "1.0")] {
                v.push(q(&["traverse", "r", sym, "--direction", dir, "--depth", depth, "--max-nodes", max, "--min-confidence", conf]));
            }
        }
        v.push(q(&["traverse", "r", sym, "--kinds", "calls", "--depth", "6"]));
        v.push(q(&["traverse", "r", sym, "--kinds", "imports,tests,calls", "--depth", "6"]));
        v.push(q(&["impact", "r", sym]));
        v.push(q(&["impact", "r", sym, "--depth", "10", "--max-nodes", "9"]));
        v.push(q(&["impact", "r", sym, "--min-confidence", "1.0", "--depth", "10"]));
        v.push(q(&["tests-for", "r", sym]));
        v.push(q(&["tests-for", "r", sym, "--max-nodes", "3"]));
        v.push(q(&["tests-for", "r", sym, "--min-confidence", "1.0"]));
    }
    v.push(q(&["impact", "r", "--file", "src/m1.py"]));
    v.push(q(&["impact", "r", "--file", "src/m1.py", "--file", "src/m4.py", "--depth", "10", "--max-nodes", "40"]));
    v.push(q(&["impact", "r", "src/m2.py::f2_0", "--file", "src/dups_b.py"]));
    for cmd in ["entry-points", "dead-code"] {
        for conf in ["0.5", "1.0"] {
            v.push(q(&[cmd, "r", "--min-confidence", conf]));
            v.push(q(&[cmd, "r", "--limit", "7", "--min-confidence", conf]));
            for cur in ["7", "14", "21", "999"] {
                v.push(q(&[cmd, "r", "--limit", "7", "--cursor", cur, "--min-confidence", conf]));
            }
        }
    }
    v
}

#[test]
fn bounded_adjacency_answers_equal_the_recorded_whole_run_answers() {
    let files = golden_fixture();
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let (ws, _c) = ws_with(&refs);
    let mut got = serde_json::Map::new();
    for args in golden_queries() {
        let out = cc(ws.path()).args(&args).assert().get_output().clone();
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        let mut rec = serde_json::Map::new();
        rec.insert("ok".into(), v["ok"].clone());
        rec.insert("data".into(), v["data"].clone());
        rec.insert("error".into(), v["error"].clone());
        rec.insert("confidence".into(), v["evidence"]["confidence"].clone());
        rec.insert("limitations".into(), v["evidence"]["limitations"].clone());
        rec.insert("page".into(), v["page"].clone());
        got.insert(args.join(" "), Value::Object(rec));
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/graph_queries.json");
    if std::env::var("GOLDEN_UPDATE").is_ok() {
        std::fs::write(&path, serde_json::to_string_pretty(&got).unwrap() + "\n").unwrap();
        return;
    }
    let want: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let want = want.as_object().unwrap();
    assert_eq!(want.len(), got.len(), "query count");
    for (k, w) in want {
        assert_eq!(&got[k], w, "answer changed for: {k}");
    }
}

#[test]
fn dead_code_ignores_callers_that_run_at_load_time_in_a_test_file() {
    // a test file's own scope (kind `test`) and a `describe` body run at load time: calling a method from them does not make
    // it live; an `it:` case does. The provider emits none of these nodes for Python, so the run gets them by direct insert.
    let lib = "class K:\n    def only_scope(self):\n        pass\n    def only_describe(self):\n        pass\n    def by_case(self):\n        pass\n";
    let (ws, _c) = ws_with(&[("src/lib.py", lib), ("tests/test_x.py", "def test_x():\n    pass\n")]);
    {
        let db = rusqlite::Connection::open(ws.path().join(".comparegraph/comparegraph.db")).unwrap();
        let (repo, run): (i64, i64) = db.query_row("SELECT repo_id, id FROM run ORDER BY id DESC LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        let add = |kind: &str, name: &str| -> i64 {
            db.execute(
                "INSERT INTO node(repo_id,run_id,kind,name,qualified_name,path,line_start,line_end,content_hash,language) VALUES (?1,?2,?3,?4,?5,'tests/x.test.ts',1,2,'h','typescript')",
                rusqlite::params![repo, run, kind, name, format!("tests/x.test.ts::{name}")],
            )
            .unwrap();
            db.last_insert_rowid()
        };
        let scope = add("test", "x.test.ts");
        let describe = add("function", "describe: x");
        let case = add("function", "it: works");
        for (src, qn) in [(scope, "only_scope"), (describe, "only_describe"), (case, "by_case")] {
            let dst: i64 = db
                .query_row("SELECT id FROM node WHERE run_id=?1 AND qualified_name=?2", rusqlite::params![run, format!("src/lib.py::{qn}")], |r| r.get(0))
                .unwrap();
            db.execute(
                "INSERT INTO edge(repo_id,run_id,kind,source_node_id,target_node_id,confidence) VALUES (?1,?2,'calls',?3,?4,1.0)",
                rusqlite::params![repo, run, src, dst],
            )
            .unwrap();
        }
    }
    let d = json(cc(ws.path()).args(["dead-code", "r"]).assert().success());
    let q = qns(&d);
    assert!(q.contains(&"src/lib.py::only_scope".to_string()), "{q:?}");
    assert!(q.contains(&"src/lib.py::only_describe".to_string()), "{q:?}");
    assert!(!q.contains(&"src/lib.py::by_case".to_string()), "{q:?}");
}

#[test]
fn a_hub_with_thousands_of_callees_is_cut_at_max_nodes_without_loading_them_all() {
    let mut app = String::from("def hub():\n");
    for i in 0..3000 {
        app.push_str(&format!("    leaf{i:04}()\n"));
    }
    for i in 0..3000 {
        app.push_str(&format!("\ndef leaf{i:04}():\n    pass\n"));
    }
    let (ws, _c) = ws_with(&[("src/hub.py", &app)]);
    let t = std::time::Instant::now();
    let v = json(
        cc(ws.path())
            .args(["traverse", "r", "src/hub.py::hub", "--direction", "callees", "--max-nodes", "10"])
            .assert()
            .success(),
    );
    assert!(t.elapsed().as_secs_f64() < 10.0, "took {:?}", t.elapsed());
    let want: Vec<String> = (0..10).map(|i| format!("src/hub.py::leaf{i:04}")).collect();
    assert_eq!(qns(&v), want, "the first ten in qualified-name order");
    assert_eq!(v["data"]["truncated"], true);
}

/// Walks every page with `--limit 1` and the returned cursor; each item appears exactly once.
fn walk(ws: &std::path::Path, verb_args: &[&str]) -> (Value, Vec<String>) {
    let mut seen = vec![];
    let mut cursor: Option<String> = None;
    let first = loop {
        let mut a: Vec<String> = verb_args.iter().map(|s| s.to_string()).collect();
        a.extend(["--limit".into(), "1".into()]);
        if let Some(c) = &cursor {
            a.extend(["--cursor".into(), c.clone()]);
        }
        let v = json(cc(ws).args(&a).assert().success());
        let items = v["data"]["items"].as_array().unwrap();
        assert!(items.len() <= 1, "{v}");
        seen.extend(qns(&v));
        match v["page"]["next"].as_str() {
            Some(n) => cursor = Some(n.to_string()),
            None => break v,
        }
    };
    (first, seen)
}

#[test]
fn traverse_impact_and_tests_for_answer_summary_first_and_page_every_item_once() {
    let mut app = String::from("def target():\n    pass\n");
    for i in 0..7 {
        app.push_str(&format!("\ndef caller{i}():\n    target()\n"));
    }
    let mut t = String::from("from src.wide import target\n");
    for i in 0..4 {
        t.push_str(&format!("\ndef test_c{i}():\n    target()\n"));
    }
    let (ws, _c) = ws_with(&[("src/wide.py", &app), ("tests/test_wide.py", &t)]);
    for args in [
        vec!["traverse", "r", "src/wide.py::target", "--direction", "callers"],
        vec!["impact", "r", "src/wide.py::target"],
        vec!["tests-for", "r", "src/wide.py::target"],
    ] {
        let all = json(cc(ws.path()).args(&args).assert().success());
        let total = all["data"]["summary"]["total"].as_u64().unwrap();
        assert!(total >= 4, "{args:?}: {all}");
        assert_eq!(all["data"]["total"], total);
        assert_eq!(all["data"]["summary"]["truncated"], false);
        assert!(all["data"]["summary"]["by_hop"].is_array());
        assert!(all["data"]["summary"]["by_kind"].is_object());
        assert!(all["data"]["summary"]["max_nodes"].as_u64().is_some());
        assert_eq!(all["page"]["total"], total);
        let full = qns(&all);
        assert_eq!(full.len() as u64, total, "default page holds them all");
        let (last, walked) = walk(ws.path(), &args);
        assert_eq!(walked.len() as u64, total, "{args:?}");
        let mut a = walked.clone();
        a.sort();
        a.dedup();
        assert_eq!(a.len(), walked.len(), "each item once: {walked:?}");
        let mut b = full.clone();
        b.sort();
        assert_eq!(a, b);
        assert!(last["page"]["next"].is_null());
        // ordered by path_confidence desc, hop asc, name
        assert_eq!(walked, full);
    }
    // the first page of a limit-1 call says there is more
    let first = json(cc(ws.path()).args(["impact", "r", "src/wide.py::target", "--limit", "1"]).assert().success());
    assert_eq!(first["page"]["next"], "1");
    cc(ws.path()).args(["impact", "r", "src/wide.py::target", "--cursor", "x"]).assert().failure();
}

#[test]
fn impact_pages_and_still_declares_truncation() {
    let mut app = String::from("def target():\n    pass\n");
    for i in 0..30 {
        app.push_str(&format!("\ndef caller{i}():\n    target()\n"));
    }
    let (ws, _c) = ws_with(&[("src/wide.py", &app)]);
    let v = json(cc(ws.path()).args(["impact", "r", "src/wide.py::target", "--max-nodes", "5", "--limit", "2"]).assert().success());
    assert_eq!(v["data"]["summary"]["truncated"], true);
    assert_eq!(v["data"]["summary"]["max_nodes"], 5);
    assert_eq!(v["data"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(v["page"]["total"], 5);
    assert!(codes(&v).contains(&"truncated".to_string()));
}
