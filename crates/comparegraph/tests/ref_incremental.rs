//! A ref build takes its previous graph from the nearest run of the repo (merge base, then base, then head) and records
//! `incremental_from`; whatever the previous run was, the answer (store_hash) equals a full build of the same ref.
mod common;
use common::{cc, git, ok};
use serde_json::Value;
use std::path::{Path, PathBuf};

const MIT: &str = "MIT License\n\nCopyright (c) 2026 S\n";

fn commit(dir: &Path, msg: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", msg]);
}
fn write(dir: &Path, rel: &str, body: &str) {
    let f = dir.join(rel);
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(f, body).unwrap();
}
fn rev(dir: &Path, r: &str) -> String {
    let o = std::process::Command::new("git").arg("-C").arg(dir).args(["rev-parse", r]).output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// Six python modules that call one another, so edges cross files.
fn module(i: usize, body: &str) -> String {
    format!("from m{} import f{}\n\ndef f{i}(x):\n    {body}\n    return f{}(x) + {i}\n", (i + 1) % 6, (i + 1) % 6, (i + 1) % 6)
}

struct Fx {
    ws: tempfile::TempDir,
    base: tempfile::TempDir,
}
impl Fx {
    fn new() -> Fx {
        let (ws, base) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let d = base.path().join("r");
        std::fs::create_dir_all(&d).unwrap();
        git(&d, &["init", "-q", "-b", "main"]);
        write(&d, "LICENSE", MIT);
        for i in 0..6 {
            write(&d, &format!("m{i}.py"), &module(i, "y = x * 2"));
        }
        commit(&d, "c1");
        ok(ws.path(), &["add", d.to_str().unwrap(), "--name", "r"]);
        Fx { ws, base }
    }
    fn clone(&self) -> PathBuf {
        self.base.path().join("r")
    }
    fn root(&self) -> &Path {
        self.ws.path()
    }
    fn build(&self, args: &[&str]) -> Value {
        let mut a = vec!["build", "r"];
        a.extend_from_slice(args);
        ok(self.root(), &a)
    }
    fn build_with_prev(&self, prev: i64, args: &[&str]) -> Value {
        let mut a = vec!["build", "r"];
        a.extend_from_slice(args);
        let o = cc(self.root()).args(&a).env("COMPAREGRAPH_PREV_RUN", prev.to_string()).output().unwrap();
        serde_json::from_slice(&o.stdout).unwrap()
    }
}
fn run_of(v: &Value) -> i64 {
    v["data"]["run"].as_i64().unwrap()
}
fn hash_of(v: &Value) -> String {
    v["data"]["store_hash"].as_str().unwrap().to_string()
}

#[test]
fn an_incremental_ref_build_equals_a_full_build_of_the_same_ref() {
    let fx = Fx::new();
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "m2.py", &module(2, "y = x * 3"));
    write(&d, "m6.py", "def extra():\n    return 1\n");
    commit(&d, "feat edits m2 and adds m6");
    git(&d, &["checkout", "-q", "main"]);

    let m = fx.build(&["--ref", "main"]);
    assert!(m["data"]["incremental_from"].is_null(), "nothing to start from: a full build");
    let inc = fx.build(&["--ref", "feat"]);
    assert_eq!(inc["data"]["incremental_from"].as_i64(), Some(run_of(&m)), "{inc}");
    assert!(inc["data"]["edges_copied"].as_i64().unwrap() > 0, "an incremental build copies the edges of untouched files: {inc}");
    let db = rusqlite::Connection::open(fx.root().join(".comparegraph/comparegraph.db")).unwrap();
    let rec: Option<i64> = db.query_row("SELECT incremental_from FROM run WHERE id=?1", [run_of(&inc)], |r| r.get(0)).unwrap();
    assert_eq!(rec, Some(run_of(&m)));

    let full = fx.build(&["--ref", "feat", "--full"]);
    assert!(full["data"]["incremental_from"].is_null());
    assert_eq!(full["data"]["edges_copied"], 0);
    assert_eq!(hash_of(&inc), hash_of(&full), "incremental and full builds of one ref must agree");
}

