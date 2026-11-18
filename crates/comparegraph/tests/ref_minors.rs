//! Review round 2, the minor findings on `build --ref`, `delta`, `gc --refs`, `repos --stale` and `decide --ref`: each test is
//! red on the code before its fix (feat/plan-14). Fixture repos are real git repositories in temp dirs, no network.
mod common;
use common::{err, git, ok, run};
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const MIT: &str = "MIT License\n\nCopyright (c) 2026 S\n";
const FOO: &str = "def foo():\n    return 1\n";
const BAR: &str = "def bar(x):\n    y = x * 2\n    return y + 1\n";

fn commit(dir: &Path, msg: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", msg]);
}
fn write(dir: &Path, rel: &str, body: &str) {
    let f = dir.join(rel);
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(f, body).unwrap();
}
fn git_out(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}");
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}
fn git_in(dir: &Path, args: &[&str], stdin: &[u8]) -> String {
    let mut c = Command::new("git").arg("-C").arg(dir).args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(stdin).unwrap();
    let o = c.wait_with_output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}
fn limits(v: &Value) -> String {
    v["evidence"]["limitations"].to_string()
}

struct Fx {
    ws: tempfile::TempDir,
    base: tempfile::TempDir,
}
impl Fx {
    /// main holds `files` (and LICENSE); feat (one commit ahead) adds src/a.py::bar; HEAD left on main; registered as `r`.
    fn new(files: &[(&str, &str)]) -> Fx {
        let (ws, base) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let d = base.path().join("r");
        std::fs::create_dir_all(&d).unwrap();
        git(&d, &["init", "-q", "-b", "main"]);
        write(&d, "LICENSE", MIT);
        write(&d, "src/a.py", FOO);
        for (p, c) in files {
            write(&d, p, c);
        }
        commit(&d, "c1");
        git(&d, &["checkout", "-q", "-b", "feat"]);
        write(&d, "src/a.py", &format!("{FOO}\n{BAR}"));
        commit(&d, "feat");
        git(&d, &["checkout", "-q", "main"]);
        ok(ws.path(), &["add", d.to_str().unwrap(), "--name", "r"]);
        Fx { ws, base }
    }
    fn clone(&self) -> PathBuf {
        self.base.path().join("r")
    }
    fn root(&self) -> &Path {
        self.ws.path()
    }
    fn tree(&self, r: &str) -> PathBuf {
        self.root().join(".comparegraph/refs/r").join(git_out(&self.clone(), &["rev-parse", r]))
    }
    fn log(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root().join(".comparegraph/actions.jsonl")).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }
    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.root().join(".comparegraph/comparegraph.db")).unwrap()
    }
}

/// a body well over 1000 bytes that still parses
fn big_py() -> String {
    (0..200).map(|i| format!("V{i} = {i} * 2\n")).collect()
}

/// MINOR: a blob over the build's `--max-file-bytes` is streamed past, never written into the tree, and said so; a later build
/// with a larger limit extracts the tree again and reads it.
#[test]
fn a_blob_over_the_size_limit_is_never_extracted_and_a_larger_limit_extracts_it() {
    let fx = Fx::new(&[("src/big.py", &big_py())]);
    let v = ok(fx.root(), &["build", "r", "--ref", "main", "--max-file-bytes", "1000"]);
    assert!(limits(&v).contains("skipped_oversize"), "{}", limits(&v));
    assert!(!fx.tree("main").join("src/big.py").exists(), "an oversize blob was written into the tree");
    assert!(fx.tree("main").join("src/a.py").is_file());
    ok(fx.root(), &["build", "r", "--ref", "main"]);
    assert!(fx.tree("main").join("src/big.py").is_file(), "a larger limit must extract the blob");
}

