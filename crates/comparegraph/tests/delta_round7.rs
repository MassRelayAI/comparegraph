//! Round-7 review probes of the delta model (feat/plan-14 at fe79562): four false 1.0s, all "work base merged and later took
//! back" (a back-merge, unrelated histories, a shallow boundary), a fast-forward then revert (a known limit, asserted as it is),
//! and the pins of round 7.
#![allow(dead_code)]
mod common;
use common::{git, ok, run};
use serde_json::Value;
use std::path::{Path, PathBuf};

const MIT: &str = "MIT License\n\nCopyright (c) 2026 S\n";
const G5: &str = "def g(items):\n    total = 0\n    for x in items:\n        total = total + x * 3 - len(items)\n        log(total, x, items)\n    return total\n";
const H5: &str = "def h(items):\n    acc = 1\n    for y in items:\n        acc = acc * y + 7 - len(items)\n        trace(acc, y, items)\n    return acc\n";
const K5: &str = "def k(items):\n    s = 2\n    for z in items:\n        s = s - z * 5 + len(items)\n        note(s, z, items)\n    return s\n";
const O5: &str = "def o(items):\n    w = 9\n    for q in items:\n        w = w + q * 11 - len(items)\n        emit(w, q, items)\n    return w\n";

fn gc(d: &Path, args: &[&str]) {
    let mut a = vec!["-c", "user.email=t@t", "-c", "user.name=t"];
    a.extend_from_slice(args);
    git(d, &a);
}
fn commit(dir: &Path, msg: &str) {
    git(dir, &["add", "-A"]);
    gc(dir, &["commit", "-q", "-m", msg]);
}
fn write(dir: &Path, rel: &str, body: &str) {
    let f = dir.join(rel);
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(f, body).unwrap();
}
fn git_out(dir: &Path, args: &[&str]) -> String {
    let o = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}");
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}
struct Fx {
    ws: tempfile::TempDir,
    base: tempfile::TempDir,
}
impl Fx {
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
    fn delta_of(&self, base: &str, head: &str) -> Value {
        run(self.ws.path(), &["delta", "r", "--base", base, "--head", head, "--refresh"])
    }
    fn delta(&self) -> Value {
        self.delta_of("main", "feat")
    }
}
fn s(v: &Value) -> String {
    format!("ok={} SHARE={} unknown={} residue={} err={}", v["ok"], v["data"]["summary"]["share_on_base_exact"], v["data"]["summary"]["unknown"], v["data"]["branch"]["residue"], v["error"])
}
fn share(v: &Value) -> Option<f64> {
    v["data"]["summary"]["share_on_base_exact"].as_f64()
}
fn file_on(d: &Path, rev: &str, p: &str) -> String {
    let o = std::process::Command::new("git").arg("-C").arg(d).args(["show", &format!("{rev}:{p}")]).output().unwrap();
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// P1: base merges feat (--no-ff), feat fast-forwards to that merge (a back-merge), base reverts g, feat adds h, base takes h.
/// mb = F = the merge: the B2 check is skipped. head holds g, base does not.
#[test]
fn p1_back_merge_then_revert() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "add g");
    git(&d, &["checkout", "-q", "main"]);
    gc(&d, &["merge", "-q", "--no-ff", "-m", "merge feat", "feat"]);
    git(&d, &["checkout", "-q", "feat"]);
    gc(&d, &["merge", "-q", "--ff-only", "main"]);
    write(&d, "src/b.py", H5);
    commit(&d, "add h");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/a.py", K5);
    commit(&d, "base reverts g");
    write(&d, "src/b.py", H5);
    commit(&d, "take h");
    let v = fx.delta();
    eprintln!("P1 {}", s(&v));
    assert!(file_on(&d, "feat", "src/a.py").contains("def g") && !file_on(&d, "main", "src/a.py").contains("def g"));
    assert_ne!(share(&v), Some(1.0), "P1 false 1.0: {}", s(&v));
    assert_eq!(v["data"]["summary"]["unknown"]["by_why"]["merged_into_base_before"].as_i64(), Some(1), "{}", s(&v));
}

