//! `gc --parse-cache`, prune-on-build and `gc --stale-runs` (lane CG-STORE A4, A5), on a fixture store.
mod common;
use common::*;

/// Copy every cache row under another provider version, as an engine version bump leaves them.
fn add_old_version(w: &Ws, v: &str) -> i64 {
    let c = w.conn();
    c.execute(
        "INSERT INTO parse_cache(file_hash, language, provider_version, payload) SELECT file_hash, language, ?1, payload FROM parse_cache WHERE provider_version NOT LIKE '0.0.%'",
        [v],
    )
    .unwrap();
    w.count(&format!("SELECT COUNT(*) FROM parse_cache WHERE provider_version = '{v}'"))
}
fn latest_hashes(w: &Ws) -> Vec<(String, Option<String>)> {
    ok(w.root(), &["repos"])["data"]["items"].as_array().unwrap().iter().map(|r| (r["name"].as_str().unwrap().to_string(), r["latest_run"]["store_hash"].as_str().map(String::from))).collect()
}

#[test]
fn dry_run_changes_nothing_and_apply_keeps_exactly_the_current_version() {
    let w = Ws::new();
    let n_old = add_old_version(&w, "0.0.1+old");
    assert!(n_old > 0);
    add_old_version(&w, "0.0.2+older");
    w.conn().query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())).unwrap();
    let before = w.store_bytes();
    let hashes = latest_hashes(&w);
    let d = ok(w.root(), &["gc", "--parse-cache"]);
    assert_eq!(d["data"]["dry_run"], true);
    assert_eq!(d["data"]["bytes_after_is"], "estimated");
    assert_eq!(d["data"]["drop"].as_array().unwrap().len(), 2, "{d}");
    assert_eq!(w.store_bytes(), before, "the dry run wrote");
    err(w.root(), &["gc", "--parse-cache", "--apply"], "invalid_arguments");
    let a = ok(w.root(), &["gc", "--parse-cache", "--apply", "--by", "t"]);
    assert_eq!(a["data"]["bytes_after_is"], "measured");
    assert_eq!(a["data"]["rows_deleted"].as_i64().unwrap(), 2 * n_old);
    let current = a["data"]["current_version"].as_str().unwrap().to_string();
    assert_eq!(w.count("SELECT COUNT(DISTINCT provider_version) FROM parse_cache"), 1);
    assert_eq!(w.count(&format!("SELECT COUNT(*) FROM parse_cache WHERE provider_version != '{current}'")), 0);
    assert_eq!(a["data"]["bytes_after"].as_u64().unwrap(), std::fs::metadata(w.db()).unwrap().len(), "bytes_after is the file size");
    assert_eq!(latest_hashes(&w), hashes, "a store_hash moved");
    ok(w.root(), &["check", "--deep"]);
    let g = ok(w.root(), &["governance", "verify"]);
    assert_eq!(g["data"]["actions"], 1);
}

