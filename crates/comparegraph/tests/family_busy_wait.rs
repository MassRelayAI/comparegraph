//! `family add` takes the write lock up front (BEGIN IMMEDIATE), so a held lock is waited out under `--wait` instead of
//! failing at once with a misleading `timed out after N s`.
use assert_cmd::Command;
use std::path::Path;
use std::time::{Duration, Instant};

fn cc(ws: &Path) -> Command {
    let mut c = Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root")
        .arg(ws)
        .env_remove("COMPAREGRAPH_WAIT")
        .env_remove("COMPAREGRAPH_VERB");
    c
}
const PACK: &str = "---\nfamily: fam\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n| g.a | g | A | def a | has a | basics | 1 |\n";

/// A workspace whose store holds family `fam` already, and a pack dir that changes it.
fn workspace() -> (tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), PACK).unwrap();
    cc(ws.path())
        .args(["family", "add"])
        .arg(pack.path())
        .assert()
        .success();
    let changed = PACK.to_string() + "| g.b | g | B | def b | has b | common | 2 |\n";
    std::fs::write(pack.path().join("rows.md"), changed).unwrap();
    (ws, pack)
}

#[test]
fn family_add_waits_out_a_held_write_lock_then_succeeds() {
    let (ws, pack) = workspace();
    // another writer, as a concurrent `claim` or `build`: takes the lock and changes the store before it commits
    let holder =
        rusqlite::Connection::open(ws.path().join(".comparegraph/comparegraph.db")).unwrap();
    holder
        .execute_batch(
            "BEGIN IMMEDIATE; UPDATE family SET description = 'held' WHERE name = 'fam';",
        )
        .unwrap();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        holder.execute_batch("COMMIT").unwrap();
    });
    let start = Instant::now();
    let out = cc(ws.path())
        .args(["family", "add"])
        .arg(pack.path())
        .args(["--wait", "10"])
        .output()
        .unwrap();
    let took = start.elapsed();
    t.join().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "family add should wait out the lock: {s}"
    );
    assert!(
        took >= Duration::from_millis(1200),
        "family add returned in {took:?}, before the lock was released"
    );
    assert!(s.contains("\"g.b\""), "the new feature was not added: {s}");
}
