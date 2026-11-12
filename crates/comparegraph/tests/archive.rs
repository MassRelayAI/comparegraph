//! `archive` / `restore` (soft) and `repos --no-port` (lane CG-STORE B3, B4), on a fixture store.
mod common;
use common::*;
use std::path::Path;

fn listing(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = walk(dir);
    v.sort();
    v
}
fn walk(d: &Path) -> Vec<String> {
    let mut out = vec![];
    for e in std::fs::read_dir(d).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            if e.file_name() != "holders" {
                out.extend(walk(&p));
            }
        } else {
            // SQLite's -wal/-shm side files come and go with connections; the store's content is compared by bytes
            let n = p.display().to_string();
            if !n.ends_with("-wal") && !n.ends_with("-shm") {
                out.push(n);
            }
        }
    }
    out
}
fn policy(w: &Ws, body: &str) -> String {
    let p = w.root().join("policy.json");
    std::fs::write(&p, body).unwrap();
    p.display().to_string()
}
const POLICY_A_PRIMARY: &str = r#"{"schema":"relevance-policy/1","weights":{"use":40,"pull":25,"recency":15,"band":10,"ui":10},
 "saturate":{"evidence_claims":25,"pull_rows":3},"recency":[{"max_days":30,"share":1.0}],"classes":{"keep_min":45,"remove_max":15,"grace_days":14},
 "primaries":[{"repo":"a","domain":"retry","why":"test","by":"t","date":"2026-11-12"}]}"#;

#[test]
fn a_dry_run_writes_nothing() {
    let w = Ws::new();
    w.conn().query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())).unwrap();
    let before = (w.store_bytes(), listing(&w.root().join(".comparegraph")));
    let v = ok(w.root(), &["archive", "b", "--why", "x", "--by", "t"]);
    assert_eq!(v["data"]["dry_run"], true);
    assert_eq!(v["data"]["plan"]["claims"], 1);
    assert_eq!((w.store_bytes(), listing(&w.root().join(".comparegraph"))), before);
}

#[test]
fn the_root_is_refused_with_and_without_a_forced_reason() {
    let w = Ws::new();
    err(w.root(), &["archive", "rootrepo", "--why", "x", "--by", "t", "--apply"], "cannot_archive_root");
    err(w.root(), &["archive", "rootrepo", "--why", "x", "--by", "t", "--apply", "--force-with-reason", "really"], "cannot_archive_root");
}

#[test]
fn a_primary_is_refused_and_a_forced_reason_lands_in_the_manifest_and_the_log() {
    let w = Ws::new();
    let p = policy(&w, POLICY_A_PRIMARY);
    let e = err(w.root(), &["archive", "a", "--why", "x", "--by", "t", "--policy", &p, "--apply"], "archive_refused");
    assert!(e["error"]["data"]["guards"][0].as_str().unwrap().contains("primary"));
    ok(w.root(), &["archive", "a", "--why", "x", "--by", "t", "--policy", &p, "--apply", "--force-with-reason", "founder said so"]);
    let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(w.root().join(".comparegraph/archive/a.json")).unwrap()).unwrap();
    assert_eq!(m["forced"], "founder said so");
    assert_eq!(m["schema"], "archive-manifest/1");
    assert_eq!(m["claims"]["count"], 2);
    let log = std::fs::read_to_string(w.root().join(".comparegraph/actions.jsonl")).unwrap();
    assert!(log.contains("\"forced\":\"founder said so\""));
}

#[test]
fn an_adopted_repo_is_refused_without_a_forced_reason() {
    let w = Ws::new();
    ok(w.root(), &["decide", "b", "adopt", "--why", "use it", "--by", "t"]);
    err(w.root(), &["archive", "b", "--why", "x", "--by", "t", "--apply"], "archive_refused");
}

#[test]
fn no_manifest_on_disk_means_no_flag() {
    use std::os::unix::fs::PermissionsExt;
    let w = Ws::new();
    let dir = w.root().join(".comparegraph/archive");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::write(dir.join("probe"), b"x").is_ok() {
        // permission bits do not bind root
        return;
    }
    let v = run(w.root(), &["archive", "b", "--why", "x", "--by", "t", "--apply"]);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(v["ok"], false, "{v}");
    assert_eq!(w.count("SELECT COUNT(*) FROM meta WHERE key LIKE 'archived:%'"), 0, "a flag without its manifest");
}

