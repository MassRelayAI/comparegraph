//! Round-8 review probes of the delta model (feat/plan-14 at 31936bb): the step-past walk stopped too early (two back-merges,
//! a conflict resolution, a base commit on top, unrelated histories joined twice). The round-8 ruling makes the item hold by
//! construction: a branch base ever merged before, joined histories or a shallow clone always carry one unknown item.
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

const C5: &str = "def c(items):\n    r = 4\n    for u in items:\n        r = r * u - 13 + len(items)\n        shout(r, u, items)\n    return r\n";
const N5: &str = "def n(items):\n    e = 6\n    for v in items:\n        e = e - v * 17 + len(items)\n        whisper(e, v, items)\n    return e\n";

/// Q3: base merges feat (M1), base commits on top (B2), feat back-merges main (reaches B2), base reverts g, takes h.
#[test]
fn q3_back_merge_after_base_commit_on_top() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "add g");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/m.py", O5);
    commit(&d, "base work");
    gc(&d, &["merge", "-q", "--no-ff", "-m", "merge feat", "feat"]);
    write(&d, "src/n.py", N5);
    commit(&d, "base on top");
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
    eprintln!("Q3 {}", s(&v));
    assert!(file_on(&d, "feat", "src/a.py").contains("def g") && !file_on(&d, "main", "src/a.py").contains("def g"));
    assert_ne!(share(&v), Some(1.0), "Q3 false 1.0: {}", s(&v));
    assert_eq!(b2_reason(&v).as_deref(), Some("merged_before"), "{}", s(&v));
}

/// Q1: two back-merges: M1 (g), base commit B2, feat back-merges, feat adds c, M2, feat back-merges M2, base reverts g, takes h.
#[test]
fn q1_two_back_merges() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "add g");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/m.py", O5);
    commit(&d, "base work");
    gc(&d, &["merge", "-q", "--no-ff", "-m", "merge feat 1", "feat"]);
    git(&d, &["checkout", "-q", "feat"]);
    gc(&d, &["merge", "-q", "--no-ff", "-m", "back-merge 1", "main"]);
    write(&d, "src/c.py", C5);
    commit(&d, "add c");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/n.py", N5);
    commit(&d, "base between");
    gc(&d, &["merge", "-q", "--no-ff", "-m", "merge feat 2", "feat"]);
    git(&d, &["checkout", "-q", "feat"]);
    gc(&d, &["merge", "-q", "--no-ff", "-m", "back-merge 2", "main"]);
    write(&d, "src/b.py", H5);
    commit(&d, "add h");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/a.py", K5);
    commit(&d, "base reverts g");
    write(&d, "src/b.py", H5);
    commit(&d, "take h");
    let v = fx.delta();
    eprintln!("Q1 {}", s(&v));
    assert!(file_on(&d, "feat", "src/a.py").contains("def g") && !file_on(&d, "main", "src/a.py").contains("def g"));
    assert_ne!(share(&v), Some(1.0), "Q1 false 1.0: {}", s(&v));
    assert_eq!(b2_reason(&v).as_deref(), Some("merged_before"), "{}", s(&v));
}

/// Q1b: two back-merges with no base commit between the merges (control).
#[test]
fn q1b_two_back_merges_no_commit_between() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "add g");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/m.py", O5);
    commit(&d, "base work");
    gc(&d, &["merge", "-q", "--no-ff", "-m", "merge feat 1", "feat"]);
    git(&d, &["checkout", "-q", "feat"]);
    gc(&d, &["merge", "-q", "--no-ff", "-m", "back-merge 1", "main"]);
    write(&d, "src/c.py", C5);
    commit(&d, "add c");
    git(&d, &["checkout", "-q", "main"]);
    gc(&d, &["merge", "-q", "--no-ff", "-m", "merge feat 2", "feat"]);
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
    eprintln!("Q1b {}", s(&v));
    assert_ne!(share(&v), Some(1.0), "Q1b false 1.0: {}", s(&v));
}

