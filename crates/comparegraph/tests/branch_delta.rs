//! CG-BR1 (`build --ref`) and CG-DELTA (`delta`): fixture repos are real git repositories in temp dirs, no network.
mod common;
use common::{err, git, ok, run, PACK};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
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
fn git_out(dir: &Path, args: &[&str]) -> String {
    let o = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}");
    String::from_utf8_lossy(&o.stdout).into_owned()
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
    fn clone(&self) -> PathBuf {
        self.base.path().join("r")
    }
    fn root(&self) -> &Path {
        self.ws.path()
    }
    fn build_ref(&self, r: &str) -> Value {
        ok(self.root(), &["build", "r", "--ref", r])
    }
    /// delta with the missing and the moved refs (and the merge base) built on the way
    fn delta(&self, base: &str, head: &str) -> Value {
        ok(self.root(), &["delta", "r", "--base", base, "--head", head, "--refresh"])
    }
    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.root().join(".comparegraph/comparegraph.db")).unwrap()
    }
    fn family(&self) {
        let pack = self.base.path().join("pack");
        write(&pack, "rows.md", PACK);
        ok(self.root(), &["family", "add", pack.to_str().unwrap()]);
    }
}
fn sha(v: &Value) -> String {
    hex::encode(Sha256::digest(v.to_string().as_bytes()))
}
fn qns(rows: &Value) -> Vec<String> {
    rows.as_array().unwrap().iter().map(|r| r["qualified_name"].as_str().unwrap().to_string()).collect()
}
fn names_of_run(fx: &Fx, run: i64) -> Vec<String> {
    let db = fx.db();
    let mut st = db.prepare("SELECT name FROM node WHERE run_id=?1 AND kind='function' ORDER BY name").unwrap();
    st.query_map([run], |r| r.get::<_, String>(0)).unwrap().map(|r| r.unwrap()).collect()
}

const FOO: &str = "def foo():\n    return 1\n";
const BAR: &str = "def bar(x):\n    y = x * 2\n    return y + 1\n";

// ---------------------------------------------------------------- CG-BR1

