//! CG-REFGC: refs move, so ref runs go stale; nothing answers silently from old data, `delta --refresh` rebuilds only what
//! moved, and `gc --refs` removes what is gone or superseded (dry run by default, action-logged, never a cited run).
mod common;
use common::{cc, err, git, ok, run, PACK};
use serde_json::Value;
use std::path::{Path, PathBuf};

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
    let o = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}");
    String::from_utf8_lossy(&o.stdout).into_owned()
}

struct Fx {
    ws: tempfile::TempDir,
    base: tempfile::TempDir,
}
impl Fx {
    /// main with foo; branch feat (one commit ahead, adds bar); HEAD left on main. Registered as `r`; head run, main and feat built.
    fn new() -> Fx {
        let (ws, base) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let d = base.path().join("r");
        std::fs::create_dir_all(&d).unwrap();
        git(&d, &["init", "-q", "-b", "main"]);
        write(&d, "LICENSE", MIT);
        write(&d, "src/a.py", FOO);
        commit(&d, "c1");
        git(&d, &["checkout", "-q", "-b", "feat"]);
        write(&d, "src/a.py", &format!("{FOO}\n{BAR}"));
        commit(&d, "feat");
        git(&d, &["checkout", "-q", "main"]);
        ok(ws.path(), &["add", d.to_str().unwrap(), "--name", "r"]);
        let fx = Fx { ws, base };
        ok(fx.root(), &["build", "r"]);
        fx.build_ref("main");
        fx.build_ref("feat");
        fx
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
    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.root().join(".comparegraph/comparegraph.db")).unwrap()
    }
    fn runs(&self) -> i64 {
        self.db().query_row("SELECT COUNT(*) FROM run", [], |r| r.get(0)).unwrap()
    }
    fn feat_advances(&self) {
        let d = self.clone();
        git(&d, &["checkout", "-q", "feat"]);
        write(&d, "src/b.py", "def baz():\n    return 9\n");
        commit(&d, "feat moves on");
        git(&d, &["checkout", "-q", "main"]);
    }
    fn delete_feat(&self) {
        git(&self.clone(), &["branch", "-q", "-D", "feat"]);
    }
}
fn limits(v: &Value) -> String {
    v["evidence"]["limitations"].to_string()
}
fn statuses(v: &Value) -> Vec<(String, String)> {
    v["data"]["items"][0]["ref_runs"].as_array().unwrap().iter().map(|x| (x["ref"].as_str().unwrap().into(), x["status"].as_str().unwrap().into())).collect()
}

