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
    // round 3: a near match is for a person to read, never landed work: the exact share is 0, the row is one `near_on_base`, and
    // base moved the symbol without the branch's exact change, so it stays a stale conflict
    let s = &v["data"]["summary"];
    assert_eq!((s["share_already_on_base"].as_f64(), s["near_on_base"]["count"].as_i64(), s["stale_conflicts"].as_i64()), (Some(0.0), Some(1), Some(1)), "{s}");
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

// ---------------------------------------------------------------- review round 2: ONE rule for "already on base" (every row type)

/// `long_process` under another name (the name is a placeholder in the body hash, so bodies compare across names).
fn long_named(name: &str, guard: bool) -> String {
    long_process(guard, "").replacen("def process(", &format!("def {name}("), 1)
}
fn on_base_of(row: &Value) -> Option<&Value> {
    row.get("already_on_base").filter(|v| !v.is_null())
}
fn row_of<'a>(v: &'a Value, section: &str, qn: &str) -> &'a Value {
    branch(v)[section].as_array().unwrap().iter().find(|r| r["qualified_name"] == qn).unwrap_or_else(|| panic!("no {section} row {qn}: {}", branch(v)))
}

/// R2-1: the branch moves `process` to b.py and adds a guard; base moves it, unchanged, to c.py. Base's copy is the merge-base
/// body: nearer the merge base than the branch, so the guard is not on base.
#[test]
fn r2_branch_moves_and_guards_while_base_moves_unchanged_is_not_on_base() {
    let fx = Fx::new(&[("src/a.py", &long_process(false, ""))]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    git(&d, &["rm", "-q", "src/a.py"]);
    write(&d, "src/b.py", &long_process(true, ""));
    commit(&d, "lane moves to b and guards");
    git(&d, &["checkout", "-q", "main"]);
    git(&d, &["mv", "src/a.py", "src/c.py"]);
    commit(&d, "base moves to c");
    let v = fx.delta("main", "feat");
    let s = &v["data"]["summary"];
    assert!(on_base_of(row_of(&v, "added", "src/b.py::process")).is_none(), "{}", branch(&v));
    assert_eq!((s["already_on_base"].as_i64(), s["share_already_on_base"].as_f64()), (Some(0), Some(0.0)), "the guard is not on base: {s}");
}

/// R2-2: the branch only moves and changes `process`; base is untouched (its copy is the merge-base copy itself).
#[test]
fn r2_branch_moves_and_changes_with_base_untouched_is_not_on_base() {
    let fx = Fx::new(&[("src/a.py", &long_process(false, ""))]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    git(&d, &["rm", "-q", "src/a.py"]);
    write(&d, "src/c.py", &long_process(true, ""));
    commit(&d, "lane moves and guards process");
    git(&d, &["checkout", "-q", "main"]);
    let v = fx.delta("main", "feat");
    assert!(on_base_of(row_of(&v, "added", "src/c.py::process")).is_none(), "the branch's guarded copy is said to be on base: {}", branch(&v));
    // the removal half of the move is not on base either: base still holds the body under src/a.py
    let rm = row_of(&v, "removed", "src/a.py::process");
    assert!(rm.get("base_removed").is_none(), "{rm}");
    assert_eq!(v["data"]["summary"]["share_already_on_base"].as_f64(), Some(0.0));
}

/// R2-3: the branch deletes `process`; base `git mv`s the file. Base still holds the function (moved): the deletion is not on base,
/// and `base_moved_to` says where base keeps it. Base keeping it in place is not on base either.
#[test]
fn r2_a_deletion_base_only_moved_is_not_on_base_and_names_where_base_keeps_it() {
    let keep = "def keep():\n    return 1\n";
    for moved in [true, false] {
        let fx = Fx::new(&[("src/a.py", &format!("{}\n{keep}", long_process(false, "")))]);
        let d = fx.clone();
        git(&d, &["checkout", "-q", "-b", "feat"]);
        write(&d, "src/a.py", keep);
        commit(&d, "lane deletes process");
        git(&d, &["checkout", "-q", "main"]);
        if moved {
            git(&d, &["mv", "src/a.py", "src/b.py"]);
            commit(&d, "base moves the file");
        }
        let v = fx.delta("main", "feat");
        let s = &v["data"]["summary"];
        assert_eq!(s["share_already_on_base"].as_f64(), Some(0.0), "moved={moved}: the deletion is said to be on base though base holds the function: {s}");
        let row = row_of(&v, "removed", "src/a.py::process");
        if moved {
            assert_eq!(row["base_moved_to"][0]["qualified_name"], "src/b.py::process", "{row}");
        } else {
            assert!(row.get("base_moved_to").is_none(), "base keeps it in place, not moved: {row}");
        }
    }
}

/// R2-4: the branch makes `f` equal to the helper `g`, which the merge base already had and base never changed. Old code on base
/// never shows the branch's work.
#[test]
fn r2_a_change_equal_to_an_unchanged_helper_is_not_on_base() {
    let g = long_named("g", true);
    let fx = Fx::new(&[("src/a.py", &format!("{}\n{g}", long_named("f", false)))]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{}\n{g}", long_named("f", true)));
    commit(&d, "lane makes f guard like g");
    git(&d, &["checkout", "-q", "main"]);
    let v = fx.delta("main", "feat");
    let row = row_of(&v, "changed", "src/a.py::f");
    assert!(on_base_of(row).is_none(), "base's f is unchanged and g is old, yet: {row}");
    assert_eq!(v["data"]["summary"]["already_on_base"].as_i64(), Some(0));
}

/// R2-5: base took the branch's change after a rename (an added row this time: the branch moved and changed): on base, exactly
/// and nearly.
#[test]
fn r2_base_took_a_moved_change_under_another_path_exactly_and_nearly() {
    for (base_body, via) in [(long_process(true, ""), "body_hash"), (long_process(true, " + 1"), "name_and_tokens")] {
        let fx = Fx::new(&[("src/a.py", &long_process(false, ""))]);
        let d = fx.clone();
        git(&d, &["checkout", "-q", "-b", "feat"]);
        git(&d, &["rm", "-q", "src/a.py"]);
        write(&d, "src/b.py", &long_process(true, ""));
        commit(&d, "lane moves to b and guards");
        git(&d, &["checkout", "-q", "main"]);
        git(&d, &["rm", "-q", "src/a.py"]);
        write(&d, "src/c.py", &base_body);
        commit(&d, "base takes the guard, at c");
        let v = fx.delta("main", "feat");
        let row = row_of(&v, "added", "src/b.py::process");
        assert_eq!(on_base_of(row).map(|m| m["via"].clone()), Some(json!(via)), "{row}");
        assert_eq!(row["already_on_base"]["matches"][0]["qualified_name"], "src/c.py::process", "{row}");
        // the removal half: base no longer holds the merge-base body anywhere
        assert!(row_of(&v, "removed", "src/a.py::process").get("base_removed").is_some(), "{}", branch(&v));
        // round 3: only the exact take is landed work; the near take leaves the added row to a person (`near_on_base`)
        let s = &v["data"]["summary"];
        let (share, near) = if via == "body_hash" { (1.0, 0) } else { (0.5, 1) };
        assert_eq!((s["share_already_on_base"].as_f64(), s["near_on_base"]["count"].as_i64()), (Some(share), Some(near)), "{s}");
    }
}

/// R2-6: changed on both sides, differently: a stale conflict, not on base.
#[test]
fn r2_changed_differently_on_both_sides_is_a_stale_conflict() {
    let fx = Fx::new(&[("src/a.py", &long_process(false, ""))]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &long_process(true, ""));
    commit(&d, "lane guards");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/a.py", &long_process(false, " + 1"));
    commit(&d, "base tweaks");
    let v = fx.delta("main", "feat");
    let s = &v["data"]["summary"];
    assert_eq!((s["already_on_base"].as_i64(), s["stale_conflicts"].as_i64()), (Some(0), Some(1)), "{s}");
    assert_eq!(row_of(&v, "changed", "src/a.py::process")["base_also_changed"], true);
}

/// R2-7 (review probe): the branch made two edits, base took the first only. The rule reads base's copy as nearer the branch than
/// the merge base, so it may be on base by tokens, never exactly: `already_on_base_exact` stays 0 and the confidence is below 1.
#[test]
fn r2_base_took_the_first_of_two_edits_is_near_never_exact() {
    let fx = Fx::new(&[("src/a.py", &long_process(false, ""))]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &long_process(true, ""));
    commit(&d, "edit 1");
    write(&d, "src/a.py", &long_process(true, " * 2"));
    commit(&d, "edit 2");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/a.py", &long_process(true, ""));
    commit(&d, "base takes edit 1");
    let v = fx.delta("main", "feat");
    let s = &v["data"]["summary"];
    assert_eq!(s["already_on_base_exact"].as_i64(), Some(0), "{s}");
    let row = row_of(&v, "changed", "src/a.py::process");
    if let Some(m) = on_base_of(row) {
        assert_eq!(m["via"], "name_and_tokens", "{row}");
        assert!(m["confidence"].as_f64().unwrap() < 1.0, "{row}");
    }
}

// ---------------------------------------------------------------- review round 2: the trivial-body floor counts from the signature's end

/// Branch adds `branch_src` in src/n.py, base adds `base_src` in src/o.py (both after the fork); the added rows by name.
fn floor_delta(branch_src: &str, base_src: &str) -> Value {
    let fx = Fx::new(&[("src/a.py", "def keep():\n    return 1\n")]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/n.py", branch_src);
    commit(&d, "lane");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/o.py", base_src);
    commit(&d, "base has look-alikes");
    fx.delta("main", "feat")
}

/// The boundary of `MIN_BODY_TOKENS` (8): a body of 7 tokens after the signature is `trivial_body`, one of 8 matches by hash.
/// Moving the constant either way turns one of the two assertions red.
#[test]
fn r2_the_trivial_floor_is_exactly_eight_tokens_after_the_signature() {
    let v = floor_delta(
        "def seven(x, y, z):\n    return -x + y + z\n\ndef eight(x, y, z, w):\n    return x + y - z - w\n",
        "def s7(x, y, z):\n    return -x + y + z\n\ndef e8(x, y, z, w):\n    return x + y - z - w\n",
    );
    let (seven, eight) = (row_of(&v, "added", "src/n.py::seven"), row_of(&v, "added", "src/n.py::eight"));
    assert_eq!(seven["note"], "trivial_body", "7 tokens (return - x + y + z): {seven}");
    assert!(seven.get("already_on_base").is_some_and(Value::is_null), "{seven}");
    assert_eq!(eight["already_on_base"]["via"], "body_hash", "8 tokens (return x + y - z - w): {eight}");
    assert!(eight.get("note").is_none(), "{eight}");
}

/// A `pass` stub whose signature spans several lines is trivial: the parameters are signature, not body.
#[test]
fn r2_a_stub_with_a_multi_line_signature_is_trivial() {
    let v = floor_delta(
        "def stub(\n    alpha,\n    beta,\n    gamma,\n):\n    pass\n",
        "def other(\n    alpha,\n    beta,\n    gamma,\n):\n    pass\n",
    );
    let stub = row_of(&v, "added", "src/n.py::stub");
    assert_eq!(stub["note"], "trivial_body", "a `pass` stub with a multi-line signature matched: {stub}");
    assert!(stub.get("already_on_base").is_some_and(Value::is_null), "{stub}");
}

/// A docstring is not body, nor is a decorator above the signature (even one spanning lines with a `)` of its own).
#[test]
fn r2_docstrings_and_decorators_do_not_count_as_body() {
    let doc = "    \"\"\"Explain the thing at some length, with many words, so it is long.\"\"\"\n    pass\n";
    let deco = "@app.route(\n    \"/a/b/c\", methods=[\"GET\", \"POST\", \"PUT\"])\n";
    let v = floor_delta(
        &format!("def documented(a, b):\n{doc}\n{deco}def handler(req):\n    return req\n"),
        &format!("def other_doc(a, b):\n{doc}\n{deco}def handler2(req):\n    return req\n"),
    );
    for qn in ["src/n.py::documented", "src/n.py::handler"] {
        let row = row_of(&v, "added", qn);
        assert_eq!(row["note"], "trivial_body", "{qn}: {row}");
    }
}

// ---------------------------------------------------------------- review round 3: near matches never count as landed work

/// The first-of-two-edits probe: base took edit 1 of 2. The exact share is 0 (`share_on_base_exact`, and
/// `share_already_on_base`, which now equals it); the near match is one `near_on_base` row with its J(B,H) and J(B,M), for a
/// person to read. `decide --summary-from-delta` stores both.
#[test]
fn r3_near_matches_are_listed_apart_and_never_in_the_share() {
    let fx = Fx::new(&[("src/a.py", &long_process(false, ""))]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &long_process(true, ""));
    commit(&d, "edit 1");
    write(&d, "src/a.py", &long_process(true, " * 2"));
    commit(&d, "edit 2");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/a.py", &long_process(true, ""));
    commit(&d, "base takes edit 1");
    let v = fx.delta("main", "feat");
    let s = &v["data"]["summary"];
    assert_eq!(s["share_on_base_exact"].as_f64(), Some(0.0), "{s}");
    assert_eq!(s["share_already_on_base"].as_f64(), Some(0.0), "the share counts exact rows only: {s}");
    assert_eq!(s["near_on_base"]["count"].as_i64(), Some(1), "{s}");
    let near = &s["near_on_base"]["rows"][0];
    assert_eq!(near["qualified_name"], "src/a.py::process", "{near}");
    let (jh, jm) = (near["j_head"].as_f64().unwrap(), near["j_merge_base"].as_f64().unwrap());
    assert!(jh >= 0.9 && jh > jm, "{near}");
    let sha = git_out(&d, &["rev-parse", "feat"]).trim().to_string();
    ok(fx.root(), &["decide", "r", "park", "--why", "w", "--by", "t", "--ref", "feat", "--commit", &sha, "--summary-from-delta"]);
    let rec = &ok(fx.root(), &["decisions", "--ref", "feat"])["data"]["items"][0]["summary"]["summary"];
    assert_eq!((rec["share_on_base_exact"].as_f64(), rec["near_on_base"]["count"].as_i64()), (Some(0.0), Some(1)), "{rec}");
}