/// MINOR: a submodule entry is not silent: the run carries `submodule_skipped` with the count.
#[test]
fn submodule_entries_are_a_limitation_with_their_count() {
    let fx = Fx::new(&[]);
    let d = fx.clone();
    let main = git_out(&d, &["rev-parse", "main"]);
    let mut entries = Command::new("git").arg("-C").arg(&d).args(["ls-tree", "main"]).output().unwrap().stdout;
    for s in ["sub1", "sub2"] {
        entries.extend_from_slice(format!("160000 commit {main}\t{s}\n").as_bytes());
    }
    let tree = git_in(&d, &["mktree"], &entries);
    let c = git_in(&d, &["-c", "user.email=t@t", "-c", "user.name=t", "commit-tree", &tree, "-p", "main", "-m", "subs"], b"");
    git(&d, &["update-ref", "refs/heads/subs", &c]);
    let v = ok(fx.root(), &["build", "r", "--ref", "subs"]);
    let l = limits(&v);
    assert!(l.contains("submodule_skipped") && l.contains("2 submodule"), "{l}");
}

/// MINOR: a tree folder that is gone under its `.complete` marker is extracted again, never read as an empty tree.
#[test]
fn a_tree_folder_gone_under_its_marker_is_extracted_again() {
    let fx = Fx::new(&[]);
    ok(fx.root(), &["build", "r", "--ref", "main"]);
    std::fs::remove_dir_all(fx.tree("main")).unwrap();
    let v = ok(fx.root(), &["build", "r", "--ref", "main", "--full"]);
    assert!(v["data"]["nodes"].as_i64().unwrap() > 0, "{v}");
    assert!(fx.tree("main").join("src/a.py").is_file());
}