#[test]
fn archived_leaves_views_and_check_and_restore_brings_back_byte_equal_answers() {
    let w = Ws::new();
    ok(w.root(), &["relate"]);
    let before: Vec<String> = [vec!["repos"], vec!["matrix", "fam"]].iter().map(|a| ok(w.root(), a)["data"].to_string()).collect();
    let check_before = ok(w.root(), &["check"])["data"]["stale_locations"].clone();
    ok(w.root(), &["archive", "b", "--why", "parked", "--by", "t", "--apply"]);
    // the clone folder may now go
    let moved = w.base.path().join("b-away");
    std::fs::rename(w.clone_dir("b"), &moved).unwrap();
    let c = ok(w.root(), &["check"]);
    assert_eq!(c["data"]["stale_locations"], 0);
    assert_eq!(c["data"]["archived_repos"], 1);
    let m = ok(w.root(), &["matrix", "fam"]);
    assert_eq!(m["data"]["columns"].as_array().unwrap().len(), 2);
    assert!(m["evidence"]["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "archived_skipped"));
    let r = ok(w.root(), &["repos"]);
    assert_eq!(r["data"]["items"].as_array().unwrap().len(), 2);
    assert!(r["evidence"]["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "archived_skipped"));
    assert_eq!(ok(w.root(), &["repos", "--archived"])["data"]["items"][0]["name"], "b");
    assert_eq!(ok(w.root(), &["repos", "--all"])["data"]["items"].as_array().unwrap().len(), 3);
    // history stays readable; writes are refused
    assert_eq!(ok(w.root(), &["claims", "--repo", "b"])["data"]["items"].as_array().unwrap().len(), 1);
    err(w.root(), &["claim", "b", "memory.recall", "unknown", "--author", "t"], "repo_archived");
    err(w.root(), &["build", "b"], "repo_archived");
    err(w.root(), &["decide", "b", "park", "--why", "x", "--by", "t"], "repo_archived");
    // restore needs the folder back at its head
    let e = err(w.root(), &["restore", "b", "--by", "t", "--apply"], "clone_missing");
    assert!(e["error"]["hint"].as_str().unwrap().contains("git clone"));
    std::fs::rename(&moved, w.clone_dir("b")).unwrap();
    ok(w.root(), &["restore", "b", "--by", "t", "--apply"]);
    let after: Vec<String> = [vec!["repos"], vec!["matrix", "fam"]].iter().map(|a| ok(w.root(), a)["data"].to_string()).collect();
    assert_eq!(after, before);
    assert_eq!(ok(w.root(), &["check"])["data"]["stale_locations"], check_before);
    let g = ok(w.root(), &["governance", "verify"]);
    assert_eq!((g["data"]["actions"].as_u64(), g["data"]["archived"].as_u64()), (Some(2), Some(0)));
}

#[test]
fn a_tampered_manifest_is_refused_on_restore_and_a_moved_head_needs_rebuild() {
    let w = Ws::new();
    ok(w.root(), &["archive", "b", "--why", "x", "--by", "t", "--apply"]);
    let m = w.root().join(".comparegraph/archive/b.json");
    let orig = std::fs::read_to_string(&m).unwrap();
    std::fs::write(&m, orig.replace("\"why\": \"x\"", "\"why\": \"z\"")).unwrap();
    err(w.root(), &["restore", "b", "--by", "t", "--apply"], "manifest_mismatch");
    std::fs::write(&m, &orig).unwrap();
    std::fs::write(w.clone_dir("b").join("src/net/c.py"), "def c():\n    return 1\n").unwrap();
    commit_all(&w.clone_dir("b"));
    err(w.root(), &["restore", "b", "--by", "t", "--apply"], "head_mismatch");
    let r = ok(w.root(), &["restore", "b", "--by", "t", "--apply", "--rebuild"]);
    assert!(r["data"]["build"]["run"].is_number(), "{r}");
}

#[test]
fn two_concurrent_archives_of_one_repo_one_wins() {
    let w = Ws::new();
    let root = w.root().to_path_buf();
    let spawn = |root: std::path::PathBuf| {
        std::thread::spawn(move || run(&root, &["archive", "b", "--why", "x", "--by", "t", "--apply"]))
    };
    let (x, y) = (spawn(root.clone()), spawn(root.clone()));
    let (x, y) = (x.join().unwrap(), y.join().unwrap());
    let oks = [&x, &y].iter().filter(|v| v["ok"] == true).count();
    assert_eq!(oks, 1, "{x}\n{y}");
    let loser = if x["ok"] == true { &y } else { &x };
    assert_eq!(loser["error"]["code"], "already_archived");
    ok(&root, &["governance", "verify"]);
}

#[test]
fn no_port_lists_the_agpl_and_study_repos_with_why() {
    let w = Ws::new();
    let v = ok(w.root(), &["repos", "--no-port"]);
    let items = v["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{v}");
    assert_eq!(items[0]["name"], "b");
    let why: Vec<&str> = items[0]["no_port"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    assert_eq!(why, ["lane study", "licence AGPL-3.0"]);
    // a policy may extend the set, never shorten it
    let p = policy(&w, &POLICY_A_PRIMARY.replace("\"primaries\"", "\"licences\":{\"never_copy_extra\":[\"MIT\"]},\"primaries\""));
    assert_eq!(ok(w.root(), &["repos", "--no-port", "--policy", &p])["data"]["items"].as_array().unwrap().len(), 3);
    let bad = policy(&w, &POLICY_A_PRIMARY.replace("\"primaries\"", "\"licences\":{\"never_copy\":[\"MIT\"]},\"primaries\""));
    err(w.root(), &["repos", "--no-port", "--policy", &bad], "invalid_policy");
}
