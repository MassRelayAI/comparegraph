//! `backfill-head` pins a run that predates the head column to a commit, only when EVERY location of the run hashes as
//! recorded at that commit; never a guess (CG-1, D451).
mod common;
use common::*;

fn null_head(w: &Ws, repo: &str) {
    w.conn().execute("UPDATE run SET head = NULL WHERE repo_id = (SELECT id FROM repo WHERE name = ?1)", [repo]).unwrap();
}
fn head_of_run(w: &Ws, repo: &str) -> Option<String> {
    w.conn().query_row("SELECT head FROM run WHERE repo_id = (SELECT id FROM repo WHERE name = ?1)", [repo], |r| r.get(0)).unwrap()
}

#[test]
fn dry_run_names_what_it_would_pin_and_writes_nothing() {
    let w = Ws::new();
    null_head(&w, "a");
    w.conn().query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())).unwrap();
    let before = w.store_bytes();
    let v = ok(w.root(), &["backfill-head"]);
    assert_eq!(v["data"]["dry_run"], true, "{v}");
    assert_eq!(v["data"]["would_pin"].as_array().unwrap().len(), 1, "{v}");
    assert!(v["data"]["unverifiable"].as_array().unwrap().is_empty(), "{v}");
    assert_eq!(head_of_run(&w, "a"), None);
    assert_eq!(w.store_bytes(), before, "a dry run wrote to the store");
}

#[test]
fn apply_pins_when_every_hash_matches_and_the_run_stays_green_after_head_moves() {
    let w = Ws::new();
    null_head(&w, "a");
    let v = ok(w.root(), &["backfill-head", "--apply", "--by", "t"]);
    assert_eq!(v["data"]["pinned"].as_array().unwrap().len(), 1, "{v}");
    assert!(head_of_run(&w, "a").is_some());
    let src: String = w.conn().query_row("SELECT head_source FROM run WHERE repo_id = (SELECT id FROM repo WHERE name = 'a')", [], |r| r.get(0)).unwrap();
    assert_eq!(src, "equivalent-by-hash");
    let d = w.clone_dir("a");
    std::fs::write(d.join("src/net/a.py"), "def retry_with_backoff():\n    return 999\n\ndef recall_turns():\n    return 2\n").unwrap();
    commit_all(&d);
    let c = ok(w.root(), &["check"]);
    assert_eq!(c["data"]["stale_locations"], 0, "{c}");
    assert_eq!(c["data"]["unpinned_runs"], 0, "{c}");
    let g = ok(w.root(), &["governance", "verify"]);
    assert_eq!(g["data"]["actions"], 1, "{g}");
}

#[test]
fn one_mismatching_location_leaves_the_run_unpinned() {
    let w = Ws::new();
    null_head(&w, "a");
    set_location_hash(w.root(), "id = (SELECT MIN(l.id) FROM location l JOIN claim k ON k.id = l.claim_id WHERE k.repo_id = (SELECT id FROM repo WHERE name = 'a'))", "drifted");
    let v = ok(w.root(), &["backfill-head", "--apply", "--by", "t"]);
    assert!(v["data"]["pinned"].as_array().unwrap().is_empty(), "{v}");
    let u = v["data"]["unverifiable"].as_array().unwrap();
    assert_eq!(u.len(), 1, "{v}");
    assert_eq!(u[0]["mismatched"], 1, "{v}");
    assert_eq!(head_of_run(&w, "a"), None, "a guess was written");
}

#[test]
fn apply_needs_by() {
    let w = Ws::new();
    null_head(&w, "a");
    let v = run(w.root(), &["backfill-head", "--apply"]);
    assert_eq!(v["ok"], false, "{v}");
    assert_eq!(head_of_run(&w, "a"), None);
}

/// The pre-lane schema: `run` without `head_source` (a store written by an older binary).
fn drop_head_source(w: &Ws) {
    w.conn().execute("ALTER TABLE run DROP COLUMN head_source", []).unwrap();
}
fn has_head_source(w: &Ws) -> bool {
    w.count("SELECT COUNT(*) FROM pragma_table_info('run') WHERE name = 'head_source'") == 1
}

#[test]
fn an_old_store_is_migrated_by_an_explicit_named_step_on_apply_only() {
    let w = Ws::new();
    drop_head_source(&w);
    null_head(&w, "a");
    w.conn().query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())).unwrap();
    let before = w.store_bytes();
    let d = ok(w.root(), &["backfill-head"]);
    assert_eq!(d["data"]["apply_will_add_column"], "run.head_source", "{d}");
    assert_eq!(d["data"]["would_pin"].as_array().unwrap().len(), 1, "{d}");
    assert_eq!(w.store_bytes(), before, "the dry run is read-only");
    assert!(!has_head_source(&w));
    let a = ok(w.root(), &["backfill-head", "--apply", "--by", "t"]);
    assert_eq!(a["data"]["migrated"][0], "run.head_source", "{a}");
    assert_eq!(a["data"]["pinned"].as_array().unwrap().len(), 1, "{a}");
    assert!(has_head_source(&w));
    assert!(head_of_run(&w, "a").is_some());
    let g = ok(w.root(), &["governance", "verify"]);
    assert_eq!(g["data"]["actions"], 1, "{g}");
    // a second apply pins nothing, migrates nothing and writes no entry
    let b = ok(w.root(), &["backfill-head", "--apply", "--by", "t"]);
    assert!(b["data"]["pinned"].as_array().unwrap().is_empty(), "{b}");
    assert!(b["data"]["migrated"].as_array().unwrap().is_empty(), "{b}");
    let g = ok(w.root(), &["governance", "verify"]);
    assert_eq!(g["data"]["actions"], 1, "no second entry: {g}");
}

#[test]
fn an_archived_repo_is_skipped() {
    let w = Ws::new();
    null_head(&w, "b");
    ok(w.root(), &["archive", "b", "--why", "t", "--by", "t", "--apply"]);
    let v = ok(w.root(), &["backfill-head"]);
    assert_eq!(v["data"]["runs_examined"], 0, "{v}");
}

#[test]
fn a_repo_with_no_recorded_head_is_unverifiable() {
    let w = Ws::new();
    null_head(&w, "a");
    w.conn().execute("UPDATE repo SET head = NULL WHERE name = 'a'", []).unwrap();
    let v = ok(w.root(), &["backfill-head"]);
    assert_eq!(v["data"]["unverifiable"][0]["reason"], "repo_has_no_recorded_head", "{v}");
    assert!(v["data"]["would_pin"].as_array().unwrap().is_empty());
}

#[test]
fn check_shows_an_equivalence_pin() {
    let w = Ws::new();
    null_head(&w, "a");
    ok(w.root(), &["backfill-head", "--apply", "--by", "t"]);
    let c = ok(w.root(), &["check"]);
    assert!(c["data"]["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("equivalent-by-hash")), "{c}");
}