#[test]
fn two_refs_give_two_runs_with_different_symbol_sets() {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{FOO}\n{BAR}"));
    commit(&d, "feat adds bar");
    git(&d, &["checkout", "-q", "main"]);

    let m = fx.build_ref("main");
    let f = fx.build_ref("feat");
    let (rm, rf) = (m["data"]["run"].as_i64().unwrap(), f["data"]["run"].as_i64().unwrap());
    assert_ne!(rm, rf);
    assert_eq!(names_of_run(&fx, rm), vec!["foo"]);
    assert_eq!(names_of_run(&fx, rf), vec!["bar", "foo"]);

    // the run records the ref as given and the commit it named
    for (run, name) in [(rm, "main"), (rf, "feat")] {
        let (r, c): (String, String) = fx.db().query_row("SELECT ref, ref_commit FROM run WHERE id=?1", [run], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(r, name);
        assert_eq!(c, git_out(&d, &["rev-parse", name]).trim());
    }
    assert_eq!(f["data"]["ref"], "feat");
    assert_eq!(f["data"]["ref_commit"], git_out(&d, &["rev-parse", "feat"]).trim());

    // `repos` shows them
    let repos = ok(fx.root(), &["repos"]);
    let refs = repos["data"]["items"][0]["ref_runs"].as_array().unwrap().clone();
    let got: Vec<(String, String)> = refs.iter().map(|x| (x["ref"].as_str().unwrap().into(), x["ref_commit"].as_str().unwrap().into())).collect();
    assert_eq!(got, vec![("feat".into(), git_out(&d, &["rev-parse", "feat"]).trim().into()), ("main".into(), git_out(&d, &["rev-parse", "main"]).trim().into())]);

    // query verbs: default = the head run (built here at HEAD = main), --ref = that ref's run
    ok(fx.root(), &["build", "r"]);
    err(fx.root(), &["show", "r", "src/a.py::bar"], "not_found");
    ok(fx.root(), &["show", "r", "src/a.py::bar", "--ref", "feat"]);
    err(fx.root(), &["show", "r", "src/a.py::bar", "--ref", "main"], "not_found");
    // a ref that was never built has no run to answer from
    git(&d, &["branch", "other"]);
    err(fx.root(), &["show", "r", "src/a.py::foo", "--ref", "other"], "not_found");
    // the head run is still today's latest: a ref build did not become "the latest run"
    let latest = ok(fx.root(), &["repos"])["data"]["items"][0]["latest_run"]["nodes"].as_i64().unwrap();
    assert!(latest > 0);
    // search answers from the head runs only
    let hits = ok(fx.root(), &["search", "bar"]);
    assert!(hits["data"]["items"].as_array().unwrap().is_empty(), "{hits}");
}

fn tree_state(d: &Path) -> (String, String, Vec<u8>) {
    (git_out(d, &["status", "--porcelain"]), git_out(d, &["rev-parse", "HEAD"]), std::fs::read(d.join(".git/index")).unwrap())
}

#[test]
fn a_ref_build_leaves_the_clones_head_index_and_working_tree_untouched() {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/b.py", BAR);
    commit(&d, "feat");
    git(&d, &["checkout", "-q", "main"]);
    // a dirty clone: a modified tracked file, an untracked file, a staged file
    write(&d, "src/a.py", "def foo():\n    return 99\n");
    write(&d, "src/untracked.py", "def u():\n    return 0\n");
    write(&d, "src/staged.py", "def s():\n    return 0\n");
    git(&d, &["add", "src/staged.py"]);
    let before = tree_state(&d);
    let files_before: Vec<u8> = std::fs::read(d.join("src/a.py")).unwrap();
    assert!(!before.0.is_empty());

    fx.build_ref("feat");
    fx.build_ref("main");
    fx.build_ref("feat"); // a second build of the same commit reuses the run and still touches nothing

    assert_eq!(tree_state(&d), before, "status, HEAD or the index changed");
    assert_eq!(std::fs::read(d.join("src/a.py")).unwrap(), files_before);
    assert_eq!(git_out(&d, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(), "main");
    // the cache folder is under the store, not in the clone
    assert!(fx.root().join(".comparegraph/refs/r").is_dir());
    assert!(!d.join(".comparegraph").exists());
    // the working tree the head build reads is not what the ref build read: foo returns 99 there, 1 at main
    let cache = std::fs::read_to_string(fx.root().join(".comparegraph/refs/r").join(git_out(&d, &["rev-parse", "main"]).trim()).join("src/a.py")).unwrap();
    assert!(cache.contains("return 1"), "{cache}");
    // `check` does not call a ref run's clone unavailable
    ok(fx.root(), &["check"]);
}

fn cached(fx: &Fx, rel: &str) -> PathBuf {
    fx.root().join(".comparegraph/refs/r").join(git_out(&fx.clone(), &["rev-parse", "main"]).trim()).join(rel)
}

/// M1: the tree is read as raw blobs. No smudge filter of the clone runs (git-lfs would fetch and write .git/lfs/objects), no eol
/// conversion, no export-ignore; the clone's status, HEAD and index stay as they were.
#[test]
fn a_ref_build_runs_no_filter_or_eol_conversion_of_the_clone() {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.clone();
    let marker = fx.base.path().join("smudge-ran");
    let script = fx.base.path().join("smudge.sh");
    let fwd = |p: &Path| p.display().to_string().replace('\\', "/");
    std::fs::write(&script, format!("#!/bin/sh\ntouch '{}'\ncat\n", fwd(&marker))).unwrap();
    for f in ["x", "lfs"] {
        git(&d, &["config", &format!("filter.{f}.smudge"), &format!("sh '{}'", fwd(&script))]);
        git(&d, &["config", &format!("filter.{f}.clean"), "cat"]);
        git(&d, &["config", &format!("filter.{f}.required"), "true"]);
    }
    git(&d, &["config", "core.autocrlf", "true"]);
    write(&d, ".gitattributes", "*.py filter=x text eol=crlf\n*.bin filter=lfs diff=lfs merge=lfs -text\nsrc/ignored.py export-ignore\n");
    write(&d, "src/ignored.py", BAR);
    let pointer = "version https://git-lfs.github.com/spec/v1\noid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\nsize 12345\n";
    write(&d, "data/blob.bin", pointer);
    commit(&d, "attributes");
    let _ = std::fs::remove_file(&marker);
    let before = tree_state(&d);

    fx.build_ref("main");

    assert!(!marker.exists(), "a smudge filter of the clone ran during a ref build");
    assert!(!d.join(".git/lfs").exists(), "an lfs-style filter wrote .git/lfs");
    assert_eq!(std::fs::read_to_string(cached(&fx, "data/blob.bin")).unwrap(), pointer, "the blob is the committed pointer, not a fetch");
    let a = std::fs::read(cached(&fx, "src/a.py")).unwrap();
    assert!(!a.contains(&b'\r'), "eol conversion was applied: {a:?}");
    assert!(cached(&fx, "src/ignored.py").is_file(), "export-ignore must have no effect on what is read");
    assert_eq!(tree_state(&d), before, "status, HEAD or the index of the clone changed");
}

#[test]
fn a_symlink_in_the_tree_is_a_file_holding_its_target_never_followed() {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.clone();
    // a symlink entry made without the OS (as a Windows clone without symlink rights has it): mode 120000, blob = the target
    let target = fx.base.path().join("target.txt");
    std::fs::write(&target, "../../../../outside.py").unwrap();
    let oid = git_out(&d, &["hash-object", "-w", target.to_str().unwrap()]).trim().to_string();
    git(&d, &["update-index", "--add", "--cacheinfo", &format!("120000,{oid},src/link.py")]);
    git(&d, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "a symlink"]);
    let before = tree_state(&d);

    fx.build_ref("main");

    let link = cached(&fx, "src/link.py");
    let meta = std::fs::symlink_metadata(&link).unwrap();
    assert!(meta.file_type().is_file(), "the link must be a regular file, never a symlink: {meta:?}");
    assert_eq!(std::fs::read_to_string(&link).unwrap(), "../../../../outside.py");
    assert_eq!(tree_state(&d), before);
}

/// M4: `search` answers from head runs only, even when a ref run is the newest run of the repo.
#[test]
fn search_never_answers_from_a_ref_run_even_the_newest() {
    let fx = lane(BAR);
    ok(fx.root(), &["build", "r"]); // the head run (main: no bar)
    fx.build_ref("feat"); // built after it: the newest run of the repo holds bar
    let newest: Option<String> = fx.db().query_row("SELECT ref FROM run ORDER BY id DESC LIMIT 1", [], |r| r.get(0)).unwrap();
    assert_eq!(newest.as_deref(), Some("feat"));
    let hits = ok(fx.root(), &["search", "bar"]);
    assert!(hits["data"]["items"].as_array().unwrap().is_empty(), "{hits}");
    assert!(!ok(fx.root(), &["search", "foo"])["data"]["items"].as_array().unwrap().is_empty(), "the head run still answers");
}

#[test]
fn an_unknown_ref_is_a_structured_refusal_and_writes_no_run() {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let runs = |fx: &Fx| fx.db().query_row("SELECT COUNT(*) FROM run", [], |r| r.get::<_, i64>(0)).unwrap();
    let v = err(fx.root(), &["build", "r", "--ref", "no/such/branch"], "not_found");
    assert_eq!(v["error"]["data"]["ref"], "no/such/branch");
    assert!(v["error"]["hint"].as_str().unwrap().contains("--ref"), "{v}");
    err(fx.root(), &["build", "r", "--ref=-oops"], "not_found");
    assert_eq!(runs(&fx), 0);
    // --ref belongs to build and the graph read verbs
    assert!(!common::cc(fx.root()).args(["claims", "--ref", "main"]).output().unwrap().status.success(), "--ref is not an option of claims");
    err(fx.root(), &["build", "r", "--providers", "reference,crg", "--ref", "main"], "invalid_arguments");
}

// ---------------------------------------------------------------- CG-DELTA (the branch's own work: merge base -> head)

/// two statements: a one-line body is too little evidence for a body match (`delta`'s trivial_body rule)
const QUX: &str = "def qux(z):\n    w = z - 3\n    return w * w\n";

fn branch(v: &Value) -> &Value {
    &v["data"]["branch"]
}

/// feat leaves base at c1 and adds `extra` to src/a.py; main stays put.
fn lane(extra: &str) -> Fx {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{FOO}\n{extra}"));
    commit(&d, "feat");
    git(&d, &["checkout", "-q", "main"]);
    fx
}

#[test]
fn one_new_function_is_exactly_the_one_branch_symbol() {
    let fx = lane(BAR);
    let v = fx.delta("main", "feat");
    let data = &v["data"];
    assert_eq!(qns(&branch(&v)["added"]), vec!["src/a.py::bar"]);
    assert!(branch(&v)["removed"].as_array().unwrap().is_empty());
    assert!(branch(&v)["changed"].as_array().unwrap().is_empty());
    let s = &data["summary"];
    assert_eq!((s["branch_symbols"].as_i64(), s["already_on_base"].as_i64(), s["stale_conflicts"].as_i64()), (Some(1), Some(0), Some(0)));
    assert_eq!(s["share_already_on_base"].as_f64(), Some(0.0));
    assert!(branch(&v)["added"][0].get("already_on_base").is_none());
    assert!(data.get("verdict").is_none() && s.get("verdict").is_none());
    assert!(data.get("whole_tree").is_none(), "the whole-tree section is off by default");
    assert_eq!(data["merge_base"]["commit"], git_out(&fx.clone(), &["rev-parse", "main"]).trim());
    assert_eq!(v["verb"], "delta");
}

#[test]
fn a_branch_body_that_changed_is_reported_changed() {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", "def foo():\n    return 2\n");
    commit(&d, "feat edits foo");
    git(&d, &["checkout", "-q", "main"]);
    let v = fx.delta("main", "feat");
    assert_eq!(qns(&branch(&v)["changed"]), vec!["src/a.py::foo"]);
    assert_eq!(v["data"]["summary"]["branch_symbols"], 1);
    assert!(branch(&v)["added"].as_array().unwrap().is_empty());
}

#[test]
fn base_gaining_unrelated_symbols_after_the_lane_left_does_not_count_as_the_lane() {
    let fx = lane(BAR);
    let d = fx.clone();
    let mut many = String::new();
    for i in 0..50 {
        many.push_str(&format!("def unrelated_{i}():\n    return {}\n\n", i + 100));
    }
    write(&d, "src/other.py", &many);
    commit(&d, "main gains 50 symbols");
    let v = fx.delta("main", "feat");
    assert_eq!(v["data"]["summary"]["branch_symbols"], 1, "{}", v["data"]["summary"]);
    assert_eq!(qns(&branch(&v)["added"]), vec!["src/a.py::bar"]);
    // the old whole-tree comparison is still there on request, and shows base's own growth as base-only
    let w = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--whole-tree"]);
    assert_eq!(w["data"]["whole_tree"]["summary"]["removed"], 50);
    assert_eq!(w["data"]["whole_tree"]["summary"]["added"], 1);
}

/// feat leaves c1 and adds bar and qux; main later takes the same code under new names in a moved file.
fn merged_with_rename() -> Fx {
    let fx = lane(&format!("{BAR}\n{QUX}"));
    let d = fx.clone();
    git(&d, &["rm", "-q", "src/a.py"]);
    write(&d, "lib/b.py", "def foo2():\n    return 1\n\ndef bar2(x):\n        y = x * 2\n        return y + 1\n\ndef qux2(z):\n    w = z - 3\n    return w * w\n");
    commit(&d, "main moves and renames");
    fx
}

#[test]
fn a_lane_merged_into_base_with_a_rename_is_entirely_already_on_base() {
    let fx = merged_with_rename();
    let v = fx.delta("main", "feat");
    let s = &v["data"]["summary"];
    assert_eq!(qns(&branch(&v)["added"]), vec!["src/a.py::bar", "src/a.py::qux"]);
    assert_eq!((s["branch_symbols"].as_i64(), s["already_on_base"].as_i64()), (Some(2), Some(2)));
    assert_eq!(s["share_already_on_base"].as_f64(), Some(1.0));
    for (i, to) in [(0, "lib/b.py::bar2"), (1, "lib/b.py::qux2")] {
        let m = &branch(&v)["added"][i]["already_on_base"];
        assert_eq!((m["confidence"].as_f64().unwrap(), m["via"].as_str().unwrap()), (1.0, "body_hash"));
        assert_eq!(m["matches"][0]["qualified_name"], to);
    }
    // base deleted src/a.py, which the lane edited
    assert_eq!(v["data"]["base_files"]["deleted"], json!([{ "path": "src/a.py", "head_modified": true }]));
    // whole-tree on request: foo (unchanged by the lane) shows too
    let w = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--whole-tree"]);
    assert_eq!(w["data"]["whole_tree"]["summary"]["added"], 3);
}

#[test]
fn a_lane_merged_unchanged_is_already_on_base_as_the_same_symbol() {
    let fx = lane(BAR);
    let d = fx.clone();
    write(&d, "src/a.py", &format!("{FOO}\n{BAR}")); // main took the lane's work as is
    commit(&d, "main merges the lane");
    let v = fx.delta("main", "feat");
    let m = &branch(&v)["added"][0]["already_on_base"];
    assert_eq!((m["via"].as_str().unwrap(), m["confidence"].as_f64().unwrap()), ("same_symbol", 1.0));
    assert_eq!(v["data"]["summary"]["share_already_on_base"].as_f64(), Some(1.0));
}

#[test]
fn a_function_the_lane_changed_and_base_rewrote_is_a_stale_conflict() {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", "def foo():\n    return 2\n");
    commit(&d, "lane edits foo");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/a.py", "def foo():\n    return 3\n");
    commit(&d, "base rewrites foo differently");
    let v = fx.delta("main", "feat");
    let s = &v["data"]["summary"];
    assert_eq!((s["branch_symbols"].as_i64(), s["stale_conflicts"].as_i64(), s["already_on_base"].as_i64()), (Some(1), Some(1), Some(0)));
    let row = &branch(&v)["changed"][0];
    assert_eq!(row["base_also_changed"], true);
    assert!(row["already_on_base"].is_null(), "not on base (a one-line body is not judged: null, note trivial_body)");
    assert_eq!(v["data"]["base_files"]["rewritten"], json!([{ "path": "src/a.py", "head_modified": true }]));

    // base changed it to the very same thing: no conflict, the lane's change is on base
    write(&d, "src/a.py", "def foo():\n    return 2\n");
    commit(&d, "base now agrees");
    let v = fx.delta("main", "feat");
    assert_eq!(v["data"]["summary"]["stale_conflicts"], 0);
    assert_eq!(branch(&v)["changed"][0]["already_on_base"]["via"], "same_symbol");
}

// ---------------------------------------------------------------- B1: a change to a symbol base moved is not "already on base"

const HANDLE: &str = "def handle(req):\n    user = req.user\n    data = load(req.path)\n    out = render(user, data)\n    return out\n";
const HANDLE_GUARDED: &str = "def handle(req):\n    if not req.ok:\n        raise Denied()\n    user = req.user\n    data = load(req.path)\n    out = render(user, data)\n    return out\n";

/// `def process(items)`: 30 lines with their own names (so the token-bigram Jaccard of a 2-line change stays above 0.9),
/// optionally a 2-line guard after the first line and a tweak on the return.
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

/// main has `before` in src/a.py; feat changes it to `lane`; main then moves the file (git mv) and writes `base` at src/b.py.
fn lane_then_base_moves(before: &str, lane: &str, base: Option<&str>) -> Fx {
    let fx = Fx::new(&[("src/a.py", before)]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", lane);
    commit(&d, "lane changes the function");
    git(&d, &["checkout", "-q", "main"]);
    git(&d, &["mv", "src/a.py", "src/b.py"]);
    if let Some(b) = base {
        write(&d, "src/b.py", b);
    }
    commit(&d, "main moves the file");
    fx
}

#[test]
fn a_change_to_a_function_base_only_moved_is_a_stale_conflict_not_on_base() {
    let fx = lane_then_base_moves(HANDLE, HANDLE_GUARDED, None);
    let v = fx.delta("main", "feat");
    let s = &v["data"]["summary"];
    assert_eq!(qns(&branch(&v)["changed"]), vec!["src/a.py::handle"], "{v}");
    assert_eq!(s["share_already_on_base"].as_f64(), Some(0.0), "{s}");
    assert_eq!(s["stale_conflicts"].as_i64(), Some(1), "{s}");
    assert_eq!(s["already_on_base"].as_i64(), Some(0), "{s}");
    let row = &branch(&v)["changed"][0];
    assert!(row["already_on_base"].is_null(), "{row}");
    assert_eq!(row["base_removed"], true);
    // where base keeps the merge-base body: the moved copy, without the branch's change
    assert_eq!(row["base_moved_to"][0]["qualified_name"], "src/b.py::handle", "{row}");
}

#[test]
fn a_long_body_whose_moved_copy_lacks_the_branch_s_two_new_lines_is_not_on_base() {
    let fx = lane_then_base_moves(&long_process(false, ""), &long_process(true, ""), None);
    let v = fx.delta("main", "feat");
    let s = &v["data"]["summary"];
    let row = &branch(&v)["changed"][0];
    assert_eq!(row["qualified_name"], "src/a.py::process");
    assert!(row["already_on_base"].is_null(), "the moved copy is nearer the merge base than the branch: {row}");
    // the moved copy IS within the name_and_tokens floor of head: only the nearer-to-head rule keeps it off base
    let w = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--whole-tree"]);
    let near = w["data"]["whole_tree"]["added"].as_array().unwrap().iter().find(|r| r["qualified_name"] == "src/a.py::process").unwrap().clone();
    assert_eq!(near["already_on_base"]["via"], "name_and_tokens", "{near}");
    assert_eq!((s["share_already_on_base"].as_f64(), s["stale_conflicts"].as_i64()), (Some(0.0), Some(1)), "{s}");
}

#[test]
fn a_branch_change_base_took_after_a_rename_is_on_base() {
    // exactly: base moved the file and holds the branch's body
    let fx = lane_then_base_moves(&long_process(false, ""), &long_process(true, ""), Some(&long_process(true, "")));
    let v = fx.delta("main", "feat");
    let row = &branch(&v)["changed"][0];
    assert_eq!((row["already_on_base"]["via"].as_str(), row["already_on_base"]["matches"][0]["qualified_name"].as_str()), (Some("body_hash"), Some("src/b.py::process")), "{row}");
    assert_eq!((v["data"]["summary"]["share_already_on_base"].as_f64(), v["data"]["summary"]["stale_conflicts"].as_i64()), (Some(1.0), Some(0)));
    // nearly: base took the branch's two lines and tweaked the return; it is nearer the branch than the merge base
    let fx = lane_then_base_moves(&long_process(false, ""), &long_process(true, ""), Some(&long_process(true, " + 1")));
    let v = fx.delta("main", "feat");
    let row = &branch(&v)["changed"][0];
    assert_eq!(row["already_on_base"]["via"], "name_and_tokens", "{row}");
    assert_eq!((v["data"]["summary"]["share_already_on_base"].as_f64(), v["data"]["summary"]["stale_conflicts"].as_i64()), (Some(1.0), Some(0)));
}

#[test]
fn a_stub_body_is_not_on_base_by_hash_or_tokens() {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/auth.py", "def verify_token(x): pass\n");
    commit(&d, "feat adds a stub");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/old.py", "def legacy_noop(x): pass\n");
    commit(&d, "main has another stub");
    let v = fx.delta("main", "feat");
    let row = &branch(&v)["added"][0];
    assert_eq!(row["qualified_name"], "src/auth.py::verify_token");
    // answered, and the answer is "not known": null, with the reason
    assert!(row.get("already_on_base").is_some_and(Value::is_null), "{row}");
    assert_eq!(row["note"], "trivial_body", "{row}");
    assert_eq!(v["data"]["summary"]["share_already_on_base"].as_f64(), Some(0.0));
    assert!(v["evidence"]["limitations"].to_string().contains("trivial_body"), "{}", v["evidence"]);
    // the same stub under the same identity on base is still the same symbol (identity, not a body match)
    write(&d, "src/auth.py", "def verify_token(x): pass\n");
    commit(&d, "main takes the stub as is");
    let v = fx.delta("main", "feat");
    assert_eq!(branch(&v)["added"][0]["already_on_base"]["via"], "same_symbol");
}

#[test]
fn runs_built_with_other_options_are_never_mixed_into_one_delta() {
    let fx = Fx::new(&[("src/a.py", FOO), ("other/o.py", BAR)]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/n.py", HANDLE);
    commit(&d, "feat adds handle");
    git(&d, &["checkout", "-q", "main"]);
    ok(fx.root(), &["build", "r", "--ref", "main", "--include", "src/**"]);
    fx.build_ref("feat");
    // main is also the merge base; its only run read src/** while feat read the whole tree: other/o.py::bar is not the branch's
    let v = run(fx.root(), &["delta", "r", "--base", "main", "--head", "feat"]);
    assert_eq!(v["ok"], false, "a delta over runs of different subsets must refuse, never answer: {v}");
    assert_eq!(v["error"]["data"]["ref"], "main", "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("include"), "{v}");
    assert_eq!(v["error"]["data"]["found"]["include"], json!(["src/**"]), "{v}");
    // told to build, it builds main with the delta's options and answers: only the branch's own symbol
    let v = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--build-missing"]);
    assert_eq!(qns(&branch(&v)["added"]), vec!["src/n.py::handle"], "{v}");
    assert_eq!(v["data"]["rebuilt"].as_array().unwrap().len(), 1, "{v}");
    // and the strict call now answers from the matching runs
    assert_eq!(ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat"])["data"]["summary"]["added"], 1);
}

#[test]
fn a_symbol_the_lane_removed_and_base_removed_too_is_marked_base_removed() {
    let fx = Fx::new(&[("src/a.py", &format!("{FOO}\n{BAR}"))]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", FOO);
    commit(&d, "lane drops bar");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/a.py", FOO);
    commit(&d, "base drops bar");
    let v = fx.delta("main", "feat");
    assert_eq!(qns(&branch(&v)["removed"]), vec!["src/a.py::bar"]);
    assert_eq!(branch(&v)["removed"][0]["base_removed"], true);
    assert_eq!(v["data"]["summary"]["share_already_on_base"].as_f64(), Some(1.0));
}

#[test]
fn a_file_base_deleted_is_reported_only_when_the_lane_touched_it() {
    let fx = Fx::new(&[("src/a.py", FOO), ("src/gone.py", "def gone():\n    return 5\n"), ("src/quiet.py", "def quiet():\n    return 8\n"), ("src/kept.py", "def kept():\n    return 6\n")]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/gone.py", "def gone():\n    return 55\n");
    write(&d, "src/new.py", BAR);
    commit(&d, "lane edits gone.py and adds a file");
    git(&d, &["checkout", "-q", "main"]);
    git(&d, &["rm", "-q", "src/gone.py", "src/quiet.py"]);
    write(&d, "src/kept.py", "def kept():\n    return 7\n");
    commit(&d, "main deletes gone.py and quiet.py, rewrites kept.py");
    let v = fx.delta("main", "feat");
    // quiet.py and kept.py were not touched by the lane: not its business
    assert_eq!(v["data"]["base_files"]["deleted"], json!([{ "path": "src/gone.py", "head_modified": true }]));
    assert!(v["data"]["base_files"]["rewritten"].as_array().unwrap().is_empty());
    let changed = &branch(&v)["changed"][0];
    assert_eq!((changed["qualified_name"].as_str().unwrap(), &changed["base_removed"]), ("src/gone.py::gone", &json!(true)));
    assert_eq!(v["data"]["summary"]["files_base_deleted"], 1);
}

#[test]
fn the_same_name_with_a_near_identical_body_is_matched_at_a_lower_stated_confidence() {
    let long = |tweak: &str| {
        let mut s = String::from("def process(items):\n    total = 0\n");
        for i in 0..14 {
            s.push_str(&format!("    total = total + items[{i}] * {} - len(items)\n", i + 2));
        }
        s.push_str(&format!("    return total{tweak}\n"));
        s
    };
    let fx = Fx::new(&[("src/a.py", "def keep():\n    return 0\n")]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("def keep():\n    return 0\n\n{}", long("")));
    commit(&d, "feat adds process");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "pkg/p.py", &long(" + 1")); // same name and kind, one token apart
    commit(&d, "main has a slightly different process");
    let v = fx.delta("main", "feat");
    let row = &branch(&v)["added"][0];
    assert_eq!(row["qualified_name"], "src/a.py::process");
    let m = &row["already_on_base"];
    assert_eq!(m["via"], "name_and_tokens");
    let c = m["confidence"].as_f64().unwrap();
    assert!((0.9..1.0).contains(&c), "{c}");
    assert_eq!(m["matches"][0]["qualified_name"], "pkg/p.py::process");

    // a body that is really different is not matched, whatever the name
    write(&d, "pkg/p.py", "def process(items):\n    return sorted(items)\n");
    commit(&d, "main rewrites process");
    let v = fx.delta("main", "feat");
    assert!(branch(&v)["added"][0].get("already_on_base").is_none(), "{v}");
}

#[test]
fn rows_list_what_is_not_on_base_first() {
    let fx = lane(&format!("{QUX}\n{BAR}"));
    let d = fx.clone();
    write(&d, "lib/b.py", "def qux2(z):\n    w = z - 3\n    return w * w\n"); // base has qux (renamed), not bar
    commit(&d, "main has qux2");
    let v = fx.delta("main", "feat");
    // bar (not on base) before qux (on base), though qux comes first by line
    assert_eq!(qns(&branch(&v)["added"]), vec!["src/a.py::bar", "src/a.py::qux"]);
}

#[test]
fn added_symbols_map_to_the_families_features_with_map_s_ranking() {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    fx.family();
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/net.py", "def retry_with_backoff(call):\n    return call()\n");
    commit(&d, "feat adds retry");
    git(&d, &["checkout", "-q", "main"]);
    let v = fx.delta("main", "feat");
    let row = &branch(&v)["added"][0];
    assert_eq!(row["qualified_name"], "src/net.py::retry_with_backoff");
    let feats: Vec<&str> = row["features"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    assert!(feats.iter().any(|f| f.starts_with("fam/") && f.contains("routing.retry")), "{feats:?}");
    assert_eq!(v["data"]["features_touched"][0]["branch_symbols"], 1);
    let one = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--family", "fam"]);
    assert_eq!(branch(&one)["added"][0]["features"], row["features"]);
    err(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--family", "nope"], "not_found");
}

#[test]
fn delta_is_deterministic_and_bounded() {
    let fx = merged_with_rename();
    let first = fx.delta("main", "feat");
    assert_eq!(first["data"]["rebuilt"].as_array().unwrap().len(), 3, "main, feat and the merge base are built on the way");
    let (a, b) = (fx.delta("main", "feat"), fx.delta("main", "feat"));
    assert!(a["data"]["rebuilt"].as_array().unwrap().is_empty());
    assert_eq!(sha(&a["data"]), sha(&b["data"]));
    assert_eq!(a["data"], b["data"]);
    let c = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--max-nodes", "1"]);
    assert_eq!(branch(&c)["added"].as_array().unwrap().len(), 1);
    assert_eq!(c["data"]["summary"]["added"], 2);
    let lims = c["evidence"]["limitations"].to_string();
    assert!(lims.contains("truncated"), "{lims}");
    err(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--max-nodes", "0"], "invalid_arguments");
    err(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--max-nodes", "2001"], "invalid_arguments");
}

#[test]
fn delta_refuses_what_is_unbuilt_unless_told_to_build_it() {
    let fx = lane(BAR);
    let d = fx.clone();
    write(&d, "src/m.py", QUX);
    commit(&d, "main moves on, so the merge base is neither tip");
    fx.build_ref("main");
    let strict = |head: &str| run(fx.root(), &["delta", "r", "--base", "main", "--head", head]);
    let v = strict("feat");
    assert_eq!(v["error"]["code"], "not_found");
    let hint = v["error"]["hint"].as_str().unwrap();
    assert!(hint.contains("build r --ref feat") && hint.contains("--build-missing"), "{v}");
    assert_eq!(v["error"]["data"]["ref"], "feat");
    fx.build_ref("feat");
    // base and head are built; the merge base commit is not
    let v = strict("feat");
    assert_eq!(v["error"]["code"], "not_found");
    assert_eq!(v["error"]["data"]["ref"], git_out(&d, &["merge-base", "main", "feat"]).trim());
    // --build-missing builds it, after which the strict call works
    ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--build-missing"]);
    assert_eq!(strict("feat")["ok"], true);
    err(fx.root(), &["delta", "r", "--base", "main", "--head", "nope"], "not_found");
    err(fx.root(), &["delta", "nope", "--base", "main", "--head", "main"], "not_found");
    // the same ref on both sides is an empty delta, not an error
    let same = fx.delta("main", "main");
    assert_eq!(same["data"]["summary"]["branch_symbols"], 0);
    assert!(same["data"]["summary"]["share_already_on_base"].is_null());
}

#[test]
fn the_mcp_tools_carry_delta_and_ref() {
    use std::io::{BufRead, BufReader, Write};
    let fx = lane(BAR);
    fx.family(); // so a claim the MCP layer let through would be written: the refusal is what stops it
    fx.build_ref("main");
    fx.build_ref("feat");
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("comparegraph"))
        .arg("--root")
        .arg(fx.root())
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut rd = BufReader::new(child.stdout.take().unwrap());
    let mut n = 0;
    let mut call = |name: &str, args: Value| -> Value {
        n += 1;
        let line = json!({"jsonrpc":"2.0","id":n,"method":"tools/call","params":{"name":name,"arguments":args}});
        writeln!(child.stdin.as_mut().unwrap(), "{line}").unwrap();
        let mut out = String::new();
        rd.read_line(&mut out).unwrap();
        let r: Value = serde_json::from_str(&out).unwrap();
        serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let d = call("comparegraph_delta", json!({"repo":"r","base":"main","head":"feat","whole_tree":true}));
    assert_eq!(d["ok"], true, "{d}");
    assert_eq!(qns(&d["data"]["branch"]["added"]), vec!["src/a.py::bar"]);
    assert!(d["data"]["whole_tree"].is_object());
    let s = call("comparegraph_show", json!({"repo":"r","qualified_name":"src/a.py::bar","ref":"feat"}));
    assert_eq!(s["ok"], true, "{s}");
    let s = call("comparegraph_show", json!({"repo":"r","qualified_name":"src/a.py::bar","ref":"main"}));
    assert_eq!(s["error"]["code"], "not_found");
    let s = call("comparegraph_search", json!({"query":"bar","ref":"feat"}));
    assert_eq!(s["error"]["code"], "invalid_arguments");
    // a claim with a ref is refused over MCP too, and writes nothing
    ok(fx.root(), &["build", "r"]);
    let c = call("comparegraph_claim", json!({"repo":"r","feature":"routing.retry","verdict":"unknown","author":"t","ref":"feat"}));
    assert_eq!(c["ok"], false, "{c}");
    assert!(c["error"]["message"].as_str().unwrap().contains("ref run"), "{c}");
    assert_eq!(fx.db().query_row("SELECT COUNT(*) FROM claim", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    drop(child.stdin.take());
    let _ = child.wait();
}
