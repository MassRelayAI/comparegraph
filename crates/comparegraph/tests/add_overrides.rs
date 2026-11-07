//! `add --licence/--lane/--licence-from` (field run 2026-11-07 item 13): a partial export has no LICENSE file.
use assert_cmd::Command;
use predicates::prelude::*;
use std::process::Command as Sys;

fn commit(d: &std::path::Path) {
    for args in [vec!["init", "-q"], vec!["add", "."], vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "x"]] {
        assert!(Sys::new("git").arg("-C").arg(d).args(&args).status().unwrap().success());
    }
}
/// A partial export: source files, no LICENSE.
fn partial_export() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("src")).unwrap();
    std::fs::write(d.path().join("src/a.ts"), "export function f() { return 1; }\n").unwrap();
    commit(d.path());
    d
}
/// A full clone with an Apache licence.
fn full_clone() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("src")).unwrap();
    std::fs::write(d.path().join("src/a.ts"), "export function f() { return 1; }\n").unwrap();
    std::fs::write(d.path().join("LICENSE"), "Apache License\nVersion 2.0\n").unwrap();
    commit(d.path());
    d
}
fn cc(ws: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(ws);
    c
}
fn data(a: &assert_cmd::assert::Assert) -> serde_json::Value {
    serde_json::from_slice::<serde_json::Value>(&a.get_output().stdout).unwrap()["data"].clone()
}
fn pair(d: &serde_json::Value) -> (Option<&str>, Option<&str>) {
    (d["licence"].as_str(), d["lane"].as_str())
}
fn stored(ws: &std::path::Path, name: &str) -> (Option<String>, String) {
    let r = comparegraph::store::Store::open(ws).unwrap().repo_by_name(name).unwrap().unwrap();
    (r.licence, r.lane)
}

#[test]
fn add_licence_and_lane_overrides_win_over_a_missing_licence_file() {
    let ws = tempfile::tempdir().unwrap();
    let p = partial_export();
    // detection alone: no LICENSE, so null/study (the reported baseline)
    let d = data(&cc(ws.path()).args(["add"]).arg(p.path()).args(["--name", "bare"]).assert().success());
    assert_eq!(pair(&d), (None, Some("study")));
    // --licence MIT: the lane follows the licence, and the row is stored so
    let d = data(&cc(ws.path()).args(["add"]).arg(p.path()).args(["--name", "mit", "--licence", "MIT"]).assert().success());
    assert_eq!(pair(&d), (Some("MIT"), Some("take")));
    assert_eq!(stored(ws.path(), "mit"), (Some("MIT".into()), "take".into()));
    // --lane alone sets the lane; the licence stays as detected
    let d = data(&cc(ws.path()).args(["add"]).arg(p.path()).args(["--name", "laned", "--lane", "take"]).assert().success());
    assert_eq!(pair(&d), (None, Some("take")));
    // an explicit --lane beats the licence's lane
    let d = data(&cc(ws.path()).args(["add"]).arg(p.path()).args(["--name", "both", "--licence", "MIT", "--lane", "study"]).assert().success());
    assert_eq!(pair(&d), (Some("MIT"), Some("study")));
    // an unknown lane is refused with the allowed values, and nothing is registered
    cc(ws.path())
        .args(["add"])
        .arg(p.path())
        .args(["--name", "bad", "--lane", "borrow"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("borrow").and(predicate::str::contains("take")).and(predicate::str::contains("study")));
    assert!(comparegraph::store::Store::open(ws.path()).unwrap().repo_by_name("bad").unwrap().is_none());
}

#[test]
fn add_licence_from_reads_the_source_clone() {
    let ws = tempfile::tempdir().unwrap();
    let p = partial_export();
    let src = full_clone();
    let d = data(&cc(ws.path()).args(["add"]).arg(p.path()).args(["--name", "x", "--licence-from"]).arg(src.path()).assert().success());
    assert_eq!(pair(&d), (Some("Apache-2.0"), Some("take")));
    // a source with no recognisable licence is refused, not silently null
    let nolic = partial_export();
    cc(ws.path())
        .args(["add"])
        .arg(p.path())
        .args(["--name", "y", "--licence-from"])
        .arg(nolic.path())
        .assert()
        .failure()
        .stdout(predicate::str::contains("licence"));
    assert!(comparegraph::store::Store::open(ws.path()).unwrap().repo_by_name("y").unwrap().is_none());
    // --licence and --licence-from together are refused
    cc(ws.path())
        .args(["add"])
        .arg(p.path())
        .args(["--name", "z", "--licence", "MIT", "--licence-from"])
        .arg(src.path())
        .assert()
        .failure();
}

#[test]
fn add_replace_applies_the_overrides_and_keeps_the_row_otherwise() {
    let ws = tempfile::tempdir().unwrap();
    let full = full_clone();
    cc(ws.path()).args(["add"]).arg(full.path()).args(["--name", "r"]).assert().success();
    // replace without overrides: licence/lane stay as registered
    let p1 = partial_export();
    let d = data(&cc(ws.path()).args(["add"]).arg(p1.path()).args(["--name", "r", "--replace"]).assert().success());
    assert_eq!(pair(&d), (Some("Apache-2.0"), Some("take")));
    assert_eq!(stored(ws.path(), "r"), (Some("Apache-2.0".into()), "take".into()));
    // replace with --licence: the row takes it and the lane follows
    let p2 = partial_export();
    let d = data(&cc(ws.path()).args(["add"]).arg(p2.path()).args(["--name", "r", "--replace", "--licence", "GPL-3.0"]).assert().success());
    assert_eq!(pair(&d), (Some("GPL-3.0"), Some("study")));
    assert_eq!(stored(ws.path(), "r"), (Some("GPL-3.0".into()), "study".into()));
    // replace with --lane: the lane only
    let p3 = partial_export();
    cc(ws.path()).args(["add"]).arg(p3.path()).args(["--name", "r", "--replace", "--lane", "take"]).assert().success();
    assert_eq!(stored(ws.path(), "r"), (Some("GPL-3.0".into()), "take".into()));
    // an unknown lane on replace is refused before the path moves
    let p4 = partial_export();
    cc(ws.path()).args(["add"]).arg(p4.path()).args(["--name", "r", "--replace", "--lane", "nope"]).assert().failure();
    let row = comparegraph::store::Store::open(ws.path()).unwrap().repo_by_name("r").unwrap().unwrap();
    assert_eq!(row.path, p3.path().canonicalize().unwrap().to_string_lossy());
}
