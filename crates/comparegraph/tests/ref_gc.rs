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
/// Defensive only: `claim` refuses `--ref`, so no claim cites a ref run through the CLI (see `a_claim_never_cites_a_ref_run`); a row inserted by hand is still protected.
fn a_run_a_claim_cites_is_never_removed() {
    let fx = Fx::new();
    fx.feat_advances();
    fx.build_ref("feat"); // feat: an older run (cited below) and a newer one (not cited)
    let pack = fx.base.path().join("pack");
    write(&pack, "rows.md", PACK);
    ok(fx.root(), &["family", "add", pack.to_str().unwrap()]);
    ok(fx.root(), &["claim", "r", "routing.retry", "unknown", "--author", "t", "--no-reach"]);
    let feat_run: i64 = fx.db().query_row("SELECT MIN(id) FROM run WHERE ref='feat'", [], |r| r.get(0)).unwrap();
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
    // MINOR: the cited run is skipped and said so; the apply goes on with the rest
    let v = ok(fx.root(), &["gc", "--refs", "--apply", "--by", "t"]);
    assert_eq!(v["data"]["cited"][0]["run"], feat_run);
    assert!(limits(&v).contains(&feat_run.to_string()), "{v}");
    let left: Vec<i64> = {
        let db = fx.db();
        let mut st = db.prepare("SELECT id FROM run WHERE ref='feat'").unwrap();
        st.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
    };
    assert_eq!(left, vec![feat_run], "the cited run stays, the other gone feat run goes");
    assert_eq!(fx.runs(), before - 1);
    let _ = (cc, run);
}

/// MINOR: `gc --refs` removes the newest run and node rows; their ids are never handed out again.
#[test]
fn ids_of_removed_ref_runs_are_never_reused() {
    let fx = Fx::new();
    let max = |t: &str| fx.db().query_row(&format!("SELECT MAX(id) FROM {t}"), [], |r| r.get::<_, i64>(0)).unwrap();
    let (run_hi, node_hi) = (max("run"), max("node"));
    assert_eq!(fx.db().query_row("SELECT ref FROM run WHERE id=?1", [run_hi], |r| r.get::<_, String>(0)).unwrap(), "feat");
    fx.delete_feat();
    ok(fx.root(), &["gc", "--refs", "--apply", "--by", "t"]);
    assert!(max("run") < run_hi, "the newest run was removed");
    let d = fx.clone();
    git(&d, &["checkout", "-q", "-b", "other"]);
    write(&d, "src/o.py", "def other(x):\n    return x - 1\n");
    commit(&d, "other");
    git(&d, &["checkout", "-q", "main"]);
    let v = fx.build_ref("other");
    assert!(v["data"]["run"].as_i64().unwrap() > run_hi, "run id {} reused (removed up to {run_hi})", v["data"]["run"]);
    let low: i64 = fx.db().query_row("SELECT MIN(id) FROM node WHERE run_id=?1", [v["data"]["run"].as_i64().unwrap()], |r| r.get(0)).unwrap();
    assert!(low > node_hi, "node id {low} reused (removed up to {node_hi})");
}

/// MINOR: a `--ref` that is not a ref name is never a LIKE pattern over commits (`_` and `%` match anything there).
#[test]
fn a_ref_of_wildcards_names_no_run() {
    let fx = Fx::new();
    err(fx.root(), &["show", "r", "src/a.py::bar", "--ref", "_______"], "not_found");
    err(fx.root(), &["show", "r", "src/a.py::bar", "--ref", "%%%%%%%"], "not_found");
    let c = git_out(&fx.clone(), &["rev-parse", "feat"]);
    ok(fx.root(), &["show", "r", "src/a.py::bar", "--ref", &c.trim()[..9]]); // a real commit prefix still names the run
}

/// MINOR: relevance's deterministic "now" is the newest claim or HEAD run; a ref run (a branch cache) does not move it.
#[test]
fn a_ref_run_does_not_move_the_store_s_newest_timestamp() {
    let fx = Fx::new();
    fx.db().execute("UPDATE run SET started_at='2099-01-01T00:00:00Z' WHERE ref IS NOT NULL", []).unwrap();
    let t = comparegraph::store::Store::open_existing(fx.root()).unwrap().newest_timestamp().unwrap().unwrap();
    assert!(t.as_str() < "2099", "{t}");
}