/// P1b: the same with a --no-ff back-merge commit on feat.
#[test]
fn p1b_no_ff_back_merge_then_revert() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "add g");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/m.py", O5);
    commit(&d, "base work");
    gc(&d, &["merge", "-q", "--no-ff", "-m", "merge feat", "feat"]);
    git(&d, &["checkout", "-q", "feat"]);
    gc(&d, &["merge", "-q", "--no-ff", "-m", "back-merge", "main"]);
    write(&d, "src/b.py", H5);
    commit(&d, "add h");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/a.py", K5);
    commit(&d, "base reverts g");
    write(&d, "src/b.py", H5);
    commit(&d, "take h");
    let v = fx.delta();
    eprintln!("P1b {}", s(&v));
    assert_ne!(share(&v), Some(1.0), "P1b false 1.0: {}", s(&v));
}

/// P2, A KNOWN LIMIT (docs/decisions.md, Plan 14 round 7): base fast-forwards to feat, reverts g; feat adds h, base takes h.
/// mb = F = feat's first tip, on base's own first-parent chain: the graph cannot tell this from base reverting its own commit
/// from before the fork, so the delta reads 1.0. This test asserts that behaviour so a change to it is seen; a person
/// confirms before a branch is deleted, and a deleted branch is always bundled first.
#[test]
fn known_limit_fast_forward_then_revert_reads_one() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "add g");
    git(&d, &["checkout", "-q", "main"]);
    gc(&d, &["merge", "-q", "--ff-only", "feat"]);
    write(&d, "src/a.py", K5);
    commit(&d, "base reverts g");
    git(&d, &["checkout", "-q", "feat"]);
    write(&d, "src/b.py", H5);
    commit(&d, "add h");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/b.py", H5);
    commit(&d, "take h");
    let v = fx.delta();
    eprintln!("P2 {}", s(&v));
    assert_eq!(share(&v), Some(1.0), "the known limit changed (good, if on purpose): {}", s(&v));
}

/// P3: feat is an orphan history; base merges it (--allow-unrelated-histories), removes g; feat adds h, base takes h.
/// F is None (head reaches no commit of base's first-parent chain): B2 skipped.
#[test]
fn p3_unrelated_histories_joined_then_revert() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "--orphan", "feat"]);
    write(&d, "src/g.py", G5);
    commit(&d, "feat root: add g");
    git(&d, &["checkout", "-q", "main"]);
    gc(&d, &["merge", "-q", "--no-ff", "--allow-unrelated-histories", "-m", "join", "feat"]);
    git(&d, &["rm", "-q", "src/g.py"]);
    commit(&d, "base drops g");
    git(&d, &["checkout", "-q", "feat"]);
    write(&d, "src/b.py", H5);
    commit(&d, "add h");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/b.py", H5);
    commit(&d, "take h");
    let v = fx.delta();
    eprintln!("P3 {}", s(&v));
    assert!(file_on(&d, "feat", "src/g.py").contains("def g") && !file_on(&d, "main", "src/g.py").contains("def g"));
    assert_ne!(share(&v), Some(1.0), "P3 false 1.0: {}", s(&v));
    assert_eq!(v["data"]["summary"]["unknown"]["by_why"]["merged_into_base_before"].as_i64(), Some(1), "{}", s(&v));
    assert!(v["evidence"]["limitations"].to_string().contains("reaches no commit of base's first-parent chain"), "{}", v["evidence"]["limitations"]);
}

