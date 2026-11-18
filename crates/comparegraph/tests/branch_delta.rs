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
    fn delta(&self, base: &str, head: &str) -> Value {
        ok(self.root(), &["delta", "r", "--base", base, "--head", head])
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

// ---------------------------------------------------------------- CG-DELTA

#[test]
fn one_new_function_is_exactly_the_one_head_only_symbol() {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{FOO}\n{BAR}"));
    commit(&d, "feat adds bar");
    git(&d, &["checkout", "-q", "main"]);
    fx.build_ref("main");
    fx.build_ref("feat");
    let v = fx.delta("main", "feat");
    let data = &v["data"];
    assert_eq!(qns(&data["added"]), vec!["src/a.py::bar"]);
    assert!(data["removed"].as_array().unwrap().is_empty());
    assert!(data["changed"].as_array().unwrap().is_empty());
    let s = &data["summary"];
    assert_eq!((s["head_only"].as_i64(), s["head_only_already_on_base"].as_i64(), s["unchanged"].as_i64()), (Some(1), Some(0), Some(1)));
    assert!(s["share_already_on_base"].as_f64().unwrap() == 0.0);
    assert!(data["added"][0].get("already_on_base").is_none());
    assert!(data.get("verdict").is_none() && s.get("verdict").is_none());
    assert_eq!(v["verb"], "delta");
}

#[test]
fn a_body_that_changed_is_reported_changed() {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", "def foo():\n    return 2\n");
    commit(&d, "feat edits foo");
    git(&d, &["checkout", "-q", "main"]);
    fx.build_ref("main");
    fx.build_ref("feat");
    let v = fx.delta("main", "feat");
    assert_eq!(qns(&v["data"]["changed"]), vec!["src/a.py::foo"]);
    assert_eq!(v["data"]["summary"]["head_only"], 1);
    assert!(v["data"]["added"].as_array().unwrap().is_empty());
}

/// A branch cut from c1, while main later moved the file and renamed both functions: the branch's code is all on base.
fn rebased_with_rename_and_move() -> Fx {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{FOO}\n{BAR}"));
    commit(&d, "feat adds bar");
    git(&d, &["checkout", "-q", "main"]);
    git(&d, &["rm", "-q", "src/a.py"]);
    write(&d, "lib/b.py", "def foo2():\n    return 1\n\ndef bar2(x):\n        y = x * 2\n        return y + 1\n"); // moved, renamed, re-indented
    commit(&d, "main moves and renames");
    fx.build_ref("main");
    fx.build_ref("feat");
    fx
}

#[test]
fn a_branch_rebased_with_a_rename_and_a_move_is_already_on_base() {
    let fx = rebased_with_rename_and_move();
    let v = fx.delta("main", "feat");
    let data = &v["data"];
    assert_eq!(qns(&data["added"]), vec!["src/a.py::foo", "src/a.py::bar"]);
    let s = &data["summary"];
    assert_eq!(s["head_only"], 2);
    assert_eq!(s["head_only_already_on_base"], 2);
    assert_eq!(s["head_only_already_on_base_exact"], 2);
    assert_eq!(s["share_already_on_base"].as_f64().unwrap(), 1.0);
    for (i, to) in [(0, "lib/b.py::foo2"), (1, "lib/b.py::bar2")] {
        let m = &data["added"][i]["already_on_base"];
        assert_eq!((m["confidence"].as_f64().unwrap(), m["via"].as_str().unwrap()), (1.0, "body_hash"));
        assert_eq!(m["matches"][0]["qualified_name"], to);
    }
    // what base has that the branch lacks is listed too, never judged
    assert_eq!(qns(&data["removed"]), vec!["lib/b.py::foo2", "lib/b.py::bar2"]);
    // base deleted src/a.py, which the branch edited
    assert_eq!(data["base_files"]["deleted"], json!([{ "path": "src/a.py", "head_modified": true }]));
    assert_eq!(s["files_base_deleted"], 1);
    assert!(data["merge_base"].as_str().unwrap().len() >= 40);
}

#[test]
fn a_file_base_deleted_is_reported_even_when_the_branch_did_not_touch_it() {
    let fx = Fx::new(&[("src/a.py", FOO), ("src/gone.py", "def gone():\n    return 5\n"), ("src/kept.py", "def kept():\n    return 6\n")]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/new.py", BAR);
    commit(&d, "feat adds a file");
    git(&d, &["checkout", "-q", "main"]);
    git(&d, &["rm", "-q", "src/gone.py"]);
    write(&d, "src/kept.py", "def kept():\n    return 7\n");
    commit(&d, "main deletes gone.py and rewrites kept.py");
    fx.build_ref("main");
    fx.build_ref("feat");
    let v = fx.delta("main", "feat");
    assert_eq!(v["data"]["base_files"]["deleted"], json!([{ "path": "src/gone.py", "head_modified": false }]));
    // kept.py was rewritten by base but not touched by the branch: not a conflict, not listed
    assert!(v["data"]["base_files"]["rewritten"].as_array().unwrap().is_empty());
    // the branch still holds gone() and the old kept(): head-only, with no copy on base
    assert!(qns(&v["data"]["added"]).contains(&"src/gone.py::gone".to_string()));
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
    fx.build_ref("main");
    fx.build_ref("feat");
    let v = fx.delta("main", "feat");
    let row = &v["data"]["added"][0];
    assert_eq!(row["qualified_name"], "src/a.py::process");
    let m = &row["already_on_base"];
    assert_eq!(m["via"], "name_and_tokens");
    let c = m["confidence"].as_f64().unwrap();
    assert!((0.9..1.0).contains(&c), "{c}");
    assert_eq!(m["matches"][0]["qualified_name"], "pkg/p.py::process");
    assert_eq!(v["data"]["summary"]["head_only_already_on_base_exact"], 0);

    // a body that is really different is not matched, whatever the name
    write(&d, "pkg/p.py", "def process(items):\n    return sorted(items)\n");
    commit(&d, "main rewrites process");
    fx.build_ref("main");
    let v = fx.delta("main", "feat");
    assert!(v["data"]["added"][0].get("already_on_base").is_none(), "{v}");
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
    fx.build_ref("main");
    fx.build_ref("feat");
    let v = fx.delta("main", "feat");
    let row = &v["data"]["added"][0];
    assert_eq!(row["qualified_name"], "src/net.py::retry_with_backoff");
    let feats: Vec<&str> = row["features"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    assert!(feats.iter().any(|f| f.starts_with("fam/") && f.contains("routing.retry")), "{feats:?}");
    assert_eq!(v["data"]["features_touched"][0]["head_only_symbols"], 1);
    // --family narrows; an unknown family is refused
    let one = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--family", "fam"]);
    assert_eq!(one["data"]["added"][0]["features"], row["features"]);
    err(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--family", "nope"], "not_found");
}

#[test]
fn delta_is_deterministic_and_bounded() {
    let fx = rebased_with_rename_and_move();
    let (a, b) = (fx.delta("main", "feat"), fx.delta("main", "feat"));
    assert_eq!(sha(&a["data"]), sha(&b["data"]));
    assert_eq!(a["data"], b["data"]);
    // rebuilding the refs (a new store would give new run ids; the same store reuses them) leaves the answer as it was
    fx.build_ref("main");
    assert_eq!(sha(&fx.delta("main", "feat")["data"]), sha(&a["data"]));
    // a cap lists fewer rows, says so, and leaves the summary whole
    let c = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--max-nodes", "1"]);
    assert_eq!(c["data"]["added"].as_array().unwrap().len(), 1);
    assert_eq!(c["data"]["summary"]["added"], 2);
    let lims = c["evidence"]["limitations"].to_string();
    assert!(lims.contains("truncated"), "{lims}");
    err(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--max-nodes", "0"], "invalid_arguments");
    err(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--max-nodes", "2001"], "invalid_arguments");
}

#[test]
fn delta_refuses_a_ref_that_is_unknown_or_not_built() {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/n.py", BAR);
    commit(&d, "feat");
    git(&d, &["checkout", "-q", "main"]);
    fx.build_ref("main");
    let v = err(fx.root(), &["delta", "r", "--base", "main", "--head", "feat"], "not_found");
    assert!(v["error"]["hint"].as_str().unwrap().contains("build r --ref feat"), "{v}");
    assert_eq!(v["error"]["data"]["ref"], "feat");
    let v = err(fx.root(), &["delta", "r", "--base", "main", "--head", "nope"], "not_found");
    assert_eq!(v["error"]["data"]["ref"], "nope");
    err(fx.root(), &["delta", "nope", "--base", "main", "--head", "main"], "not_found");
    // the same ref on both sides is an empty delta, not an error
    let same = run(fx.root(), &["delta", "r", "--base", "main", "--head", "main"]);
    assert_eq!(same["ok"], true);
    assert_eq!(same["data"]["summary"]["head_only"], 0);
    assert!(same["data"]["summary"]["share_already_on_base"].is_null());
}

#[test]
fn a_branch_that_moved_since_its_build_needs_a_new_build() {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    fx.build_ref("feat");
    write(&d, "src/c.py", BAR);
    commit(&d, "feat moves on");
    git(&d, &["checkout", "-q", "main"]);
    fx.build_ref("main");
    err(fx.root(), &["delta", "r", "--base", "main", "--head", "feat"], "not_found");
    fx.build_ref("feat");
    assert_eq!(qns(&fx.delta("main", "feat")["data"]["added"]), vec!["src/c.py::bar"]);
}

#[test]
fn the_mcp_tools_carry_delta_and_ref() {
    use std::io::{BufRead, BufReader, Write};
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{FOO}\n{BAR}"));
    commit(&d, "feat");
    git(&d, &["checkout", "-q", "main"]);
    fx.build_ref("main");
    fx.build_ref("feat");
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("comparegraph")).arg("--root").arg(fx.root()).arg("mcp").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
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
    let d = call("comparegraph_delta", json!({"repo":"r","base":"main","head":"feat"}));
    assert_eq!(d["ok"], true, "{d}");
    assert_eq!(qns(&d["data"]["added"]), vec!["src/a.py::bar"]);
    let s = call("comparegraph_show", json!({"repo":"r","qualified_name":"src/a.py::bar","ref":"feat"}));
    assert_eq!(s["ok"], true, "{s}");
    let s = call("comparegraph_show", json!({"repo":"r","qualified_name":"src/a.py::bar","ref":"main"}));
    assert_eq!(s["error"]["code"], "not_found");
    let s = call("comparegraph_search", json!({"query":"bar","ref":"feat"}));
    assert_eq!(s["error"]["code"], "invalid_arguments");
    drop(child.stdin.take());
    let _ = child.wait();
}