/// MINOR: COMPAREGRAPH_PREV_RUN is a test hook of ref builds only; a head build takes its own previous run.
#[test]
fn the_previous_run_hook_is_ignored_by_a_head_build() {
    let fx = Fx::new();
    let head: i64 = fx.db().query_row("SELECT id FROM run WHERE ref IS NULL", [], |r| r.get(0)).unwrap();
    let feat: i64 = fx.db().query_row("SELECT id FROM run WHERE ref='feat'", [], |r| r.get(0)).unwrap();
    write(&fx.clone(), "src/c.py", "def c(x):\n    return x * 3\n");
    let o = cc(fx.root()).args(["build", "r"]).env("COMPAREGRAPH_PREV_RUN", feat.to_string()).output().unwrap();
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["data"]["incremental_from"].as_i64(), Some(head), "{v}");
}

/// MINOR: `decide --summary-from-delta` with a short commit that names two recorded deltas is refused, never the last one.
/// (Review round 2: the commit must now be one feat reaches, so the two recorded deltas share the prefix of feat's real sha.)
#[test]
fn an_ambiguous_short_commit_for_a_branch_verdict_is_refused() {
    let fx = Fx::new();
    let sha = git_out(&fx.clone(), &["rev-parse", "feat"]).trim().to_string();
    // round 3: the stored delta must have used the base the decision names (`--base`); round 4 (M2): and this engine, provider
    // and model, so the full sha's record is a real delta, and only the second, faked row shares its prefix
    ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--build-missing"]);
    let fake = format!("{}zzzz", &sha[..7]);
    fx.db().execute("INSERT INTO meta(key, value) VALUES (?1, ?2)", [format!("delta:r:{fake}"), "{\"base_ref\":\"main\",\"summary\":{}}".to_string()]).unwrap();
    let e = err(fx.root(), &["decide", "r", "park", "--why", "w", "--by", "t", "--ref", "feat", "--commit", &sha[..7], "--summary-from-delta", "--base", "main"], "invalid_arguments");
    assert!(e["error"]["message"].as_str().unwrap().contains("ambiguous"), "{e}");
    ok(fx.root(), &["decide", "r", "park", "--why", "w", "--by", "t", "--ref", "feat", "--commit", &sha, "--summary-from-delta", "--base", "main"]);
}

/// MINOR: the meta row `delta` writes is an action-log line, and the log still verifies.
#[test]
fn delta_s_record_of_its_summary_is_an_action_log_line() {
    let fx = Fx::new();
    ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat"]);
    let log = std::fs::read_to_string(fx.root().join(".comparegraph/actions.jsonl")).unwrap_or_default();
    assert!(log.lines().any(|l| l.contains("\"delta-record\"")), "{log}");
    assert_eq!(ok(fx.root(), &["governance", "verify"])["data"]["chain_ok"], true);
}