/// P4: the round-6 merged-then-reverted shape, but base's first-parent chain is cut by a shallow boundary (.git/shallow)
/// before the fork, while the merge base stays reachable through the merge's second parent.
#[test]
fn p4_shallow_cuts_base_first_parent_before_the_fork() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "add g");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/m.py", O5);
    commit(&d, "c2");
    let c2 = git_out(&d, &["rev-parse", "HEAD"]);
    gc(&d, &["merge", "-q", "--no-ff", "-m", "merge feat", "feat"]);
    write(&d, "src/a.py", K5);
    commit(&d, "base reverts g");
    git(&d, &["checkout", "-q", "feat"]);
    write(&d, "src/b.py", H5);
    commit(&d, "add h");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/b.py", H5);
    commit(&d, "take h");
    let full = fx.delta();
    eprintln!("P4 full history {}", s(&full));
    std::fs::write(d.join(".git/shallow"), format!("{c2}\n")).unwrap();
    let v = fx.delta();
    eprintln!("P4 shallow {}", s(&v));
    assert_ne!(share(&v), Some(1.0), "P4 false 1.0 under a shallow boundary: {}", s(&v));
    assert!(v["evidence"]["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "shallow_history"), "{}", v["evidence"]["limitations"]);
}

/// P6: an octopus merge on base (feat and another branch), then base reverts g; feat adds h, base takes h.
#[test]
fn p6_octopus_merge_then_revert() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "add g");
    git(&d, &["checkout", "-q", "-b", "other", "main"]);
    write(&d, "src/o.py", O5);
    commit(&d, "add o");
    git(&d, &["checkout", "-q", "main"]);
    gc(&d, &["merge", "-q", "--no-ff", "-m", "octopus", "feat", "other"]);
    write(&d, "src/a.py", K5);
    commit(&d, "base reverts g");
    git(&d, &["checkout", "-q", "feat"]);
    write(&d, "src/b.py", H5);
    commit(&d, "add h");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/b.py", H5);
    commit(&d, "take h");
    let v = fx.delta();
    eprintln!("P6 {}", s(&v));
    assert_ne!(share(&v), Some(1.0), "P6 false 1.0: {}", s(&v));
}

/// P7: head an ancestor of base; head equal to base.
#[test]
fn p7_head_ancestor_and_equal() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["branch", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "base adds g");
    let v = fx.delta();
    eprintln!("P7 ancestor {}", s(&v));
    assert_ne!(share(&v), Some(1.0));
    git(&d, &["branch", "-f", "feat", "main"]);
    let v = fx.delta();
    eprintln!("P7 equal {}", s(&v));
    assert_ne!(share(&v), Some(1.0));
    // ancestor of base, but base reverted the branch's work afterwards
    git(&d, &["checkout", "-q", "-b", "f2"]);
    write(&d, "src/b.py", H5);
    commit(&d, "f2 adds h");
    git(&d, &["checkout", "-q", "main"]);
    gc(&d, &["merge", "-q", "--no-ff", "-m", "m", "f2"]);
    git(&d, &["rm", "-q", "src/b.py"]);
    commit(&d, "base drops h");
    let v = fx.delta_of("main", "f2");
    eprintln!("P7 merged-ancestor-then-reverted {}", s(&v));
    assert_ne!(share(&v), Some(1.0));
}

/// P8: the round-6 merged-then-reverted shape with base given as a tag and as a bare sha.
#[test]
fn p8_base_tag_and_sha() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "add g");
    git(&d, &["checkout", "-q", "main"]);
    gc(&d, &["merge", "-q", "--no-ff", "-m", "merge feat", "feat"]);
    gc(&d, &["revert", "--no-edit", "-m", "1", "HEAD"]);
    git(&d, &["checkout", "-q", "feat"]);
    write(&d, "src/b.py", H5);
    commit(&d, "add h");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/b.py", H5);
    commit(&d, "take h");
    gc(&d, &["tag", "-a", "-m", "t", "v1"]);
    let sha = git_out(&d, &["rev-parse", "main"]);
    for b in ["v1", sha.as_str()] {
        let v = fx.delta_of(b, "feat");
        eprintln!("P8 base={b} {}", s(&v));
        assert_ne!(share(&v), Some(1.0), "P8 base {b}: {}", s(&v));
    }
}

