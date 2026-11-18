//! The delta "already on base" model (design read 2026-11-19, docs/decisions.md "Plan 14: the delta model"): whole-tree copy
//! counts, a computed unread set, one-to-one credit and a three-valued answer. Each test is a review probe (rounds 1-3) or a
//! slice's red test; fixture repos are real git repositories in temp dirs, no network.
#![allow(dead_code)]
mod common;
use common::{git, ok};
use serde_json::Value;
use std::path::{Path, PathBuf};

const MIT: &str = "MIT License\n\nCopyright (c) 2026 S\n";
const BAR: &str = "def bar(x):\n    y = x * 2\n    return y + 1\n";
const FOO: &str = "def foo():\n    return 1\n";
const KEEP: &str = "def keep():\n    return 1\n";

fn commit(dir: &Path, msg: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", msg]);
}
fn write(dir: &Path, rel: &str, body: &str) {
    let f = dir.join(rel);
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(f, body).unwrap();
}

struct Fx {
    ws: tempfile::TempDir,
    base: tempfile::TempDir,
}
impl Fx {
    /// A clone on branch `main` with the given files, registered as `r` in a fresh store.
    fn new(files: &[(&str, &str)]) -> Fx {
        let (ws, base) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let d = base.path().join("r");
        std::fs::create_dir_all(&d).unwrap();
        git(&d, &["init", "-q", "-b", "main"]);
        write(&d, "LICENSE", MIT);
        for (p, c) in files {
            write(&d, p, c);
        }
        commit(&d, "c1");
        ok(ws.path(), &["add", d.to_str().unwrap(), "--name", "r"]);
        Fx { ws, base }
    }
    fn d(&self) -> PathBuf {
        self.base.path().join("r")
    }
    fn root(&self) -> &Path {
        self.ws.path()
    }
    fn delta(&self) -> Value {
        ok(self.root(), &["delta", "r", "--base", "main", "--head", "feat", "--refresh"])
    }
    fn feat(&self) {
        git(&self.d(), &["checkout", "-q", "-b", "feat"]);
    }
    fn main(&self) {
        git(&self.d(), &["checkout", "-q", "main"]);
    }
}

/// `def process(items)`: 30 lines with their own names, optionally a 2-line guard and a tweak on the return.
fn long_process(guard: bool, tweak: &str) -> String {
    let mut s = String::from("def process(items):\n    total = 0\n");
    if guard {
        s.push_str("    if not items:\n        raise Denied()\n");
    }
    for i in 0..30 {
        s.push_str(&format!("    total = total + items[{i}] * {} - len(items) + k{i}\n", i + 2));
    }
    s.push_str(&format!("    return total{tweak}\n"));
    s
}
fn long_named(name: &str, guard: bool) -> String {
    long_process(guard, "").replacen("def process(", &format!("def {name}("), 1)
}
fn exact_share(v: &Value) -> Option<f64> {
    v["data"]["summary"]["share_on_base_exact"].as_f64()
}
fn branch(v: &Value) -> &Value {
    &v["data"]["branch"]
}
fn limits(v: &Value) -> String {
    v["evidence"]["limitations"].to_string()
}

// ---------------------------------------------------------------- slice 0: every side's limitation; the whole-tree exact share

/// P5's limitation half: base renames the file to a Windows reserved name, which base's run skips. The skipped path must reach
/// the delta's own limitations (every limitation of every side is carried, not a fixed list of codes).
#[test]
fn s0_every_side_limitation_is_carried_into_the_delta() {
    let fx = Fx::new(&[("src/a.py", &format!("{}\n{KEEP}", long_process(false, ""))), ("src/k.py", KEEP)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", KEEP);
    commit(&d, "lane deletes process");
    fx.main();
    git(&d, &["mv", "src/a.py", "src/aux.py"]);
    commit(&d, "base renames to aux.py");
    let v = fx.delta();
    let l = limits(&v);
    assert!(l.contains("windows_reserved_skipped") && l.contains("src/aux.py"), "the skipped path is silent in delta: {l}");
    let lim = v["evidence"]["limitations"].as_array().unwrap().iter().find(|x| x["code"] == "windows_reserved_skipped").unwrap().clone();
    assert!(lim["detail"].as_str().unwrap().starts_with("base"), "the side is named: {lim}");
}

/// P8: the whole-tree summary carries the exact share under the branch summary's name.
#[test]
fn p8_whole_tree_has_share_on_base_exact() {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", &format!("{FOO}\n{BAR}"));
    commit(&d, "feat adds bar");
    fx.main();
    let v = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--refresh", "--whole-tree"]);
    let ws = &v["data"]["whole_tree"]["summary"];
    assert_eq!(ws["share_on_base_exact"].as_f64(), Some(0.0), "whole_tree.summary lacks share_on_base_exact: {ws}");
    assert_eq!(ws["share_on_base_exact"], ws["share_already_on_base"], "{ws}");
}

// ---------------------------------------------------------------- slice 1: copy counts per (kind, body hash) replace `fresh`

/// A row is landed work (exact): its answer is an exact match, never a near one.
fn exact(row: &Value) -> bool {
    row.get("already_on_base").is_some_and(|m| m.is_object() && m["via"] != "name_and_tokens")
}
fn row_of<'a>(v: &'a Value, section: &str, qn: &str) -> &'a Value {
    branch(v)[section].as_array().unwrap().iter().find(|r| r["qualified_name"] == qn).unwrap_or_else(|| panic!("no {section} row {qn}: {}", branch(v)))
}

