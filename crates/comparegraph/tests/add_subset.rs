//! Field run 2026-11-07 items 6 and 7 (`add` half): `add/build --include/--exclude` recorded on the run, a `moved_to` note
//! on a registered repo, and `add --source-commit` for an export whose fresh `git init` has no head.
use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;
use std::process::Command as Sys;

fn commit(d: &Path) {
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
        assert!(Sys::new("git")
            .arg("-C")
            .arg(d)
            .args(&args)
            .status()
            .unwrap()
            .success());
    }
}
/// A clone with core code, tests and a GUI folder.
fn clone() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, s) in [
        (
            "src/core.py",
            "def core():\n    return helper()\n\ndef helper():\n    return 1\n",
        ),
        ("src/tests/test_core.py", "def test_core():\n    pass\n"),
        ("gui/app.ts", "export function render() { return 1; }\n"),
        ("tests/test_gui.py", "def test_gui():\n    pass\n"),
    ] {
        std::fs::create_dir_all(d.path().join(p).parent().unwrap()).unwrap();
        std::fs::write(d.path().join(p), s).unwrap();
    }
    commit(d.path());
    d
}
fn cc(ws: &Path) -> Command {
    let mut c = Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(ws);
    c
}
fn data(a: &assert_cmd::assert::Assert) -> serde_json::Value {
    serde_json::from_slice::<serde_json::Value>(&a.get_output().stdout).unwrap()["data"].clone()
}
fn paths_of_latest(ws: &Path, name: &str) -> Vec<String> {
    let s = comparegraph::store::Store::open(ws).unwrap();
    let r = s.repo_by_name(name).unwrap().unwrap();
    let run = s.latest_done_run(r.id.unwrap()).unwrap().unwrap();
    let mut p: Vec<String> = s
        .nodes_of_run(run.id.unwrap())
        .unwrap()
        .into_iter()
        .map(|n| n.path)
        .collect();
    p.sort();
    p.dedup();
    p
}
fn repo_item(ws: &Path, name: &str) -> serde_json::Value {
    let d = data(&cc(ws).arg("repos").assert().success());
    d["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == name)
        .unwrap()
        .clone()
}
fn strs(v: &serde_json::Value) -> Vec<&str> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect()
}

#[test]
fn add_subset_is_applied_by_build_and_recorded_on_the_run() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone();
    let d = data(
        &cc(ws.path())
            .arg("add")
            .arg(c.path())
            .args(["--name", "x", "--include", "src/**", "--exclude", "tests"])
            .assert()
            .success(),
    );
    assert_eq!(
        (strs(&d["include"]), strs(&d["exclude"])),
        (vec!["src/**"], vec!["tests"])
    );
    let b = data(&cc(ws.path()).args(["build", "x"]).assert().success());
    // the subset the build used is on its report and on the stored run
    assert_eq!(
        (strs(&b["include"]), strs(&b["exclude"])),
        (vec!["src/**"], vec!["tests"])
    );
    assert_eq!(paths_of_latest(ws.path(), "x"), vec!["src/core.py"]);
    let item = repo_item(ws.path(), "x");
    assert_eq!(strs(&item["include"]), vec!["src/**"]);
    assert_eq!(strs(&item["latest_run"]["include"]), vec!["src/**"]);
    assert_eq!(strs(&item["latest_run"]["exclude"]), vec!["tests"]);
    // build flags replace the repo's default subset for that run only, and the run says so
    let b = data(
        &cc(ws.path())
            .args(["build", "x", "--exclude", "gui"])
            .assert()
            .success(),
    );
    assert_eq!(b["reused"], false);
    assert_eq!(
        (strs(&b["include"]), strs(&b["exclude"])),
        (vec![], vec!["gui"])
    );
    assert_eq!(
        paths_of_latest(ws.path(), "x"),
        vec!["src/core.py", "src/tests/test_core.py", "tests/test_gui.py"]
    );
    let item = repo_item(ws.path(), "x");
    assert_eq!(strs(&item["latest_run"]["exclude"]), vec!["gui"]);
    assert_eq!(
        strs(&item["include"]),
        vec!["src/**"],
        "the repo's default subset stays"
    );
}

#[test]
fn a_changed_subset_is_never_answered_by_reuse() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone();
    cc(ws.path())
        .arg("add")
        .arg(c.path())
        .args(["--name", "x"])
        .assert()
        .success();
    cc(ws.path())
        .args(["build", "x", "--exclude", "gui"])
        .assert()
        .success();
    // the same files, but a different (no-op) exclude: the run must record what was asked, not the old subset
    let b = data(
        &cc(ws.path())
            .args(["build", "x", "--exclude", "gui", "--exclude", "*.rb"])
            .assert()
            .success(),
    );
    assert_eq!(b["reused"], false);
    assert_eq!(strs(&b["exclude"]), vec!["gui", "*.rb"]);
    // the identical subset again is reused
    let b = data(
        &cc(ws.path())
            .args(["build", "x", "--exclude", "gui", "--exclude", "*.rb"])
            .assert()
            .success(),
    );
    assert_eq!(b["reused"], true);
    assert_eq!(strs(&b["exclude"]), vec!["gui", "*.rb"]);
}