/// P9: the clone's .git is byte-identical (every file's content hash, size and mode, and the listing) across a delta that runs
/// both merge-trees (B2 from F and the backstop), with a tag base too.
#[cfg(unix)]
#[test]
fn p9_git_dir_byte_identical() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "add g");
    git(&d, &["checkout", "-q", "main"]);
    gc(&d, &["merge", "-q", "--no-ff", "-m", "merge feat", "feat"]);
    gc(&d, &["revert", "--no-edit", "-m", "1", "HEAD"]);
    git(&d, &["checkout", "-q", "feat"]);
    write(&d, "src/b.py", H5);
    commit(&d, "add h");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/c.py", O5);
    commit(&d, "base conflicts nothing");
    gc(&d, &["tag", "v1"]);
    git(&d, &["gc", "-q"]); // packed objects: a write to a pack or the index would show
    fx.delta();
    fx.delta_of("v1", "feat");
    let snap = |p: &Path| -> String {
        let o = std::process::Command::new("bash")
            .arg("-c")
            .arg(format!("cd '{}' && find .git -printf '%p %s %m %y\\n' | sort && find .git -type f -print0 | sort -z | xargs -0 sha256sum", p.display()))
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    let before = snap(&d);
    let v1 = fx.delta();
    let v2 = fx.delta_of("v1", "feat");
    let after = snap(&d);
    eprintln!("P9 {} / {}", s(&v1), s(&v2));
    eprintln!("P9 b2 limitation present: {}", v1["evidence"]["limitations"].to_string().contains("merged_into_base_before_paths"));
    eprintln!("P9 snapshot lines: {}", before.lines().count());
    assert!(!before.is_empty() && before.contains("HEAD"), "snapshot empty: {before}");
    assert_eq!(before, after);
}

/// P9b: the same snapshot taken before the first delta, so the builds of the three refs (`--build-missing`) are inside it.
#[cfg(unix)]
#[test]
fn p9b_git_dir_byte_identical_including_builds() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "add g");
    git(&d, &["checkout", "-q", "main"]);
    gc(&d, &["merge", "-q", "--no-ff", "-m", "merge feat", "feat"]);
    gc(&d, &["revert", "--no-edit", "-m", "1", "HEAD"]);
    git(&d, &["checkout", "-q", "feat"]);
    write(&d, "src/b.py", H5);
    commit(&d, "add h");
    git(&d, &["checkout", "-q", "main"]);
    git(&d, &["gc", "-q"]);
    let snap = |p: &Path| -> String {
        let o = std::process::Command::new("bash")
            .arg("-c")
            .arg(format!("cd '{}' && find .git -printf '%p %s %m %y\n' | sort && find .git -type f -print0 | sort -z | xargs -0 sha256sum", p.display()))
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    let before = snap(&d);
    let v = run(fx.ws.path(), &["delta", "r", "--base", "main", "--head", "feat", "--build-missing"]);
    let after = snap(&d);
    eprintln!("P9b {} rebuilt={}", s(&v), v["data"]["rebuilt"]);
    assert!(before.contains("HEAD"));
    assert_eq!(before, after);
}