#[test]
fn a_wrong_previous_run_falls_back_or_matches_never_changes_the_answer() {
    let fx = Fx::new();
    let d = fx.clone();
    // an unrelated history: other files, other symbols
    git(&d, &["checkout", "-q", "-b", "other"]);
    for i in 0..6 {
        std::fs::remove_file(d.join(format!("m{i}.py"))).unwrap();
    }
    write(&d, "z.py", "def zed():\n    return 0\n");
    commit(&d, "unrelated");
    git(&d, &["checkout", "-q", "main"]);
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "m4.py", &module(4, "y = x * 5"));
    commit(&d, "feat");
    git(&d, &["checkout", "-q", "main"]);

    let other = fx.build(&["--ref", "other"]);
    let full = fx.build(&["--ref", "feat", "--full"]);
    // the same commit under another name, forced to start from the unrelated run
    git(&d, &["branch", "feat2", "feat"]);
    let forced = fx.build_with_prev(run_of(&other), &["--ref", "feat2"]);
    assert_eq!(forced["data"]["incremental_from"].as_i64(), Some(run_of(&other)));
    assert_eq!(hash_of(&forced), hash_of(&full));
    // and from a run of the head (working tree) too
    let head = fx.build(&[]);
    git(&d, &["branch", "feat3", "feat"]);
    let from_head = fx.build_with_prev(run_of(&head), &["--ref", "feat3"]);
    assert_eq!(hash_of(&from_head), hash_of(&full));
}

#[test]
fn the_nearest_run_is_the_previous_the_merge_base_before_the_base_ref() {
    let fx = Fx::new();
    let d = fx.clone();
    let c1 = rev(&d, "HEAD");
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "m1.py", &module(1, "y = x * 7"));
    commit(&d, "feat");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "m3.py", &module(3, "y = x * 9"));
    commit(&d, "main moves on");
    let base = fx.build(&["--ref", "main"]); // distance to feat: 1 (main) + 1 (feat) = 2
    let mb = fx.build(&["--ref", &c1]); // the merge base: distance 1 (the branch's own commit)
    let feat = fx.build(&["--ref", "feat"]);
    assert_eq!(feat["data"]["incremental_from"].as_i64(), Some(run_of(&mb)), "the merge base's run is nearer than the base's");
    assert_ne!(run_of(&mb), run_of(&base));
    let full = fx.build(&["--ref", "feat", "--full"]);
    assert_eq!(hash_of(&feat), hash_of(&full));
}

#[test]
fn the_reason_an_incremental_build_fell_back_is_recorded_on_the_run() {
    let fx = Fx::new();
    let d = fx.clone();
    write(&d, "pkg/__init__.py", "X = 1\n");
    commit(&d, "main adds a package file");
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "pkg/__init__.py", "X = 2\n");
    write(&d, "m2.py", &module(2, "y = x * 3"));
    commit(&d, "feat changes the package file");
    git(&d, &["checkout", "-q", "main"]);
    let m = fx.build(&["--ref", "main"]);
    assert!(m["data"]["incremental_fallback"].is_null());
    let f = fx.build(&["--ref", "feat"]);
    assert_eq!(f["data"]["incremental_from"].as_i64(), Some(run_of(&m)));
    let why = f["data"]["incremental_fallback"].as_str().unwrap_or_else(|| panic!("no reason recorded: {f}"));
    assert!(why.contains("__init__.py"), "{why}");
    assert_eq!(f["data"]["edges_copied"], 0);
    let db = rusqlite::Connection::open(fx.root().join(".comparegraph/comparegraph.db")).unwrap();
    let rec: Option<String> = db.query_row("SELECT incremental_fallback FROM run WHERE id=?1", [run_of(&f)], |r| r.get(0)).unwrap();
    assert_eq!(rec.as_deref(), Some(why));
    // the answer is still the full answer
    assert_eq!(hash_of(&f), hash_of(&fx.build(&["--ref", "feat", "--full"])));
}
