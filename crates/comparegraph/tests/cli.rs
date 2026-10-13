use assert_cmd::Command;
use predicates::prelude::*;
use std::process::Command as Sys;

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
        Sys::new("git")
            .arg("-C")
            .arg(d.path())
            .args(&args)
            .status()
            .unwrap();
    }
    d
}
fn cc(ws: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(ws);
    c
}

#[test]
fn add_build_check_export_through_the_binary() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone();
    cc(ws.path())
        .args(["add"])
        .arg(c.path())
        .args(["--name", "a", "--as-root"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"lane\":\"take\""))
        .stdout(predicate::str::contains("\"role\":\"root\""));
    cc(ws.path())
        .args(["build", "a"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"nodes\":3"));
    cc(ws.path())
        .args(["check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"problems\":[]"));
    let out = ws.path().join("g.json");
    cc(ws.path())
        .args(["export", "json", "--repo", "a", "--out"])
        .arg(&out)
        .assert()
        .success();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    // the file written by --out is the raw export, not an envelope
    assert_eq!(v["repos"][0]["nodes"].as_array().unwrap().len(), 3);
}
#[test]
fn refusals_exit_1_with_reason() {
    let ws = tempfile::tempdir().unwrap();
    cc(ws.path())
        .args(["build", "nope"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("no repo 'nope'"))
        .stdout(predicate::str::contains("\"ok\":false"));
    cc(ws.path())
        .args(["export", "json"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("0 nodes"))
        .stdout(predicate::str::contains("\"ok\":false"));
}
#[test]
fn doctor_reports_store_and_providers() {
    let ws = tempfile::tempdir().unwrap();
    cc(ws.path()).args(["build", "x"]).assert().code(1); // build creates the store
    cc(ws.path())
        .args(["doctor"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"reference\""))
        .stdout(predicate::str::contains("\"schema_version\":1"));
}

#[test]
fn conform_exits_0_on_repo_fixtures_and_1_when_a_fixture_fails() {
    let ws = tempfile::tempdir().unwrap();
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    cc(ws.path())
        .args(["conform", "reference", "--fixtures"])
        .arg(&fixtures)
        .assert()
        .success();
    let bad = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(bad.path().join("one")).unwrap();
    std::fs::write(bad.path().join("one/a.py"), "def f():\n    pass\n").unwrap();
    std::fs::write(
        bad.path().join("one/expected.json"),
        r#"{"nodes":["a.py::nonexistent"],"edges":[]}"#,
    )
    .unwrap();
    cc(ws.path())
        .args(["conform", "reference", "--fixtures"])
        .arg(bad.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\"passed\":false"))
        .stdout(predicate::str::contains("\"code\":\"below_floor\""))
        .stdout(predicate::str::contains("one: nodes"));
}

fn json_of(a: &assert_cmd::assert::Assert) -> serde_json::Value {
    serde_json::from_slice(&a.get_output().stdout).unwrap()
}
#[test]
fn envelope_shape_on_success_and_error() {
    let ws = tempfile::tempdir().unwrap();
    cc(ws.path()).args(["build", "x"]).assert().code(1); // build creates the store
    let a = cc(ws.path()).args(["doctor"]).assert().success();
    let v = json_of(&a);
    assert_eq!(v["ok"], true);
    assert_eq!(v["verb"], "doctor");
    assert_eq!(v["schema"], 1);
    assert_eq!(v["evidence"]["confidence"], "observed");
    assert!(v["cost"]["bytes"].as_u64().unwrap() > 0);
    assert_eq!(v["data"]["schema_version"], 1);
    let a = cc(ws.path()).args(["build", "nope"]).assert().code(1);
    let v = json_of(&a);
    assert_eq!(v["ok"], false);
    assert_eq!(v["verb"], "build");
    assert_eq!(v["error"]["code"], "not_found");
    assert!(v["error"]["message"].as_str().unwrap().contains("nope"));
}
#[test]
fn envelope_data_and_check_confidence() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone();
    cc(ws.path())
        .args(["add"])
        .arg(c.path())
        .args(["--name", "a"])
        .assert()
        .success();
    let v = json_of(&cc(ws.path()).args(["build", "a"]).assert().success());
    assert_eq!(v["data"]["nodes"], 3);
    let v = json_of(&cc(ws.path()).args(["check"]).assert().success());
    assert_eq!(v["evidence"]["confidence"], "derived");
    assert_eq!(v["data"]["problems"].as_array().unwrap().len(), 0);
}
#[test]
fn build_envelope_carries_run_limitations() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone();
    std::fs::write(c.path().join("src/bad.py"), [0xff, 0xfe, 0xfd, b'\n']).unwrap();
    cc(ws.path())
        .args(["add"])
        .arg(c.path())
        .args(["--name", "a"])
        .assert()
        .success();
    let v = json_of(&cc(ws.path()).args(["build", "a"]).assert().success());
    let l = v["evidence"]["limitations"].as_array().unwrap();
    assert!(l.iter().any(|x| x["code"] == "skipped_file"), "{l:?}");
    assert!(v["data"].get("limitation_items").is_none());
    assert!(v["data"]["communities"].as_u64().unwrap() >= 1);
}