#[test]
fn an_incremental_subset_build_equals_a_full_one() {
    let c = clone();
    let a = tempfile::tempdir().unwrap();
    cc(a.path())
        .arg("add")
        .arg(c.path())
        .args(["--name", "x"])
        .assert()
        .success();
    cc(a.path()).args(["build", "x"]).assert().success();
    let inc = data(
        &cc(a.path())
            .args(["build", "x", "--exclude", "tests"])
            .assert()
            .success(),
    );
    let b = tempfile::tempdir().unwrap();
    cc(b.path())
        .arg("add")
        .arg(c.path())
        .args(["--name", "x"])
        .assert()
        .success();
    let full = data(
        &cc(b.path())
            .args(["build", "x", "--exclude", "tests", "--full"])
            .assert()
            .success(),
    );
    assert_eq!(inc["store_hash"], full["store_hash"]);
    assert_eq!(
        paths_of_latest(a.path(), "x"),
        vec!["gui/app.ts", "src/core.py"]
    );
}

#[test]
fn a_bad_glob_is_refused_before_anything_is_written() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone();
    cc(ws.path())
        .arg("add")
        .arg(c.path())
        .args(["--name", "x", "--include", "src/[a"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("src/[a"));
    cc(ws.path())
        .arg("add")
        .arg(c.path())
        .args(["--name", "x", "--include", "!src"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("--exclude"));
    assert!(comparegraph::store::Store::open(ws.path())
        .unwrap()
        .repo_by_name("x")
        .unwrap()
        .is_none());
    cc(ws.path())
        .arg("add")
        .arg(c.path())
        .args(["--name", "x"])
        .assert()
        .success();
    cc(ws.path())
        .args(["build", "x", "--exclude", "a{b"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("a{b"));
    let s = comparegraph::store::Store::open(ws.path()).unwrap();
    assert!(
        s.latest_run(s.repo_by_name("x").unwrap().unwrap().id.unwrap())
            .unwrap()
            .is_none(),
        "no run row for a refused build"
    );
}

#[test]
fn moved_to_is_set_at_add_amended_without_a_path_and_shown() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone();
    let d = data(
        &cc(ws.path())
            .arg("add")
            .arg(c.path())
            .args(["--name", "openhands", "--moved-to", "software-agent-sdk"])
            .assert()
            .success(),
    );
    assert_eq!(d["moved_to"], "software-agent-sdk");
    assert_eq!(
        repo_item(ws.path(), "openhands")["moved_to"],
        "software-agent-sdk"
    );
    // amend the notes of a registered repo: no path, only --name and the notes
    let d = data(
        &cc(ws.path())
            .args([
                "add",
                "--name",
                "openhands",
                "--moved-to",
                "https://github.com/x/sdk",
            ])
            .assert()
            .success(),
    );
    assert_eq!(d["moved_to"], "https://github.com/x/sdk");
    assert_eq!(
        repo_item(ws.path(), "openhands")["moved_to"],
        "https://github.com/x/sdk"
    );
    // show carries the note with the licence lane
    cc(ws.path())
        .args(["build", "openhands"])
        .assert()
        .success();
    let s = data(
        &cc(ws.path())
            .args(["show", "openhands"])
            .arg("src/core.py::core")
            .assert()
            .success(),
    );
    assert_eq!(s["moved_to"], "https://github.com/x/sdk");
    // an empty value clears it
    cc(ws.path())
        .args(["add", "--name", "openhands", "--moved-to", ""])
        .assert()
        .success();
    assert!(repo_item(ws.path(), "openhands")["moved_to"].is_null());
    // amending an unknown repo, or with nothing to amend, is refused
    cc(ws.path())
        .args(["add", "--name", "nope", "--moved-to", "y"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("nope"));
    cc(ws.path())
        .args(["add", "--name", "openhands"])
        .assert()
        .failure();
}

#[test]
fn source_commit_names_the_upstream_commit_of_a_fresh_git_init_export() {
    let ws = tempfile::tempdir().unwrap();
    let e = tempfile::tempdir().unwrap();
    std::fs::write(e.path().join("a.py"), "def f():\n    pass\n").unwrap();
    assert!(Sys::new("git")
        .arg("-C")
        .arg(e.path())
        .args(["init", "-q"])
        .status()
        .unwrap()
        .success());
    // the reported baseline: no commit, so head is null
    let d = data(
        &cc(ws.path())
            .arg("add")
            .arg(e.path())
            .args(["--name", "bare"])
            .assert()
            .success(),
    );
    assert!(d["head"].is_null());
    assert!(d["source_commit"].is_null());
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let d = data(
        &cc(ws.path())
            .arg("add")
            .arg(e.path())
            .args(["--name", "x", "--source-commit", sha])
            .assert()
            .success(),
    );
    assert_eq!(d["source_commit"], sha);
    assert_eq!(repo_item(ws.path(), "x")["source_commit"], sha);
    let b = data(&cc(ws.path()).args(["build", "x"]).assert().success());
    assert_eq!(b["source_commit"], sha);
    let s = data(
        &cc(ws.path())
            .args(["show", "x", "a.py::f"])
            .assert()
            .success(),
    );
    assert_eq!(s["source_commit"], sha);
    // set later on a registered repo, without a path
    cc(ws.path())
        .args(["add", "--name", "bare", "--source-commit", "abc1234"])
        .assert()
        .success();
    assert_eq!(repo_item(ws.path(), "bare")["source_commit"], "abc1234");
    // not a commit id: refused, nothing written
    cc(ws.path())
        .arg("add")
        .arg(e.path())
        .args(["--name", "y", "--source-commit", "main"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("main"));
    assert!(comparegraph::store::Store::open(ws.path())
        .unwrap()
        .repo_by_name("y")
        .unwrap()
        .is_none());
}

#[test]
fn replace_keeps_the_notes_but_drops_a_source_commit_not_given_again() {
    let ws = tempfile::tempdir().unwrap();
    let a = clone();
    let b = clone();
    cc(ws.path())
        .arg("add")
        .arg(a.path())
        .args([
            "--name",
            "x",
            "--source-commit",
            "abc1234",
            "--moved-to",
            "core",
            "--include",
            "src/**",
        ])
        .assert()
        .success();
    let a = cc(ws.path())
        .arg("add")
        .arg(b.path())
        .args(["--name", "x", "--replace"])
        .assert()
        .success();
    let env: serde_json::Value = serde_json::from_slice(&a.get_output().stdout).unwrap();
    assert!(
        env["data"]["source_commit"].is_null(),
        "a commit named for the old checkout does not describe the new one"
    );
    assert!(
        env["evidence"]["limitations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["code"] == "source_commit_cleared"),
        "{env}"
    );
    let item = repo_item(ws.path(), "x");
    assert_eq!(item["moved_to"], "core");
    assert_eq!(strs(&item["include"]), vec!["src/**"]);
    assert!(item["source_commit"].is_null());
    let c = clone();
    let d = data(
        &cc(ws.path())
            .arg("add")
            .arg(c.path())
            .args(["--name", "x", "--replace", "--source-commit", "def5678"])
            .assert()
            .success(),
    );
    assert_eq!(d["source_commit"], "def5678");
    assert_eq!(repo_item(ws.path(), "x")["source_commit"], "def5678");
}

#[test]
fn a_store_from_before_the_columns_gains_them_on_open() {
    let ws = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(ws.path().join(".comparegraph")).unwrap();
    {
        // the repo and run tables as schema.sql had them before the notes and the subset
        let c =
            rusqlite::Connection::open(ws.path().join(".comparegraph/comparegraph.db")).unwrap();
        c.execute_batch(
            "CREATE TABLE repo (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, path TEXT NOT NULL, remote TEXT, head TEXT, licence TEXT,
               lane TEXT NOT NULL DEFAULT 'study', role TEXT NOT NULL DEFAULT 'comparator', category TEXT, stars INTEGER, added_at TEXT NOT NULL);
             CREATE TABLE run (id INTEGER PRIMARY KEY, repo_id INTEGER NOT NULL REFERENCES repo(id), provider TEXT NOT NULL, provider_version TEXT NOT NULL,
               started_at TEXT NOT NULL, seconds REAL NOT NULL DEFAULT 0, status TEXT NOT NULL, limitations_json TEXT NOT NULL DEFAULT '[]', store_hash TEXT);
             INSERT INTO repo(id,name,path,added_at) VALUES (1,'old','/x','t');
             INSERT INTO run(id,repo_id,provider,provider_version,started_at,status) VALUES (1,1,'reference','1','t','done');",
        )
        .unwrap();
    }
    let s = comparegraph::store::Store::open(ws.path()).unwrap();
    let r = s.repo_by_name("old").unwrap().unwrap();
    assert_eq!(
        (
            r.source_commit,
            r.moved_to,
            r.include.len(),
            r.exclude.len()
        ),
        (None, None, 0, 0)
    );
    assert_eq!(
        s.run_subset(comparegraph::domain::RunId(1)).unwrap(),
        (vec![], vec![])
    );
}