/// MINOR: `gc --refs` never removes a tree while `<commit>.lock` is held: the run is kept and listed, and a later gc (the lock
/// gone) removes run, marker and tree.
#[test]
fn gc_refs_skips_a_tree_whose_lock_is_held() {
    let fx = Fx::new(&[]);
    ok(fx.root(), &["build", "r"]);
    ok(fx.root(), &["build", "r", "--ref", "feat"]);
    let tree = fx.tree("feat");
    let commit = tree.file_name().unwrap().to_string_lossy().into_owned();
    let lock = tree.with_file_name(format!("{commit}.lock"));
    let marker = tree.with_file_name(format!("{commit}.complete"));
    git(&fx.clone(), &["branch", "-q", "-D", "feat"]);
    std::fs::write(&lock, std::process::id().to_string()).unwrap(); // a live holder: this test
    let v = ok(fx.root(), &["gc", "--refs", "--apply", "--by", "t"]);
    assert!(tree.is_dir() && marker.is_file(), "a locked tree was removed: {v}");
    assert!(limits(&v).contains("lock"), "{}", limits(&v));
    assert_eq!(fx.db().query_row("SELECT COUNT(*) FROM run WHERE ref='feat'", [], |r| r.get::<_, i64>(0)).unwrap(), 1, "{v}");
    std::fs::remove_file(&lock).unwrap();
    ok(fx.root(), &["gc", "--refs", "--apply", "--by", "t"]);
    assert!(!tree.exists() && !marker.exists(), "the tree and its marker stay after the lock went");
    assert_eq!(fx.db().query_row("SELECT COUNT(*) FROM run WHERE ref='feat'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
}

/// MINOR: `delta` carries each side's run limitations (here a file over max_file_bytes) into its own, and refuses sides read
/// with different file limits.
#[test]
fn delta_carries_side_limitations_and_refuses_mixed_file_limits() {
    let fx = Fx::new(&[("src/big.py", &big_py())]);
    ok(fx.root(), &["build", "r", "--ref", "main", "--max-file-bytes", "1000"]);
    ok(fx.root(), &["build", "r", "--ref", "feat"]);
    let e = run(fx.root(), &["delta", "r", "--base", "main", "--head", "feat"]);
    assert_eq!(e["ok"], false, "sides read with different file limits must refuse: {e}");
    assert!(e["error"]["message"].as_str().unwrap().contains("max_file_bytes"), "{e}");
    ok(fx.root(), &["build", "r", "--ref", "feat", "--max-file-bytes", "1000"]);
    let v = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat"]);
    let l = limits(&v);
    assert!(l.contains("skipped_oversize") && l.contains("base") && l.contains("head"), "{l}");
}

/// MINOR: the `gc-refs` action-log line carries the id high-water mark, and a store restored from an older backup takes it
/// from the log: an id gc removed is never handed out again.
#[test]
fn the_id_high_water_mark_is_in_the_gc_line_and_survives_a_restore() {
    let fx = Fx::new(&[]);
    ok(fx.root(), &["build", "r"]);
    let out = tempfile::tempdir().unwrap();
    let b = ok(fx.root(), &["backup", "--out", out.path().to_str().unwrap(), "--by", "t"]);
    let file = b["data"]["file"].as_str().unwrap().to_string();
    let feat_run = ok(fx.root(), &["build", "r", "--ref", "feat"])["data"]["run"].as_i64().unwrap();
    git(&fx.clone(), &["branch", "-q", "-D", "feat"]);
    ok(fx.root(), &["gc", "--refs", "--apply", "--by", "t"]);
    let gc = fx.log().into_iter().rev().find(|e| e["verb"] == "gc-refs").unwrap();
    assert!(gc["inputs"]["id_hwm"]["run"].as_i64().unwrap_or(0) >= feat_run, "{gc}");
    ok(fx.root(), &["restore-store", &file, "--apply", "--by", "t"]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "other"]);
    write(&d, "src/o.py", "def other(x):\n    return x - 1\n");
    commit(&d, "other");
    git(&d, &["checkout", "-q", "main"]);
    let r = ok(fx.root(), &["build", "r", "--ref", "other"])["data"]["run"].as_i64().unwrap();
    assert!(r > feat_run, "run id {r} handed out again after a restore (gc removed up to {feat_run})");
}

/// MINOR: `delta` appends a `delta-record` line only when the value it keeps changes.
#[test]
fn delta_records_its_summary_only_when_it_changes() {
    let fx = Fx::new(&[]);
    let records = |fx: &Fx| fx.log().iter().filter(|e| e["verb"] == "delta-record").count();
    ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--build-missing"]);
    ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat"]);
    ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat"]);
    assert_eq!(records(&fx), 1, "an unchanged summary was recorded again");
    let d = fx.clone();
    git(&d, &["checkout", "-q", "feat"]);
    write(&d, "src/b.py", "def baz(q):\n    r = q + 7\n    return r * 3\n");
    commit(&d, "feat moves on");
    git(&d, &["checkout", "-q", "main"]);
    ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--refresh"]);
    assert_eq!(records(&fx), 2);
    ok(fx.root(), &["governance", "verify"]);
}

/// MINOR: a `git diff` that fails (here a tree object is missing from the clone) is a limitation, not empty file lists.
#[test]
fn a_failed_git_diff_is_a_limitation_not_empty_lists() {
    let fx = Fx::new(&[]);
    ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--build-missing"]);
    let d = fx.clone();
    let oid = git_out(&d, &["rev-parse", "feat:src"]);
    let obj = d.join(".git/objects").join(&oid[..2]).join(&oid[2..]);
    let mut p = std::fs::metadata(&obj).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    p.set_readonly(false);
    std::fs::set_permissions(&obj, p).unwrap();
    std::fs::remove_file(&obj).unwrap();
    let v = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat"]);
    // round 4 (B3): the touched set comes from the two trees (`git ls-tree`), so the failure is named as the listing that failed
    assert!(limits(&v).contains("the files the branch touched"), "a failed tree listing answered empty lists silently: {}", limits(&v));
    assert_eq!(v["data"]["summary"]["files_branch_touched"], Value::Null, "{}", v["data"]["summary"]);
}

/// MINOR: `repos --stale` pages over the repos that have a stale ref run, not over all repos.
#[test]
fn repos_stale_pages_over_the_stale_repos_only() {
    let ws = tempfile::tempdir().unwrap();
    let base = tempfile::tempdir().unwrap();
    for name in ["a", "b", "c"] {
        let d = base.path().join(name);
        std::fs::create_dir_all(&d).unwrap();
        git(&d, &["init", "-q", "-b", "main"]);
        write(&d, "LICENSE", MIT);
        write(&d, "src/a.py", FOO);
        commit(&d, "c1");
        ok(ws.path(), &["add", d.to_str().unwrap(), "--name", name]);
        ok(ws.path(), &["build", name, "--ref", "main"]);
    }
    let c = base.path().join("c");
    git(&c, &["branch", "-q", "gone"]);
    ok(ws.path(), &["build", "c", "--ref", "gone"]);
    git(&c, &["branch", "-q", "-D", "gone"]);
    let v = ok(ws.path(), &["repos", "--stale", "--limit", "1"]);
    let names: Vec<&str> = v["data"]["items"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["c"], "{v}");
    assert_eq!((v["page"]["total"].as_i64(), v["page"]["next"].is_null()), (Some(1), true), "{v}");
}

/// MINOR: `decide --ref <ref> --commit <sha>` refuses a commit the ref does not reach (or that the clone does not hold).
#[test]
fn a_branch_verdict_needs_a_commit_the_ref_reaches() {
    let fx = Fx::new(&[]);
    let d = fx.clone();
    write(&d, "src/m.py", BAR);
    commit(&d, "main moves on");
    let on_main = git_out(&d, &["rev-parse", "main"]);
    let feat = git_out(&d, &["rev-parse", "feat"]);
    let args = |c: &str| vec!["decide".to_string(), "r".into(), "park".into(), "--why".into(), "w".into(), "--by".into(), "t".into(), "--ref".into(), "feat".into(), "--commit".into(), c.to_string()];
    let call = |c: &str| run(fx.root(), &args(c).iter().map(String::as_str).collect::<Vec<_>>());
    let e = call(&on_main);
    assert_eq!((e["ok"].clone(), e["error"]["code"].clone()), (Value::Bool(false), Value::from("invalid_arguments")), "a commit feat does not reach was accepted: {e}");
    err(fx.root(), &args("abcdef1234").iter().map(String::as_str).collect::<Vec<_>>(), "invalid_arguments");
    assert_eq!(call(&feat)["ok"], true);
    assert_eq!(call(&git_out(&d, &["rev-parse", "feat~1"]))["ok"], true, "an ancestor of the ref is reached");
}

// ---------------------------------------------------------------- review round 3, the minors

/// a 30-line body named `name`, with a tweak on its return
fn long_fn(name: &str, tweak: &str) -> String {
    let mut s = format!("def {name}(items):\n    total = 0\n");
    for i in 0..30 {
        s.push_str(&format!("    total = total + items[{i}] * {} - len(items) + k{i}\n", i + 2));
    }
    s.push_str(&format!("    return total{tweak}\n"));
    s
}

/// MINOR (round 3): the near rows of the summary are capped at `--max-nodes`, and the cut says `truncated`.
#[test]
fn the_near_rows_are_capped_at_max_nodes() {
    let fx = Fx::new(&[]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "feat"]);
    write(&d, "src/n.py", &format!("{}\n{}", long_fn("p1", ""), long_fn("p2", "")));
    commit(&d, "feat adds p1 and p2");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/o.py", &format!("{}\n{}", long_fn("p1", " + 1"), long_fn("p2", " + 1")));
    commit(&d, "main has near copies of both");
    let v = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--refresh", "--max-nodes", "1"]);
    let near = &v["data"]["summary"]["near_on_base"];
    assert_eq!((near["count"].as_i64(), near["rows"].as_array().map(Vec::len)), (Some(2), Some(1)), "{near}");
    assert!(limits(&v).contains("summary.near_on_base.rows"), "{}", limits(&v));
}