/// P10: the branch deletes A.py (one function g); base renames A.py to C.py and rewrites part of g (git still sees a rename,
/// token J < 0.9). rename/delete conflict at C.py: the merged tree keeps base's C.py, so the tree check passes and the per-row
/// check (path A.py) passes; only the share-level conflict check stands between this and 1.0.
#[test]
fn p10_rename_delete_conflict_only_the_share_level_check() {
    let g_old = "def g(items):\n    total = 0\n    for x in items:\n        total = total + x * 3 - len(items)\n        log(total, x, items)\n        if total > 100:\n            total = total // 2\n        record(total)\n        flush(items, total)\n    return total\n";
    let g_new = "def g(items):\n    total = 0\n    for x in items:\n        total = total + x * 3 - len(items)\n        log(total, x, items)\n        while total > 999 and items:\n            total -= items.pop() * 17\n        archive(total, 'z', items, None)\n        flush(items, total)\n    return total\n";
    let fx = Fx::new(&[("src/A.py", g_old), ("src/keep.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    git(&d, &["rm", "-q", "src/A.py"]);
    commit(&d, "delete A");
    git(&d, &["checkout", "-q", "main"]);
    git(&d, &["mv", "src/A.py", "src/C.py"]);
    write(&d, "src/C.py", g_new);
    commit(&d, "rename A to C, rewrite part of g");
    let mt = std::process::Command::new("git").arg("-C").arg(&d).args(["merge-tree", "--write-tree", "--name-only", "main", "feat"]).output().unwrap();
    eprintln!("P10 merge-tree rc={:?} out={}", mt.status.code(), String::from_utf8_lossy(&mt.stdout));
    let v = fx.delta();
    eprintln!("P10 {} removed={}", s(&v), v["data"]["branch"]["removed"]);
    assert!(file_on(&d, "main", "src/C.py").contains("def g"));
    assert_ne!(share(&v), Some(1.0), "P10 false 1.0: {}", s(&v));
    // round 7: any conflict turns every yes row unknown (`merge_conflict`), whatever its path
    let row = v["data"]["branch"]["removed"].as_array().unwrap().iter().find(|r| r["qualified_name"] == "src/A.py::g").unwrap().clone();
    assert_eq!(row["why"], "merge_conflict", "{row}");
}

/// Round 7, the B2 fallback: when the merge from F cannot be had (a test seam makes `--merge-base` fail), the broad rule stands
/// in: the merged-then-reverted shape still has its item, and so does merged-and-kept (the broad rule cannot tell kept work).
#[test]
fn r7_the_b2_fallback_stands_in_when_the_merge_from_f_fails() {
    let mk = |revert: bool| {
        let fx = Fx::new(&[("src/a.py", K5)]);
        let d = fx.d();
        git(&d, &["checkout", "-q", "-b", "feat"]);
        write(&d, "src/a.py", &format!("{K5}\n{G5}"));
        commit(&d, "add g");
        git(&d, &["checkout", "-q", "main"]);
        gc(&d, &["merge", "-q", "--no-ff", "-m", "merge feat", "feat"]);
        write(&d, "src/a.py", &if revert { K5.to_string() } else { format!("# base note\n{K5}\n{G5}") });
        commit(&d, "base goes on");
        git(&d, &["checkout", "-q", "feat"]);
        write(&d, "src/b.py", H5);
        commit(&d, "add h");
        git(&d, &["checkout", "-q", "main"]);
        write(&d, "src/b.py", H5);
        commit(&d, "take h");
        let o = common::cc(fx.ws.path()).env("COMPAREGRAPH_TEST_NO_MERGE_BASE", "1").args(["delta", "r", "--base", "main", "--head", "feat", "--refresh"]).output().unwrap();
        let v: Value = serde_json::from_slice(&o.stdout).unwrap();
        let kept = fx.delta(); // without the seam: the merge from F
        (v, kept)
    };
    let (seam, _) = mk(true);
    assert_eq!(seam["data"]["summary"]["unknown"]["by_why"]["merged_into_base_before"].as_i64(), Some(1), "{}", s(&seam));
    let (seam, tight) = mk(false);
    assert_eq!(seam["data"]["summary"]["unknown"]["by_why"]["merged_into_base_before"].as_i64(), Some(1), "the broad rule flags kept work: {}", s(&seam));
    assert_eq!(tight["data"]["summary"]["unknown"]["by_why"].get("merged_into_base_before"), None, "{}", s(&tight));
}