/// P1: the branch makes f equal to the merge base's helper g; base only moves the file. Base never got the branch's change:
/// h - m = 1 (one new copy of g's body) but b - m = 0 (base made none), so A4 credits nothing.
#[test]
fn p1_copy_of_an_old_helper_with_base_moving_the_file() {
    let g = long_named("g", true);
    let fx = Fx::new(&[("src/a.py", &format!("{}\n{g}", long_named("f", false)))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", &format!("{}\n{g}", long_named("f", true)));
    commit(&d, "lane makes f guard like g");
    fx.main();
    git(&d, &["mv", "src/a.py", "src/c.py"]);
    commit(&d, "base moves the file");
    let v = fx.delta();
    let row = row_of(&v, "changed", "src/a.py::f");
    assert!(!exact(row), "FALSE EXACT: {row}");
    assert_eq!(exact_share(&v), Some(0.0), "{}", v["data"]["summary"]);
}

/// P2: the branch swaps the bodies of f and g; base only moves the file. h - m = 0 for both bodies (A3): never exact.
#[test]
fn p2_swapped_bodies_with_base_moving_the_file() {
    let fx = Fx::new(&[("src/a.py", &format!("{}\n{}", long_named("f", false), long_named("g", true)))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", &format!("{}\n{}", long_named("f", true), long_named("g", false)));
    commit(&d, "lane swaps");
    fx.main();
    git(&d, &["mv", "src/a.py", "src/c.py"]);
    commit(&d, "base moves the file");
    let v = fx.delta();
    assert_eq!(exact_share(&v), Some(0.0), "FALSE EXACT: {}", branch(&v));
    for qn in ["src/a.py::f", "src/a.py::g"] {
        assert_eq!(row_of(&v, "changed", qn)["note"], "body_on_merge_base", "{}", branch(&v));
    }
}

/// P2b: swapped bodies, base untouched.
#[test]
fn p2b_swapped_bodies_base_untouched() {
    let fx = Fx::new(&[("src/a.py", &format!("{}\n{}", long_named("f", false), long_named("g", true)))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", &format!("{}\n{}", long_named("f", true), long_named("g", false)));
    commit(&d, "lane swaps");
    fx.main();
    let v = fx.delta();
    assert_eq!(exact_share(&v), Some(0.0), "FALSE EXACT: {}", branch(&v));
}

/// P3: the branch adds a new function whose body copies the old helper g; base only moves g's file.
#[test]
fn p3_added_copy_of_an_old_helper_with_base_moving_the_file() {
    let fx = Fx::new(&[("src/a.py", &long_named("g", true))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/n.py", &long_named("h", true));
    commit(&d, "lane adds h, a copy of g");
    fx.main();
    git(&d, &["mv", "src/a.py", "src/c.py"]);
    commit(&d, "base moves the file");
    let v = fx.delta();
    let row = row_of(&v, "added", "src/n.py::h");
    assert!(!exact(row), "FALSE EXACT: {row}");
    assert_eq!(exact_share(&v), Some(0.0));
}

/// P9: the branch writes the same new body twice (f changed to X, and a new k = X); base took only k. Base's one copy is k's
/// own (A1), so it credits nothing more: f is not on base (one-to-one credit).
#[test]
fn p9_one_base_copy_vouches_for_one_branch_row_only() {
    let fx = Fx::new(&[("src/a.py", &long_named("f", false))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", &long_named("f", true));
    write(&d, "src/k.py", &long_named("k", true));
    commit(&d, "lane guards f and adds k with the same body");
    fx.main();
    write(&d, "src/k.py", &long_named("k", true));
    commit(&d, "base takes k only");
    let v = fx.delta();
    let f = row_of(&v, "changed", "src/a.py::f");
    assert!(!exact(f), "FALSE EXACT: base's f is unguarded: {f}");
    assert_eq!(row_of(&v, "added", "src/k.py::k")["already_on_base"]["via"], "same_symbol", "{}", branch(&v));
    assert_eq!(exact_share(&v), Some(0.5), "{}", v["data"]["summary"]);
}

/// Duplicate keys are counted (`load_side` keeps every copy): two copies of a body on base under one key still count twice.
/// Here the provider gives each redefinition its own `#n` name, so the copies are distinct rows; the branch's two new copies
/// of X are credited by base's two copies of X, one each.
#[test]
fn s1_two_base_copies_credit_two_branch_rows() {
    let fx = Fx::new(&[("src/a.py", KEEP)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/n.py", &format!("{}\n{}", long_named("p", true), long_named("q", true)));
    commit(&d, "lane adds p and q, one body");
    fx.main();
    write(&d, "src/o.py", &format!("{}\n{}", long_named("p2", true), long_named("q2", true)));
    commit(&d, "base takes both under other names");
    let v = fx.delta();
    for qn in ["src/n.py::p", "src/n.py::q"] {
        assert_eq!(row_of(&v, "added", qn)["already_on_base"]["via"], "body_hash", "{}", branch(&v));
    }
    assert_eq!(exact_share(&v), Some(1.0));
}
