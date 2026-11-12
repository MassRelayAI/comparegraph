//! `backup` and `restore-store` (lane CG-STORE A1, A2), on a fixture store built in a temp dir.
mod common;
use common::*;
use std::path::{Path, PathBuf};

fn answers(root: &Path) -> Vec<String> {
    [vec!["claims", "--history"], vec!["decisions", "--history"], vec!["relate", "--stored"]]
        .iter()
        .map(|a| ok(root, a)["data"].to_string())
        .collect()
}

#[test]
fn a_full_backup_restores_to_byte_equal_answers() {
    let w = Ws::new();
    ok(w.root(), &["relate"]);
    ok(w.root(), &["decide", "a", "park", "--why", "later", "--by", "t"]);
    let want = answers(w.root());
    let out = tempfile::tempdir().unwrap();
    let b = ok(w.root(), &["backup", "--out", out.path().to_str().unwrap(), "--by", "t"]);
    assert_eq!(b["data"]["verified"], true);
    let file = PathBuf::from(b["data"]["file"].as_str().unwrap());
    let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(b["data"]["manifest"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(manifest["schema"], "backup-manifest/1");
    assert_eq!(manifest["sha256"], b["data"]["sha256"]);
    assert_eq!(manifest["counts"]["claim"], 4);
    assert_eq!(manifest["consistent"], true);
    // a fresh workspace restored from the copy answers byte-equal
    let fresh = tempfile::tempdir().unwrap();
    let plan = ok(fresh.path(), &["restore-store", file.to_str().unwrap()]);
    assert_eq!(plan["data"]["dry_run"], true);
    assert!(!fresh.path().join(".comparegraph/comparegraph.db").exists(), "the dry run wrote a store");
    ok(fresh.path(), &["restore-store", file.to_str().unwrap(), "--apply", "--by", "t"]);
    // family snapshots live beside the store; the claims, decisions and bands come from the copy
    assert_eq!(answers(fresh.path()), want);
}

#[test]
fn a_claims_only_backup_restores_claims_and_a_build_makes_them_checkable() {
    let w = Ws::new();
    ok(w.root(), &["decide", "b", "reject", "--why", "agpl", "--by", "t"]);
    let want = ok(w.root(), &["claims", "--history"])["data"].to_string();
    let out = tempfile::tempdir().unwrap();
    let b = ok(w.root(), &["backup", "--claims-only", "--out", out.path().to_str().unwrap(), "--by", "t"]);
    let file = b["data"]["file"].as_str().unwrap().to_string();
    assert!(file.ends_with(".jsonl"));
    assert_eq!(b["data"]["counts"]["claim"], 4);
    assert!(std::fs::metadata(&file).unwrap().len() < 200_000, "claims-only carries no graph");
    let fresh = tempfile::tempdir().unwrap();
    // the family snapshot files sit beside the store, as in a real workspace
    copy_dir(&w.root().join(".comparegraph/families"), &fresh.path().join(".comparegraph/families"));
    let r = ok(fresh.path(), &["restore-store", &file, "--apply", "--by", "t"]);
    assert!(r["evidence"]["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "graph_not_restored"), "{r}");
    assert_eq!(ok(fresh.path(), &["claims", "--history"])["data"].to_string(), want);
    // runs are back as `restored` with no graph: check names them through unbuilt_claim_runs
    let c = err(fresh.path(), &["check"], "check_failed");
    assert!(c["data"]["unbuilt_claim_runs"].as_u64().unwrap() >= 4, "{c}");
    // rebuild every repo: the claims become checkable again
    for n in ["rootrepo", "a", "b"] {
        ok(fresh.path(), &["build", n]);
    }
    let c = ok(fresh.path(), &["check"]);
    assert_eq!(c["data"]["stale_locations"], 0, "{c}");
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

#[test]
fn keep_two_with_three_present_leaves_two_of_that_kind_only() {
    let w = Ws::new();
    let out = tempfile::tempdir().unwrap();
    let o = out.path().to_str().unwrap();
    ok(w.root(), &["backup", "--claims-only", "--out", o, "--by", "t"]);
    for _ in 0..3 {
        // backups are named by the second: never two in one second
        std::thread::sleep(std::time::Duration::from_millis(1100));
        ok(w.root(), &["backup", "--out", o, "--by", "t", "--keep", "2"]);
    }
    let names: Vec<String> = std::fs::read_dir(out.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    let full = names.iter().filter(|n| n.starts_with("comparegraph-2") && n.ends_with(".db")).count();
    let claims = names.iter().filter(|n| n.starts_with("comparegraph-claims-") && n.ends_with(".jsonl")).count();
    assert_eq!((full, claims), (2, 1), "{names:?}");
    assert_eq!(names.iter().filter(|n| n.ends_with(".manifest.json")).count(), 3);
}

#[test]
fn a_tampered_backup_is_refused_and_the_store_is_untouched() {
    let w = Ws::new();
    let out = tempfile::tempdir().unwrap();
    let b = ok(w.root(), &["backup", "--out", out.path().to_str().unwrap(), "--by", "t"]);
    let file = b["data"]["file"].as_str().unwrap().to_string();
    let mut bytes = std::fs::read(&file).unwrap();
    let n = bytes.len();
    bytes[n - 10] ^= 0xff;
    std::fs::write(&file, bytes).unwrap();
    w.conn().query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())).unwrap();
    let before = std::fs::read(w.db()).unwrap();
    err(w.root(), &["restore-store", &file, "--apply", "--by", "t"], "backup_mismatch");
    assert_eq!(std::fs::read(w.db()).unwrap(), before);
}

#[test]
fn restore_apply_keeps_the_replaced_store_and_writes_a_pre_restore_backup() {
    let w = Ws::new();
    let out = tempfile::tempdir().unwrap();
    let b = ok(w.root(), &["backup", "--out", out.path().to_str().unwrap(), "--by", "t"]);
    // a later claim, which the restore rolls back
    ok(w.root(), &["claim", "b", "memory.recall", "none", "--author", "t", "--searched", "recall turns"]);
    let r = ok(w.root(), &["restore-store", b["data"]["file"].as_str().unwrap(), "--apply", "--by", "t"]);
    let replaced = PathBuf::from(r["data"]["plan"]["replaced_to"].as_str().unwrap());
    assert!(replaced.is_file(), "the replaced store is kept: {r}");
    let pre = PathBuf::from(r["data"]["pre_restore_backup"]["file"].as_str().unwrap());
    assert!(pre.is_file());
    assert_eq!(ok(w.root(), &["claims", "--repo", "b"])["data"]["items"].as_array().unwrap().len(), 1, "the restore rolled back the later claim");
    // the log kept every entry and the restored store points at its head again
    let g = ok(w.root(), &["governance", "verify"]);
    assert!(g["data"]["actions"].as_u64().unwrap() >= 3, "{g}");
}

#[test]
fn restore_refuses_while_the_store_is_open_elsewhere() {
    let w = Ws::new();
    let out = tempfile::tempdir().unwrap();
    let b = ok(w.root(), &["backup", "--out", out.path().to_str().unwrap(), "--by", "t"]);
    let holder = w.conn();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    err(w.root(), &["restore-store", b["data"]["file"].as_str().unwrap(), "--apply", "--by", "t"], "store_busy");
    holder.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn a_backup_never_migrates_a_store_that_predates_this_binary() {
    let w = Ws::new();
    // drop an added column to make the store look like an older binary's (rebuild the table without it)
    let c = w.conn();
    c.execute_batch(
        "PRAGMA foreign_keys=OFF; CREATE TABLE repo_old AS SELECT id,name,path,remote,head,licence,lane,role,category,stars,added_at FROM repo;
         DROP TABLE repo; ALTER TABLE repo_old RENAME TO repo;",
    )
    .unwrap();
    drop(c);
    let cols = |w: &Ws| -> Vec<String> {
        let c = w.conn();
        let mut st = c.prepare("PRAGMA table_info(repo)").unwrap();
        st.query_map([], |r| r.get(1)).unwrap().map(|x| x.unwrap()).collect()
    };
    assert!(!cols(&w).contains(&"moved_to".to_string()));
    let out = tempfile::tempdir().unwrap();
    let b = ok(w.root(), &["backup", "--out", out.path().to_str().unwrap(), "--by", "t"]);
    assert_eq!(b["data"]["verified"], true);
    ok(w.root(), &["backup", "--claims-only", "--out", out.path().to_str().unwrap(), "--by", "t"]);
    ok(w.root(), &["check"]);
    ok(w.root(), &["gc", "--parse-cache"]);
    ok(w.root(), &["gc", "--stale-runs"]);
    ok(w.root(), &["relevance"]);
    ok(w.root(), &["governance", "verify"]);
    assert!(!cols(&w).contains(&"moved_to".to_string()), "a store-safety verb migrated the store");
}

/// The live store: an older binary's `repo` table (columns missing) WITH its partial unique index. The temp view that
/// shadows `repo` made VACUUM fail with `views may not be indexed` (backup, gc --parse-cache).
#[test]
fn a_vacuum_verb_works_on_a_store_whose_indexed_table_predates_columns() {
    let w = Ws::new();
    let c = w.conn();
    c.execute_batch(
        "PRAGMA foreign_keys=OFF; CREATE TABLE repo_old AS SELECT id,name,path,remote,head,licence,lane,role,category,stars,added_at FROM repo;
         DROP TABLE repo; ALTER TABLE repo_old RENAME TO repo;
         CREATE UNIQUE INDEX idx_repo_one_root ON repo(role) WHERE role = 'root';",
    )
    .unwrap();
    drop(c);
    let out = tempfile::tempdir().unwrap();
    let b = ok(w.root(), &["backup", "--out", out.path().to_str().unwrap(), "--by", "t"]);
    assert_eq!(b["data"]["verified"], true);
    ok(w.root(), &["gc", "--parse-cache", "--keep-versions", "2", "--apply", "--by", "t"]);
    ok(w.root(), &["governance", "verify"]);
}
