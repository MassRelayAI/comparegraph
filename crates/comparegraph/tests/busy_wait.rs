//! `--wait <secs>` / `COMPAREGRAPH_WAIT`: a writer blocked by another writer waits for the store lock, and when the wait
//! runs out the error says so (how long, and who has the store open when that is known) instead of `database is locked`.
use assert_cmd::Command;
use std::path::Path;
use std::time::{Duration, Instant};

fn cc(ws: &Path) -> Command {
    let mut c = Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(ws).env_remove("COMPAREGRAPH_WAIT").env_remove("COMPAREGRAPH_VERB");
    c
}
/// A workspace with one registered clone, and a second clone ready to `add`.
fn workspace() -> (tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let base = tempfile::tempdir().unwrap();
    for n in ["a", "b"] {
        std::fs::create_dir_all(base.path().join(n).join("src")).unwrap();
        std::fs::write(base.path().join(n).join("src/x.ts"), "export const x = 1;\n").unwrap();
    }
    cc(ws.path()).arg("add").arg(base.path().join("a")).args(["--name", "a"]).assert().success();
    (ws, base)
}
/// Another connection takes the write lock, as a concurrent `claim` or `build` does.
fn hold_write_lock(ws: &Path) -> rusqlite::Connection {
    let c = rusqlite::Connection::open(ws.join(".comparegraph/comparegraph.db")).unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    c
}
fn add_b(ws: &Path, base: &Path) -> Command {
    let mut c = cc(ws);
    c.arg("add").arg(base.join("b")).args(["--name", "b"]);
    c
}

#[test]
fn a_blocked_writer_waits_then_succeeds_within_wait() {
    let (ws, base) = workspace();
    let holder = hold_write_lock(ws.path());
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        holder.execute_batch("COMMIT").unwrap();
    });
    let start = Instant::now();
    let out = add_b(ws.path(), base.path()).args(["--wait", "10"]).output().unwrap();
    let took = start.elapsed();
    t.join().unwrap();
    assert!(out.status.success(), "add should wait out the lock: {}", String::from_utf8_lossy(&out.stdout));
    assert!(took >= Duration::from_millis(1200), "add returned in {took:?}, before the lock was released");
}

#[test]
fn the_default_wait_outlasts_the_old_five_seconds() {
    let (ws, base) = workspace();
    let holder = hold_write_lock(ws.path());
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(6500));
        holder.execute_batch("COMMIT").unwrap();
    });
    let out = add_b(ws.path(), base.path()).output().unwrap();
    t.join().unwrap();
    assert!(out.status.success(), "default wait gave up before 6.5 s: {}", String::from_utf8_lossy(&out.stdout));
}

#[test]
fn a_writer_that_outwaits_wait_fails_saying_it_timed_out() {
    let (ws, base) = workspace();
    let _holder = hold_write_lock(ws.path());
    let start = Instant::now();
    let out = add_b(ws.path(), base.path()).args(["--wait", "1"]).output().unwrap();
    let took = start.elapsed();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "add must fail while the lock is held: {s}");
    assert!(took < Duration::from_secs(4), "--wait 1 took {took:?}");
    assert!(s.contains("timed out after 1 s waiting for the store lock"), "no timeout message: {s}");
    assert!(!s.contains("database is locked"), "raw sqlite message leaked: {s}");
    // the holder here is a bare sqlite connection, not a comparegraph process: the message must not invent one
    assert!(s.contains("holder unknown"), "the unknown holder must be said honestly: {s}");
}

#[test]
fn comparegraph_wait_env_sets_the_wait() {
    let (ws, base) = workspace();
    let _holder = hold_write_lock(ws.path());
    let start = Instant::now();
    let out = add_b(ws.path(), base.path()).env("COMPAREGRAPH_WAIT", "1").output().unwrap();
    let took = start.elapsed();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "{s}");
    assert!(took < Duration::from_secs(4), "COMPAREGRAPH_WAIT=1 took {took:?}");
    assert!(s.contains("timed out after 1 s"), "{s}");
}

#[test]
fn the_timeout_names_the_comparegraph_verb_and_pid_that_has_the_store_open() {
    let (ws, base) = workspace();
    // this test process plays a running `claim`: it opens the store through the library and holds the write lock
    std::env::set_var("COMPAREGRAPH_VERB", "claim");
    let store = comparegraph::store::Store::open(ws.path()).unwrap();
    std::env::remove_var("COMPAREGRAPH_VERB");
    store.conn().execute_batch("BEGIN IMMEDIATE").unwrap();
    let out = add_b(ws.path(), base.path()).args(["--wait", "1"]).output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "{s}");
    assert!(s.contains(&format!("claim (pid {})", std::process::id())), "holder not named: {s}");
    store.conn().execute_batch("ROLLBACK").unwrap();
}