/// MINOR (round 3): `decide --summary-from-delta` stores the last delta of the commit only when that delta used the base the
/// decision names (`--base`): a delta against another base is refused, and so is a call that names none.
#[test]
fn a_summary_from_delta_needs_the_base_the_delta_used() {
    let fx = Fx::new(&[]);
    let d = fx.clone();
    git(&d, &["branch", "-q", "other", "main"]);
    let sha = git_out(&d, &["rev-parse", "feat"]);
    ok(fx.root(), &["delta", "r", "--base", "other", "--head", "feat", "--build-missing"]);
    let call = |base: Option<&str>| {
        let mut a = vec!["decide", "r", "park", "--why", "w", "--by", "t", "--ref", "feat", "--commit", sha.as_str(), "--summary-from-delta"];
        if let Some(b) = base {
            a.extend(["--base", b]);
        }
        run(fx.root(), &a)
    };
    let e = call(Some("main"));
    assert_eq!((e["ok"].clone(), e["error"]["code"].clone()), (Value::Bool(false), Value::from("invalid_arguments")), "a delta against `other` was stored for --base main: {e}");
    assert!(e["error"]["message"].as_str().unwrap().contains("other"), "{e}");
    assert_eq!(call(None)["error"]["code"], "invalid_arguments", "--summary-from-delta without --base must be refused");
    let v = call(Some("other"));
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["data"]["summary"]["base_ref"], "other", "{v}");
}

