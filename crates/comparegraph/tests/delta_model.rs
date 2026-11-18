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

// ---------------------------------------------------------------- slice 2: the unread set U(T); removed rows R1-R5; A5

/// The branch deletes `process` from src/a.py (keeping `keep`); `base_change` then edits base's src/a.py.
fn deletion_with_base(base_change: impl Fn(&Path)) -> Value {
    let fx = Fx::new(&[("src/a.py", &format!("{}\n{KEEP}", long_process(false, ""))), ("src/k.py", KEEP)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", KEEP);
    commit(&d, "lane deletes process");
    fx.main();
    base_change(&d);
    commit(&d, "base changes the file");
    fx.delta()
}

/// P4: base keeps `process` but its file grows over max_file_bytes, so base's run never reads it. "No copy found" is not "gone":
/// base gained a blob its run did not read (R5).
#[test]
fn p4_deletion_where_base_file_is_oversize() {
    let v = deletion_with_base(|d| {
        let pad = format!("# {}\n", "x".repeat(1_100_000));
        write(d, "src/a.py", &format!("{}\n{KEEP}\n{pad}", long_process(false, "")));
    });
    assert_eq!(exact_share(&v), Some(0.0), "FALSE EXACT: base still holds process: {}", branch(&v));
    assert_eq!(row_of(&v, "removed", "src/a.py::process")["note"], "base_unread", "{}", branch(&v));
}

/// P4b: base adds one latin-1 byte to the file (its run skips a non-UTF-8 file).
#[test]
fn p4b_deletion_where_base_file_became_non_utf8() {
    let v = deletion_with_base(|d| {
        let mut bytes = format!("{}\n{KEEP}\n# caf", long_process(false, "")).into_bytes();
        bytes.push(0xe9);
        bytes.push(b'\n');
        std::fs::write(d.join("src/a.py"), bytes).unwrap();
    });
    assert_eq!(exact_share(&v), Some(0.0), "FALSE EXACT: base still holds process: {}", branch(&v));
    assert!(!row_of(&v, "removed", "src/a.py::process")["note"].is_null(), "{}", branch(&v));
}

/// P5: base renames the file to a Windows reserved name (skipped by base's run); base holds `process` at src/aux.py.
#[test]
fn p5_deletion_where_base_renamed_the_file_to_a_reserved_name() {
    let v = deletion_with_base(|d| git(d, &["mv", "src/a.py", "src/aux.py"]));
    assert!(limits(&v).contains("windows_reserved_skipped"), "{}", limits(&v));
    assert_eq!(exact_share(&v), Some(0.0), "FALSE EXACT: base still holds process: {}", branch(&v));
    assert_eq!(row_of(&v, "removed", "src/a.py::process")["note"], "base_unread", "{}", branch(&v));
}

/// R4: base deleted the symbol too, and base has no blob its run did not read: yes.
#[test]
fn s2_a_deletion_base_made_too_with_nothing_unread_is_on_base() {
    let v = deletion_with_base(|d| write(d, "src/a.py", &format!("{KEEP}\n")));
    assert_eq!(exact_share(&v), Some(1.0), "{}", branch(&v));
}

/// A5: the merge base holds g's body in a file its run could not read (over max_file_bytes); base trims the file, so base's run
/// reads g. Base's copy of the body is old code the merge base already had: b - m is only positive because m missed it.
#[test]
fn s2_a_credit_needs_the_merge_base_read_whole() {
    let pad = format!("# {}\n", "x".repeat(1_100_000));
    let fx = Fx::new(&[("src/big.py", &format!("{}\n{pad}", long_named("g", true))), ("src/k.py", KEEP)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/n.py", &long_named("h", true));
    commit(&d, "lane adds h, a copy of g");
    fx.main();
    write(&d, "src/big.py", &long_named("g", true));
    commit(&d, "base trims big.py: its run now reads g");
    let v = fx.delta();
    let row = row_of(&v, "added", "src/n.py::h");
    assert!(!exact(row), "FALSE EXACT: g was on the merge base, unread: {row}");
    assert_eq!(row["note"], "merge_base_unread", "{row}");
    assert_eq!(exact_share(&v), Some(0.0));
}

// ---------------------------------------------------------------- slice 3: on_base yes | no | unknown, with via and why

/// The duplicate-credit split (A4): the branch writes one new body three times; base took it once, under another name. One
/// credit for three rows: which row base's copy is cannot be told, so all three are unknown (`shared_credit`), and unknown
/// counts against the exact share.
#[test]
fn s3_one_credit_for_three_rows_is_unknown_for_all_three() {
    let fx = Fx::new(&[("src/a.py", KEEP)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/n.py", &format!("{}\n{}\n{}", long_named("p", true), long_named("q", true), long_named("r", true)));
    commit(&d, "lane adds p, q and r, one body");
    fx.main();
    write(&d, "src/o.py", &long_named("p2", true));
    commit(&d, "base takes one copy");
    let v = fx.delta();
    for qn in ["src/n.py::p", "src/n.py::q", "src/n.py::r"] {
        let row = row_of(&v, "added", qn);
        assert_eq!((row["on_base"].as_str(), row["why"].as_str()), (Some("unknown"), Some("shared_credit")), "{row}");
        assert!(row.get("via").is_none(), "{row}");
    }
    let s = &v["data"]["summary"];
    assert_eq!(s["unknown"]["count"].as_i64(), Some(3), "{s}");
    assert_eq!(s["unknown"]["by_why"]["shared_credit"].as_i64(), Some(3), "{s}");
    assert_eq!((s["on_base"]["yes"].as_i64(), s["on_base"]["no"].as_i64(), s["on_base"]["unknown"].as_i64()), (Some(0), Some(0), Some(3)), "{s}");
    assert_eq!(exact_share(&v), Some(0.0), "{s}");
    assert_eq!(s["share_already_on_base"], s["share_on_base_exact"], "{s}");
}

/// R4 needs base to have added or changed no gitlink since the fork: a submodule's commit is never read, so a symbol base
/// "deleted" may live on in it. The branch and base both delete `bar`; base also adds a submodule entry.
#[test]
fn s3_a_deletion_with_a_gitlink_added_on_base_is_unknown() {
    let fx = Fx::new(&[("src/a.py", &format!("{FOO}\n{BAR}"))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", FOO);
    commit(&d, "lane deletes bar");
    fx.main();
    write(&d, "src/a.py", &format!("{FOO}
")); // not the branch's blob: the file itself is not the same on both
    commit(&d, "base deletes bar too");
    let tip = git_out(&d, &["rev-parse", "main"]);
    let mut entries = std::process::Command::new("git").arg("-C").arg(&d).args(["ls-tree", "main"]).output().unwrap().stdout;
    entries.extend_from_slice(format!("160000 commit {tip}\tvendored\n").as_bytes());
    let tree = git_in(&d, &["mktree"], &entries);
    let c = git_in(&d, &["-c", "user.email=t@t", "-c", "user.name=t", "commit-tree", &tree, "-p", "main", "-m", "base adds a submodule"], b"");
    git(&d, &["update-ref", "refs/heads/main", &c]);
    let v = fx.delta();
    let row = row_of(&v, "removed", "src/a.py::bar");
    assert_eq!((row["on_base"].as_str(), row["why"].as_str()), (Some("unknown"), Some("base_unread")), "{row}");
    assert_eq!(exact_share(&v), Some(0.0), "{}", v["data"]["summary"]);
    // without the gitlink the same deletion is on base (R4)
    git(&d, &["update-ref", "refs/heads/main", &tip]);
    let v = fx.delta();
    let row = row_of(&v, "removed", "src/a.py::bar");
    assert_eq!((row["on_base"].as_str(), row["via"].as_str()), (Some("yes"), Some("absent_on_base")), "{row}");
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let o = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}");
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}
fn git_in(dir: &Path, args: &[&str], stdin: &[u8]) -> String {
    use std::io::Write;
    let mut c = std::process::Command::new("git").arg("-C").arg(dir).args(args).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(stdin).unwrap();
    let o = c.wait_with_output().unwrap();
    assert!(o.status.success(), "git {args:?}");
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}