/// Q2: back-merge with a conflict resolution (k's line), then base reverts g and takes head's resolution and h.
#[test]
fn q2_back_merge_with_conflict_resolution() {
    let k_base = K5.replace("s = 2", "s = 3");
    let k_feat = K5.replace("s = 2", "s = 5");
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/g.py", G5);
    commit(&d, "add g");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/m.py", O5);
    commit(&d, "base work");
    gc(&d, &["merge", "-q", "--no-ff", "-m", "merge feat", "feat"]);
    write(&d, "src/a.py", &k_base);
    commit(&d, "base edits k");
    git(&d, &["checkout", "-q", "feat"]);
    write(&d, "src/a.py", &k_feat);
    commit(&d, "feat edits k");
    let _ = std::process::Command::new("git").arg("-C").arg(&d).args(["-c", "user.email=t@t", "-c", "user.name=t", "merge", "-q", "--no-ff", "-m", "back-merge", "main"]).output();
    write(&d, "src/a.py", &k_feat);
    commit(&d, "back-merge resolved");
    write(&d, "src/b.py", H5);
    commit(&d, "add h");
    git(&d, &["checkout", "-q", "main"]);
    git(&d, &["rm", "-q", "src/g.py"]);
    commit(&d, "base drops g");
    write(&d, "src/a.py", &k_feat);
    write(&d, "src/b.py", H5);
    commit(&d, "take k and h");
    let v = fx.delta();
    eprintln!("Q2 {}", s(&v));
    assert!(file_on(&d, "feat", "src/g.py").contains("def g") && !file_on(&d, "main", "src/g.py").contains("def g"));
    assert_ne!(share(&v), Some(1.0), "Q2 false 1.0: {}", s(&v));
    assert_eq!(b2_reason(&v).as_deref(), Some("merged_before"), "{}", s(&v));
}

/// Q5: unrelated histories joined twice, base commit between, feat back-merges both joins, base drops g, takes h.
#[test]
fn q5_unrelated_joined_twice() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "--orphan", "feat"]);
    write(&d, "src/g.py", G5);
    commit(&d, "feat root: add g");
    git(&d, &["checkout", "-q", "main"]);
    gc(&d, &["merge", "-q", "--no-ff", "--allow-unrelated-histories", "-m", "join 1", "feat"]);
    git(&d, &["checkout", "-q", "feat"]);
    gc(&d, &["merge", "-q", "--no-ff", "-m", "back 1", "main"]);
    write(&d, "src/c.py", C5);
    commit(&d, "add c");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/n.py", N5);
    commit(&d, "base between");
    gc(&d, &["merge", "-q", "--no-ff", "-m", "join 2", "feat"]);
    git(&d, &["checkout", "-q", "feat"]);
    gc(&d, &["merge", "-q", "--no-ff", "-m", "back 2", "main"]);
    write(&d, "src/b.py", H5);
    commit(&d, "add h");
    git(&d, &["checkout", "-q", "main"]);
    git(&d, &["rm", "-q", "src/g.py"]);
    commit(&d, "base drops g");
    write(&d, "src/b.py", H5);
    commit(&d, "take h");
    let v = fx.delta();
    eprintln!("Q5 {}", s(&v));
    assert!(file_on(&d, "feat", "src/g.py").contains("def g") && !file_on(&d, "main", "src/g.py").contains("def g"));
    assert_ne!(share(&v), Some(1.0), "Q5 false 1.0: {}", s(&v));
    assert_eq!(b2_reason(&v).as_deref(), Some("merged_before"), "{}", s(&v));
}

/// Q6: octopus on base (feat + other), feat back-merges, base commits on top first, reverts g, takes h.
#[test]
fn q6_octopus_then_back_merge() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "add g");
    git(&d, &["checkout", "-q", "-b", "other", "main"]);
    write(&d, "src/o.py", O5);
    commit(&d, "add o");
    git(&d, &["checkout", "-q", "main"]);
    gc(&d, &["merge", "-q", "--no-ff", "-m", "octopus", "other", "feat"]);
    git(&d, &["checkout", "-q", "feat"]);
    gc(&d, &["merge", "-q", "--no-ff", "-m", "back", "main"]);
    write(&d, "src/b.py", H5);
    commit(&d, "add h");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/a.py", K5);
    commit(&d, "base reverts g");
    write(&d, "src/b.py", H5);
    commit(&d, "take h");
    let v = fx.delta();
    eprintln!("Q6 {}", s(&v));
    assert_ne!(share(&v), Some(1.0), "Q6 false 1.0: {}", s(&v));
}

/// The reason of the one `merged_into_base_before` item, if the delta has one (exactly one, never more).
fn b2_reason(v: &Value) -> Option<String> {
    let items: Vec<&Value> = v["data"]["branch"]["residue"].as_array()?.iter().filter(|r| r["why"] == "merged_into_base_before").collect();
    assert!(items.len() <= 1, "more than one merged_into_base_before item: {items:?}");
    items.first().map(|r| r["reason"].as_str().unwrap_or("").to_string())
}

/// Control: a branch base never merged (an ordinary lane forked from base) has no `merged_into_base_before` item.
#[test]
fn r8_a_branch_never_merged_into_base_has_no_such_item() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    write(&d, "src/m.py", O5);
    commit(&d, "base before the fork");
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/b.py", H5);
    commit(&d, "add h");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/n.py", N5);
    commit(&d, "base after the fork");
    let v = fx.delta();
    assert_eq!(b2_reason(&v), None, "{}", s(&v));
}