#[test]
fn keep_versions_two_keeps_two_and_all_cache_drops_everything() {
    let w = Ws::new();
    add_old_version(&w, "0.0.1+old");
    add_old_version(&w, "0.0.2+newer");
    ok(w.root(), &["gc", "--parse-cache", "--keep-versions", "2", "--apply", "--by", "t"]);
    let mut st = w.conn();
    let vs: Vec<String> = {
        let tx = st.transaction().unwrap();
        let mut q = tx.prepare("SELECT DISTINCT provider_version FROM parse_cache ORDER BY 1").unwrap();
        let v = q.query_map([], |r| r.get(0)).unwrap().map(|x| x.unwrap()).collect();
        v
    };
    assert_eq!(vs.len(), 2, "{vs:?}");
    assert!(vs.contains(&"0.0.2+newer".to_string()), "the newest other version is the one kept: {vs:?}");
    let v = ok(w.root(), &["gc", "--parse-cache", "--all-cache", "--apply", "--by", "t"]);
    assert!(v["evidence"]["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "cache_dropped"));
    assert_eq!(w.count("SELECT COUNT(*) FROM parse_cache"), 0);
}

#[test]
fn a_build_after_gc_reproduces_the_same_store_hash() {
    let w = Ws::new();
    let before = latest_hashes(&w);
    ok(w.root(), &["gc", "--parse-cache", "--all-cache", "--apply", "--by", "t"]);
    let b = ok(w.root(), &["build", "a", "--full"]);
    let h = before.iter().find(|x| x.0 == "a").unwrap().1.clone().unwrap();
    assert_eq!(b["data"]["store_hash"].as_str().unwrap(), h, "determinism survives the cache loss");
}

#[test]
fn a_held_store_is_refused_not_waited_on() {
    let w = Ws::new();
    let holder = w.conn();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let t = std::time::Instant::now();
    err(w.root(), &["gc", "--parse-cache", "--apply", "--by", "t"], "store_busy");
    assert!(t.elapsed().as_secs() < 20, "it waited");
    holder.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn a_build_after_a_version_bump_leaves_one_version_for_the_files_it_parsed() {
    use sha2::Digest;
    let w = Ws::new();
    let sha = |p: &std::path::Path| hex::encode(sha2::Sha256::digest(std::fs::read(p).unwrap()));
    let a_py = w.clone_dir("a").join("src/net/a.py");
    let b_py = w.clone_dir("a").join("src/net/b.py");
    std::fs::write(&b_py, "def helper():
    return 7
").unwrap();
    commit_all(&w.clone_dir("a"));
    ok(w.root(), &["build", "a"]);
    add_old_version(&w, "0.0.1+old");
    let old = |h: &str| w.count(&format!("SELECT COUNT(*) FROM parse_cache WHERE provider_version = '0.0.1+old' AND file_hash = '{h}'"));
    let (ha, hb_root) = (sha(&a_py), sha(&w.clone_dir("b").join("src/net/a.py")));
    assert_eq!((old(&ha), old(&hb_root)), (1, 1));
    // change b.py only: the build still asks the cache for a.py (a hit) and parses b.py
    std::fs::write(&b_py, "def helper():
    return 8
").unwrap();
    commit_all(&w.clone_dir("a"));
    let b = ok(w.root(), &["build", "a"]);
    assert_eq!(b["data"]["reused"], false);
    assert_eq!(old(&ha), 0, "a file this build read kept its old-version row");
    assert_eq!(old(&hb_root), 1, "another clone's row was pruned by this build");
    assert!(w.count("SELECT COUNT(*) FROM parse_cache WHERE provider_version != '0.0.1+old'") > 0);
}

#[test]
fn stale_runs_marks_only_old_running_rows() {
    let w = Ws::new();
    let c = w.conn();
    c.execute_batch(
        "INSERT INTO run(repo_id, provider, provider_version, started_at, status) SELECT id, 'reference', 'v', '2026-01-01T00:00:00Z', 'running' FROM repo WHERE name = 'a';
         INSERT INTO run(repo_id, provider, provider_version, started_at, status) SELECT id, 'reference', 'v', strftime('%Y-%m-%dT%H:%M:%SZ','now'), 'running' FROM repo WHERE name = 'b';",
    )
    .unwrap();
    drop(c);
    let d = ok(w.root(), &["gc", "--stale-runs"]);
    assert_eq!(d["data"]["stale"].as_array().unwrap().len(), 1, "{d}");
    assert_eq!(d["data"]["running_but_young"].as_array().unwrap().len(), 1);
    assert_eq!(w.count("SELECT COUNT(*) FROM run WHERE status = 'running'"), 2, "dry run wrote");
    let a = ok(w.root(), &["gc", "--stale-runs", "--apply", "--by", "t"]);
    assert_eq!(a["data"]["marked_abandoned"].as_array().unwrap().len(), 1);
    assert_eq!(w.count("SELECT COUNT(*) FROM run WHERE status = 'abandoned'"), 1);
    assert_eq!(w.count("SELECT COUNT(*) FROM run WHERE status = 'running'"), 1, "the young run was touched");
    ok(w.root(), &["governance", "verify"]);
}

#[test]
fn connection_settings_keep_store_hashes() {
    // synchronous NORMAL / a 64 MiB page cache / PRAGMA optimize on close must not move a single store_hash
    let w = Ws::new();
    let h1 = latest_hashes(&w);
    ok(w.root(), &["build", "a", "--full"]);
    ok(w.root(), &["check", "--deep"]);
    assert_eq!(latest_hashes(&w), h1);
}