/// MAJOR: a clone that cannot be read (moved away, unmounted, git not on PATH) says nothing about its refs: every ref run is
/// `unknown`, never `gone`, and `gc --refs --apply` removes nothing of it.
#[test]
fn an_unreachable_clone_makes_its_ref_runs_unknown_and_gc_removes_nothing() {
    let fx = Fx::new();
    fx.delete_feat(); // even feat, really gone, cannot be told apart while the clone is unreadable
    let clone = fx.clone();
    let away = clone.with_file_name("r-away");
    std::fs::rename(&clone, &away).unwrap();
    let st = ok(fx.root(), &["repos", "--stale"]);
    let s = statuses(&st);
    assert!(s.len() == 2 && s.iter().all(|x| x.1 == "unknown"), "{st}");
    let runs = fx.runs();
    let v = ok(fx.root(), &["gc", "--refs", "--apply", "--by", "t"]);
    assert_eq!(fx.runs(), runs, "nothing may be removed while the clone cannot be read: {v}");
    assert!(v["data"]["removed_runs"].as_array().unwrap().is_empty(), "{v}");
    assert_eq!(v["data"]["unknown"].as_array().unwrap().len(), 2, "{v}");
    assert!(limits(&v).contains("unknown"), "gc must say what it skipped: {v}");
    let q = ok(fx.root(), &["show", "r", "src/a.py::bar", "--ref", "feat"]);
    assert!(limits(&q).contains("stale: unknown"), "{q}");

    // git not on PATH: the same
    std::fs::rename(&away, &clone).unwrap();
    let o = cc(fx.root()).args(["gc", "--refs", "--apply", "--by", "t"]).env("PATH", "").output().unwrap();
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(fx.runs(), runs, "git not runnable: nothing removed: {v}");

    // the clone is readable again: feat is gone and goes, main stays
    let v = ok(fx.root(), &["gc", "--refs", "--apply", "--by", "t"]);
    assert_eq!(v["data"]["removed_runs"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(fx.runs(), runs - 1);
}

/// M5: a run's `ref` decides whether its rows may be deleted, so it is fixed once the run has rows or is done.
#[test]
fn a_run_s_ref_cannot_be_changed_once_the_run_has_rows() {
    let fx = Fx::new();
    let db = fx.db();
    let head: i64 = db.query_row("SELECT id FROM run WHERE ref IS NULL", [], |r| r.get(0)).unwrap();
    let feat: i64 = db.query_row("SELECT id FROM run WHERE ref='feat'", [], |r| r.get(0)).unwrap();
    for (sql, id) in [
        ("UPDATE run SET ref='x' WHERE id=?1", head),
        ("UPDATE run SET ref_commit='0123456789abcdef0123456789abcdef01234567' WHERE id=?1", head),
        ("UPDATE run SET ref=NULL, ref_commit=NULL WHERE id=?1", feat),
        ("UPDATE run SET ref='main' WHERE id=?1", feat),
    ] {
        let e = db.execute(sql, [id]);
        assert!(e.as_ref().is_err_and(|e| e.to_string().contains("run.ref")), "{sql} on run {id} was allowed: {e:?}");
    }
    // so a head run's rows stay append-only
    assert!(db.execute("DELETE FROM node WHERE run_id=?1", [head]).is_err());
    // the other columns of a run stay writable (finish_run, set_run_head)
    db.execute("UPDATE run SET head=head WHERE id=?1", [head]).unwrap();
}

/// M5: `claim` itself refuses to cite a ref run, whatever pinned the run choice to a ref.
#[test]
fn a_claim_while_a_ref_is_pinned_is_refused_by_claim_itself() {
    use comparegraph::application::claim::{claim_batch_with, BatchRow, ClaimOut};
    let fx = Fx::new();
    let pack = fx.base.path().join("pack");
    write(&pack, "rows.md", PACK);
    ok(fx.root(), &["family", "add", pack.to_str().unwrap()]);
    let row = "{\"repo\":\"r\",\"feature\":\"routing.retry\",\"verdict\":\"unknown\",\"author\":\"t\",\"no_reach\":true}\n";
    let mut errors: Vec<String> = vec![];
    let _ = comparegraph::store::with_run_ref(Some("feat"), || {
        claim_batch_with(fx.root(), row.as_bytes(), true, |r: BatchRow<ClaimOut>| errors.extend(r.error.map(|e| e.message)))
    });
    assert!(errors.iter().any(|m| m.contains("ref run")), "{errors:?}");
    assert_eq!(fx.db().query_row("SELECT COUNT(*) FROM claim c JOIN run u ON u.id = c.run_id WHERE u.ref IS NOT NULL", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
}

fn trigger_sql(fx: &Fx) -> Vec<(String, String)> {
    let db = fx.db();
    let mut st = db.prepare("SELECT name, sql FROM sqlite_master WHERE type='trigger' ORDER BY name").unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect()
}

#[test]
fn head_runs_stay_append_only_and_gc_refs_prunes_a_ref_run_without_touching_a_trigger() {
    let fx = Fx::new();
    // a head run with rows in all four tables: two files that call each other form a community with members
    let d = fx.clone();
    write(&d, "src/c.py", "from src.d import g

def f(x):
    return g(x) + 1
");
    write(&d, "src/d.py", "from src.c import f

def g(x):
    return f(x) * 2
");
    commit(&d, "two modules that call each other");
    ok(fx.root(), &["build", "r"]);
    let head: i64 = fx.db().query_row("SELECT MAX(id) FROM run WHERE ref IS NULL", [], |r| r.get(0)).unwrap();
    // the ruling: ref runs are a prunable cache, head runs are append-only exactly as before
    for (sql, rows) in [
        ("DELETE FROM community_member WHERE community_id IN (SELECT id FROM community WHERE run_id=?1)", "SELECT COUNT(*) FROM community_member WHERE community_id IN (SELECT id FROM community WHERE run_id=?1)"),
        ("DELETE FROM community WHERE run_id=?1", "SELECT COUNT(*) FROM community WHERE run_id=?1"),
        ("DELETE FROM edge WHERE run_id=?1", "SELECT COUNT(*) FROM edge WHERE run_id=?1"),
        ("DELETE FROM node WHERE run_id=?1", "SELECT COUNT(*) FROM node WHERE run_id=?1"),
    ] {
        let n: i64 = fx.db().query_row(rows, [head], |r| r.get(0)).unwrap();
        assert!(n > 0, "the head run has no rows to protect for: {sql}");
        let e = fx.db().execute(sql, [head]);
        assert!(e.as_ref().is_err_and(|e| e.to_string().contains("append-only")), "a head run row was deletable: {sql}: {e:?}");
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

#[test]
fn a_claim_never_cites_a_ref_run() {
    let fx = Fx::new();
    let pack = fx.base.path().join("pack");
    write(&pack, "rows.md", PACK);
    ok(fx.root(), &["family", "add", pack.to_str().unwrap()]);
    let e = err(fx.root(), &["claim", "r", "routing.retry", "unknown", "--author", "t", "--ref", "feat"], "invalid_arguments");
    assert!(e["error"]["message"].as_str().unwrap().contains("ref run"), "{e}");
    assert_eq!(e["error"]["data"]["ref"], "feat");
    assert!(e["error"]["hint"].as_str().unwrap().contains("decide --ref"), "{e}");
    // a batch row with a ref is refused row by row
    let f = fx.base.path().join("rows.jsonl");
    std::fs::write(&f, "{\"repo\":\"r\",\"feature\":\"routing.retry\",\"verdict\":\"unknown\",\"author\":\"t\",\"ref\":\"feat\"}\n").unwrap();
    let o = cc(fx.root()).args(["claim", "--batch", f.to_str().unwrap()]).output().unwrap();
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(!o.status.success() && text.contains("ref run"), "{text}");
    // and nothing cites a ref run
    assert_eq!(fx.db().query_row("SELECT COUNT(*) FROM claim c JOIN run u ON u.id = c.run_id WHERE u.ref IS NOT NULL", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    // the head claim still works
    ok(fx.root(), &["claim", "r", "routing.retry", "unknown", "--author", "t", "--no-reach"]);
}

#[test]
fn a_branch_verdict_names_ref_and_commit_by_value_and_survives_gc_of_the_ref() {
    let fx = Fx::new();
    let clone = fx.clone();
    let sha = git_out(&clone, &["rev-parse", "feat"]).trim().to_string();
    // the numbers come from the last delta; none computed yet is a refusal, a missing commit too
    // round 3: --summary-from-delta names the base the delta used (`--base`)
    err(fx.root(), &["decide", "r", "park", "--why", "w", "--by", "t", "--ref", "feat", "--commit", &sha, "--summary-from-delta", "--base", "main"], "not_found");
    err(fx.root(), &["decide", "r", "park", "--why", "w", "--by", "t", "--ref", "feat"], "invalid_arguments");
    ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat"]);
    let d = ok(fx.root(), &["decide", "r", "reject", "--why", "already on main", "--by", "tester", "--ref", "feat", "--commit", &sha, "--summary-from-delta", "--base", "main", "--bundle", "D:/bundles/feat.bundle"]);
    assert_eq!(d["data"]["scope"], "ref");
    assert_eq!(d["data"]["ref"], "feat");
    assert_eq!(d["data"]["ref_commit"], sha.as_str());
    assert_eq!(d["data"]["bundle"], "D:/bundles/feat.bundle"); // a string: the file does not exist and is never read
    assert_eq!(d["data"]["summary"]["summary"]["branch_symbols"], 1);
    // a later verdict on the same branch supersedes the first; a verdict on another branch does not
    let d2 = ok(fx.root(), &["decide", "r", "adopt", "--why", "after all", "--by", "tester", "--ref", "feat", "--commit", &sha]);
    assert_eq!(d2["data"]["supersedes_id"], d["data"]["id"]);
    // a branch verdict is not a repo adoption: nothing treats the repo as adopted
    let guard = fx.db().query_row("SELECT COUNT(*) FROM decision WHERE scope='repo'", [], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(guard, 0);
    let latest = ok(fx.root(), &["decisions", "--ref", "feat"]);
    assert_eq!(latest["data"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(latest["data"]["items"][0]["disposition"], "adopt");
    assert_eq!(ok(fx.root(), &["decisions", "--ref", "feat", "--history"])["data"]["items"].as_array().unwrap().len(), 2);
    assert!(ok(fx.root(), &["decisions", "--ref", "other"])["data"]["items"].as_array().unwrap().is_empty());

    // the bundle is written outside CompareGraph, the branch is deleted, and its data goes; the verdicts stay
    fx.delete_feat();
    ok(fx.root(), &["gc", "--refs", "--apply", "--by", "t"]);
    assert_eq!(fx.db().query_row("SELECT COUNT(*) FROM run WHERE ref='feat'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    let after = ok(fx.root(), &["decisions", "--ref", "feat", "--history"]);
    assert_eq!(after["data"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(after["data"]["items"][1]["ref_commit"], sha.as_str());
    assert_eq!(after["data"]["items"][0]["bundle"], "D:/bundles/feat.bundle");
    // the append-only guard of decisions holds for the new columns too
    assert!(fx.db().execute("UPDATE decision SET bundle='x'", []).is_err());
    ok(fx.root(), &["governance", "verify"]);
}