#[test]
fn search_verb_returns_page_in_envelope() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone();
    cc(ws.path())
        .args(["add"])
        .arg(c.path())
        .args(["--name", "r"])
        .assert()
        .success();
    cc(ws.path()).args(["build", "r"]).assert().success();
    let out = cc(ws.path())
        .args(["search", "src", "--limit", "1"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["verb"], "search");
    assert_eq!(v["page"]["next"], "1");
    assert_eq!(v["data"]["items"].as_array().unwrap().len(), 1);
    cc(ws.path()).args(["search", "***"]).assert().code(1);
}

#[test]
fn failed_check_has_check_failed_code_and_incomplete_fts_is_reported() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone();
    cc(ws.path())
        .args(["add"])
        .arg(c.path())
        .args(["--name", "a"])
        .assert()
        .success();
    cc(ws.path()).args(["build", "a"]).assert().success();
    let db = ws.path().join(".comparegraph/comparegraph.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute(
        "DELETE FROM node_fts WHERE rowid = (SELECT MIN(rowid) FROM node_fts)",
        [],
    )
    .unwrap();
    drop(conn);
    let a = cc(ws.path()).args(["check"]).assert().code(1);
    let v = json_of(&a);
    assert_eq!(v["error"]["code"], "check_failed");
    assert!(v["data"]["problems"]
        .to_string()
        .contains("search index incomplete: rebuild a"));
}

#[test]
fn show_compact_through_the_binary() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone();
    cc(ws.path())
        .args(["add"])
        .arg(c.path())
        .args(["--name", "a"])
        .assert()
        .success();
    cc(ws.path()).args(["build", "a"]).assert().success();
    let a = cc(ws.path())
        .args(["show", "a", "src/a.ts::f"])
        .assert()
        .success();
    let v = json_of(&a);
    assert_eq!(v["verb"], "show");
    assert_eq!(v["ok"], true);
    assert!(v["data"].to_string().contains("\"f\""));
    assert!(v["data"].get("code").is_none() && v["data"].get("lines").is_none());
}

#[test]
fn read_verbs_on_an_empty_root_are_not_found_and_create_nothing() {
    for args in [
        vec!["search", "x"],
        vec!["repos"],
        vec!["communities", "a"],
        vec!["show", "a", "b"],
        vec!["check"],
        vec!["export", "json"],
        vec!["family", "list"],
        vec!["family", "show", "x"],
    ] {
        let ws = tempfile::tempdir().unwrap();
        let out = cc(ws.path()).args(&args).output().unwrap();
        assert!(!out.status.success(), "{args:?}");
        let s = String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr);
        assert!(
            s.contains("not_found") || s.contains("no workspace"),
            "{args:?}: {s}"
        );
        assert!(
            !ws.path().join(".comparegraph").exists(),
            "{args:?} created a store"
        );
    }
}

#[test]
fn root_may_follow_the_verb_and_repos_pages() {
    let ws = tempfile::tempdir().unwrap();
    for n in ["a", "b", "c"] {
        let c = clone();
        cc(ws.path()).args(["add"]).arg(c.path()).args(["--name", n]).assert().success();
    }
    let run = |args: &[&str]| -> serde_json::Value {
        let out = Command::cargo_bin("comparegraph").unwrap().args(args).arg("--root").arg(ws.path()).output().unwrap();
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let p1 = run(&["repos", "--limit", "2"]);
    assert_eq!(p1["data"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(p1["page"]["total"], 3);
    let next = p1["page"]["next"].as_str().unwrap().to_string();
    let p2 = run(&["repos", "--limit", "2", "--cursor", &next]);
    assert_eq!(p2["data"]["items"].as_array().unwrap().len(), 1);
    assert!(p2["page"]["next"].is_null());
    // front placement still works
    cc(ws.path()).arg("repos").assert().success();
}

#[test]
fn build_progress_goes_to_stderr_and_stdout_stays_one_envelope() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone();
    cc(ws.path()).args(["add"]).arg(c.path()).args(["--name", "a"]).assert().success();
    let out = cc(ws.path()).args(["build", "a", "--progress"]).output().unwrap();
    assert!(out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("[progress]"), "{err}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("[progress]"));
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout is exactly one JSON value");
    assert_eq!(v["ok"], true);
    let quiet = cc(ws.path()).args(["build", "a", "--full"]).output().unwrap();
    assert!(!String::from_utf8_lossy(&quiet.stderr).contains("[progress]"));
    let env = cc(ws.path()).env("COMPAREGRAPH_PROGRESS", "1").args(["build", "a", "--full"]).output().unwrap();
    assert!(String::from_utf8_lossy(&env.stderr).contains("[progress]"));
}