/// M2 (round 4): the kept delta names the engine, provider and model that computed it; `decide --summary-from-delta` refuses
/// one kept under another model (here the kept value is edited to say so).
#[test]
fn a_summary_from_another_model_is_refused() {
    let fx = Fx::new(&[]);
    let sha = git_out(&fx.clone(), &["rev-parse", "feat"]);
    let v = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--build-missing"]);
    assert!(v["ok"].as_bool().unwrap());
    let key = format!("delta:r:{sha}");
    let kept: String = fx.db().query_row("SELECT value FROM meta WHERE key=?1", [&key], |r| r.get(0)).unwrap();
    let mut j: Value = serde_json::from_str(&kept).unwrap();
    assert!(j["engine"].is_string() && j["model"].is_string() && j["provider_version"].is_string(), "{j}");
    j["model"] = Value::from("an-older-model");
    fx.db().execute("UPDATE meta SET value=?1 WHERE key=?2", [j.to_string(), key]).unwrap();
    let e = run(fx.root(), &["decide", "r", "park", "--why", "w", "--by", "t", "--ref", "feat", "--commit", &sha, "--summary-from-delta", "--base", "main"]);
    assert_eq!(e["error"]["code"], "invalid_arguments", "{e}");
    assert!(e["error"]["message"].as_str().unwrap().contains("an-older-model"), "{e}");
}

/// Round 4 minor: a `--base` that no longer resolves cannot be checked against the kept delta: refused.
#[test]
fn a_summary_from_delta_against_a_gone_base_is_refused() {
    let fx = Fx::new(&[]);
    let d = fx.clone();
    git(&d, &["branch", "-q", "other", "main"]);
    let sha = git_out(&d, &["rev-parse", "feat"]);
    ok(fx.root(), &["delta", "r", "--base", "other", "--head", "feat", "--build-missing"]);
    git(&d, &["branch", "-q", "-D", "other"]);
    let e = run(fx.root(), &["decide", "r", "park", "--why", "w", "--by", "t", "--ref", "feat", "--commit", &sha, "--summary-from-delta", "--base", "other"]);
    assert_eq!(e["error"]["code"], "invalid_arguments", "{e}");
    assert!(e["error"]["message"].as_str().unwrap().contains("no longer names"), "{e}");
}