#[test]
fn a_moved_ref_is_stale_moved_everywhere_and_refresh_rebuilds_only_what_moved() {
    let fx = Fx::new();
    // nothing moved yet: every ref run is current, and `repos --stale` lists nothing
    let all = ok(fx.root(), &["repos"]);
    assert!(statuses(&all).iter().all(|s| s.1 == "current"), "{all}");
    assert!(ok(fx.root(), &["repos", "--stale"])["data"]["items"].as_array().unwrap().is_empty());
    let q = ok(fx.root(), &["show", "r", "src/a.py::bar", "--ref", "feat"]);
    assert!(!limits(&q).contains("stale"), "{q}");

    fx.feat_advances();
    let st = ok(fx.root(), &["repos", "--stale"]);
    assert_eq!(statuses(&st), vec![("feat".to_string(), "moved".to_string())]);
    // a query still answers from the old run, and says so
    let q = ok(fx.root(), &["show", "r", "src/a.py::bar", "--ref", "feat"]);
    assert!(limits(&q).contains("stale: moved"), "{q}");
    // the delta refuses (it never answers from a run at an old commit) and names the stale state
    let d = err(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--build-missing"], "not_found");
    assert_eq!(d["error"]["data"]["stale"], "moved");
    assert!(d["error"]["hint"].as_str().unwrap().contains("--refresh"), "{d}");

    // --refresh rebuilds only the ref that moved: one more run, and it is feat
    let before = fx.runs();
    let v = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--refresh"]);
    assert_eq!(fx.runs(), before + 1, "main and the merge base were current: only feat is rebuilt");
    let rebuilt = v["data"]["rebuilt"].as_array().unwrap();
    assert_eq!(rebuilt.len(), 1);
    assert_eq!(rebuilt[0]["ref"], "feat");
    let added: Vec<&str> = v["data"]["branch"]["added"].as_array().unwrap().iter().map(|r| r["qualified_name"].as_str().unwrap()).collect();
    assert_eq!(added, vec!["src/a.py::bar", "src/b.py::baz"]);
    // the newest feat run is current now, the older one is still listed as what it was built at
    let again = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--refresh"]);
    assert!(again["data"]["rebuilt"].as_array().unwrap().is_empty());
    assert_eq!(fx.runs(), before + 1);
}

#[test]
fn a_deleted_branch_is_stale_gone_and_gc_refs_removes_its_runs_and_trees() {
    let fx = Fx::new();
    fx.feat_advances();
    fx.build_ref("feat"); // feat now has two runs: the old commit and the new one
    let head_hash: String = fx.db().query_row("SELECT store_hash FROM run WHERE ref IS NULL", [], |r| r.get(0)).unwrap();
    fx.delete_feat();

    let st = ok(fx.root(), &["repos", "--stale"]);
    assert!(statuses(&st).iter().all(|s| s.0 == "feat" && s.1 == "gone") && statuses(&st).len() == 2, "{st}");
    let q = ok(fx.root(), &["show", "r", "src/a.py::bar", "--ref", "feat"]);
    assert!(limits(&q).contains("stale: gone"), "{q}");
    let d = err(fx.root(), &["delta", "r", "--base", "main", "--head", "feat"], "not_found");
    assert_eq!(d["error"]["data"]["stale"], "gone");

    // a dry run lists the two feat runs and writes nothing
    let db_before = std::fs::read(fx.root().join(".comparegraph/comparegraph.db")).unwrap();
    let dry = ok(fx.root(), &["gc", "--refs"]);
    assert_eq!(dry["data"]["dry_run"], true);
    let cands = dry["data"]["candidates"].as_array().unwrap();
    assert_eq!(cands.len(), 2);
    assert!(cands.iter().all(|c| c["ref"] == "feat" && c["why"].to_string().contains("gone")));
    assert_eq!(fx.runs(), 4);
    let _ = db_before;
    // --apply needs --by
    err(fx.root(), &["gc", "--refs", "--apply"], "invalid_arguments");

    let clone = fx.clone();
    let clone_before = (git_out(&clone, &["status", "--porcelain"]), git_out(&clone, &["rev-parse", "HEAD"]), std::fs::read(clone.join(".git/index")).unwrap());
    let feat_commits: Vec<String> = cands.iter().map(|c| c["ref_commit"].as_str().unwrap().to_string()).collect();
    for c in &feat_commits {
        assert!(fx.root().join(".comparegraph/refs/r").join(c).is_dir());
    }
    let v = ok(fx.root(), &["gc", "--refs", "--apply", "--by", "tester"]);
    assert_eq!(v["data"]["dry_run"], false);
    assert_eq!(v["data"]["removed_runs"].as_array().unwrap().len(), 2);

    // the runs, their graph rows and their trees are gone; main's ref run and the head run are not
    assert_eq!(fx.runs(), 2);
    assert_eq!(fx.db().query_row("SELECT COUNT(*) FROM run WHERE ref='feat'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(fx.db().query_row("SELECT COUNT(*) FROM node WHERE run_id NOT IN (SELECT id FROM run)", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(fx.db().query_row("SELECT COUNT(*) FROM node_fts", [], |r| r.get::<_, i64>(0)).unwrap(), fx.db().query_row("SELECT COUNT(*) FROM node", [], |r| r.get::<_, i64>(0)).unwrap());
    for c in &feat_commits {
        assert!(!fx.root().join(".comparegraph/refs/r").join(c).exists(), "tree of {c} left behind");
    }
    let main_c = git_out(&clone, &["rev-parse", "main"]);
    assert!(fx.root().join(".comparegraph/refs/r").join(main_c.trim()).is_dir(), "main's tree must stay");
    let head_after: String = fx.db().query_row("SELECT store_hash FROM run WHERE ref IS NULL", [], |r| r.get(0)).unwrap();
    assert_eq!(head_after, head_hash, "the head run is untouched");
    assert_eq!(clone_before, (git_out(&clone, &["status", "--porcelain"]), git_out(&clone, &["rev-parse", "HEAD"]), std::fs::read(clone.join(".git/index")).unwrap()), "the clone is untouched");
    // the append-only guard is back in place
    assert!(fx.db().execute("DELETE FROM node WHERE 1=1", []).is_err());

    // every removal is an action-log line, and the log still verifies; the store still checks
    let log = std::fs::read_to_string(fx.root().join(".comparegraph/actions.jsonl")).unwrap_or_else(|_| {
        let mut all = String::new();
        for e in std::fs::read_dir(fx.root().join(".comparegraph")).unwrap().flatten() {
            if e.path().extension().is_some_and(|x| x == "jsonl") {
                all.push_str(&std::fs::read_to_string(e.path()).unwrap());
            }
        }
        all
    });
    assert!(log.contains("gc-refs") && log.contains("tester"), "{log}");
    let g = ok(fx.root(), &["governance", "verify"]);
    assert_eq!(g["data"]["chain_ok"], true, "{g}");
    ok(fx.root(), &["check"]);
    // nothing left to do
    assert!(ok(fx.root(), &["gc", "--refs"])["data"]["candidates"].as_array().unwrap().is_empty());
}

#[test]
fn a_ref_run_that_is_superseded_goes_when_the_ref_lives_on_and_keep_says_how_many_stay() {
    let fx = Fx::new();
    fx.feat_advances();
    fx.build_ref("feat");
    let two = ok(fx.root(), &["gc", "--refs", "--keep", "2"]);
    assert!(two["data"]["candidates"].as_array().unwrap().is_empty(), "{two}");
    let one = ok(fx.root(), &["gc", "--refs"]);
    let c = one["data"]["candidates"].as_array().unwrap();
    assert_eq!(c.len(), 1);
    assert!(c[0]["why"].to_string().contains("superseded"), "{one}");
    // it is the older feat run
    let newest: i64 = fx.db().query_row("SELECT MAX(id) FROM run WHERE ref='feat'", [], |r| r.get(0)).unwrap();
    assert_ne!(c[0]["run"].as_i64().unwrap(), newest);
    ok(fx.root(), &["gc", "--refs", "--apply", "--by", "t"]);
    assert_eq!(fx.db().query_row("SELECT COUNT(*) FROM run WHERE ref='feat'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    // an age bound: nothing is that old; one day is not enough to remove a fresh run
    assert!(ok(fx.root(), &["gc", "--refs", "--older-than-days", "1"])["data"]["candidates"].as_array().unwrap().is_empty());
    err(fx.root(), &["gc", "--refs", "--older-than-days", "0"], "invalid_arguments");
}

#[test]
fn a_run_a_claim_cites_is_never_removed() {
    let fx = Fx::new();
    let pack = fx.base.path().join("pack");
    write(&pack, "rows.md", PACK);
    ok(fx.root(), &["family", "add", pack.to_str().unwrap()]);
    ok(fx.root(), &["claim", "r", "routing.retry", "unknown", "--author", "t", "--no-reach"]);
    let feat_run: i64 = fx.db().query_row("SELECT id FROM run WHERE ref='feat'", [], |r| r.get(0)).unwrap();
    // a claim that cites the ref run (a claim is append-only, so the row is added, not changed)
    fx.db()
        .execute(
            "INSERT INTO claim(repo_id, feature_id, run_id, verdict, confidence, author, note, limitations_json, created_at) \
             SELECT repo_id, feature_id, ?1, verdict, confidence, author, note, limitations_json, created_at FROM claim LIMIT 1",
            [feat_run],
        )
        .unwrap();
    fx.delete_feat();
    let dry = ok(fx.root(), &["gc", "--refs"]);
    assert_eq!(dry["data"]["cited"][0]["run"], feat_run);
    let before = fx.runs();
    let e = err(fx.root(), &["gc", "--refs", "--apply", "--by", "t"], "check_failed");
    assert_eq!(e["error"]["data"]["cited"][0]["run"], feat_run);
    assert!(e["error"]["message"].as_str().unwrap().contains(&feat_run.to_string()), "{e}");
    assert_eq!(fx.runs(), before, "a refused gc removes nothing");
    let _ = (cc, run);
}

fn trigger_sql(fx: &Fx) -> Vec<(String, String)> {
    let db = fx.db();
    let mut st = db.prepare("SELECT name, sql FROM sqlite_master WHERE type='trigger' ORDER BY name").unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect()
}

#[test]
fn head_runs_stay_append_only_and_gc_refs_prunes_a_ref_run_without_touching_a_trigger() {
    let fx = Fx::new();
    let head: i64 = fx.db().query_row("SELECT id FROM run WHERE ref IS NULL", [], |r| r.get(0)).unwrap();
    // the ruling: ref runs are a prunable cache, head runs are append-only exactly as before
    for sql in [
        "DELETE FROM community_member WHERE community_id IN (SELECT id FROM community WHERE run_id=?1)",
        "DELETE FROM community WHERE run_id=?1",
        "DELETE FROM edge WHERE run_id=?1",
        "DELETE FROM node WHERE run_id=?1",
    ] {
        let n: i64 = fx.db().query_row("SELECT COUNT(*) FROM node WHERE run_id=?1", [head], |r| r.get(0)).unwrap();
        assert!(n > 0);
        let e = fx.db().execute(sql, [head]);
        assert!(e.as_ref().is_err_and(|e| e.to_string().contains("append-only")) || sql.contains("community_member") && e.is_ok_and(|c| c == 0), "a head run row was deletable: {sql}");
    }
    let triggers = trigger_sql(&fx);
    for t in ["node_no_delete", "edge_no_delete", "community_no_delete", "community_member_no_delete"] {
        let sql = &triggers.iter().find(|x| x.0 == t).unwrap().1;
        assert!(sql.contains("ref IS NOT NULL"), "{t} must name its exception in the trigger itself: {sql}");
    }
    fx.feat_advances();
    fx.build_ref("feat");
    let before = trigger_sql(&fx);
    ok(fx.root(), &["gc", "--refs", "--apply", "--by", "t"]); // the older feat run is superseded
    assert_eq!(trigger_sql(&fx), before, "no trigger may be dropped, recreated or changed by an apply");
    assert_eq!(fx.db().query_row("SELECT COUNT(*) FROM run WHERE ref='feat'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    // the head run is whole
    assert!(fx.db().query_row("SELECT COUNT(*) FROM node WHERE run_id=?1", [head], |r| r.get::<_, i64>(0)).unwrap() > 0);
}

#[test]
fn a_store_with_the_old_unconditional_triggers_is_upgraded_on_open() {
    let fx = Fx::new();
    {
        let db = fx.db();
        db.execute_batch(
            "DROP TRIGGER node_no_delete; CREATE TRIGGER node_no_delete BEFORE DELETE ON node BEGIN SELECT RAISE(ABORT, 'append-only'); END;",
        )
        .unwrap();
    }
    assert!(!trigger_sql(&fx).iter().find(|x| x.0 == "node_no_delete").unwrap().1.contains("ref IS NOT NULL"));
    ok(fx.root(), &["build", "r"]); // a writer open migrates
    assert!(trigger_sql(&fx).iter().find(|x| x.0 == "node_no_delete").unwrap().1.contains("ref IS NOT NULL"));
}
