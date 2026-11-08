//! The additive column migration in `Store::open`: concurrent opens of a pre-migration store, a read-only store,
//! a store with a newer schema_version, and the `--wait` / `COMPAREGRAPH_WAIT` bounds.
use assert_cmd::Command;
use std::path::Path;

fn cc(ws: &Path) -> Command {
    let mut c = Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(ws).env_remove("COMPAREGRAPH_WAIT").env_remove("COMPAREGRAPH_VERB");
    c
}
fn db(ws: &Path) -> std::path::PathBuf {
    ws.join(".comparegraph/comparegraph.db")
}
/// The columns added after the first release, dropped again: the shape of a store written by an older binary.
const LATER_COLUMNS: &[(&str, &str)] = &[
    ("repo", "source_commit"),
    ("repo", "moved_to"),
    ("repo", "include_json"),
    ("repo", "exclude_json"),
    ("run", "include_json"),
    ("run", "exclude_json"),
    ("claim", "at_commit"),
    ("claim", "out_of_subset"),
    ("claim", "reach"),
    ("claim", "reach_path"),
    ("claim", "reach_note"),
];
fn columns(ws: &Path, table: &str) -> Vec<String> {
    let c = rusqlite::Connection::open(db(ws)).unwrap();
    let mut st = c.prepare(&format!("PRAGMA table_info({table})")).unwrap();
    let v = st.query_map([], |r| r.get::<_, String>(1)).unwrap().map(|r| r.unwrap()).collect();
    v
}
/// A workspace with one registered clone whose store lacks every later column.
fn old_store() -> (tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let base = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(base.path().join("a/src")).unwrap();
    std::fs::write(base.path().join("a/src/x.ts"), "export const x = 1;\n").unwrap();
    cc(ws.path()).arg("add").arg(base.path().join("a")).args(["--name", "a"]).assert().success();
    let c = rusqlite::Connection::open(db(ws.path())).unwrap();
    for (t, col) in LATER_COLUMNS {
        c.execute_batch(&format!("ALTER TABLE {t} DROP COLUMN {col}")).unwrap();
    }
    drop(c);
    assert!(!columns(ws.path(), "repo").iter().any(|n| n == "source_commit"));
    (ws, base)
}

#[test]
fn concurrent_opens_of_a_pre_migration_store_all_succeed() {
    for trial in 0..5 {
        let (ws, _base) = old_store();
        let kids: Vec<_> = (0..6)
            .map(|_| {
                std::process::Command::new(assert_cmd::cargo::cargo_bin("comparegraph"))
                    .arg("--root")
                    .arg(ws.path())
                    .arg("repos")
                    .env_remove("COMPAREGRAPH_WAIT")
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for k in kids {
            let out = k.wait_with_output().unwrap();
            let s = String::from_utf8_lossy(&out.stdout);
            assert!(out.status.success(), "trial {trial}: {s}");
        }
        for (t, col) in LATER_COLUMNS {
            assert!(columns(ws.path(), t).iter().any(|n| n == col), "trial {trial}: {t}.{col} missing");
        }
    }
}

#[test]
fn a_read_only_pre_migration_store_still_answers_read_verbs() {
    use std::os::unix::fs::PermissionsExt;
    let (ws, _base) = old_store();
    let f = db(ws.path());
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o444)).unwrap();
    // permission bits do not bind root: skip when a write still succeeds
    if std::fs::OpenOptions::new().write(true).open(&f).is_ok() {
        return;
    }
    let out = cc(ws.path()).arg("repos").output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "a read verb on a read-only old store: {s}");
    assert!(s.contains("\"a\""), "{s}");
    // a writer is refused with a message that says why
    let out = cc(ws.path()).args(["decide", "a", "adopt", "--why", "w", "--by", "me", "--feature", "x", "--family", "f"])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(!out.status.success(), "{s}");
    assert!(s.contains("read-only") || s.contains("readonly"), "{s}");
    // nothing was migrated
    assert!(!columns(ws.path(), "repo").iter().any(|n| n == "source_commit"));
}

#[test]
fn a_newer_schema_version_is_refused_before_anything_is_changed() {
    let (ws, _base) = old_store();
    let c = rusqlite::Connection::open(db(ws.path())).unwrap();
    c.execute("UPDATE meta SET value = '2' WHERE key = 'schema_version'", []).unwrap();
    drop(c);
    let out = cc(ws.path()).arg("repos").output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(!out.status.success() && s.contains("schema_version"), "{s}");
    assert!(!columns(ws.path(), "repo").iter().any(|n| n == "source_commit"), "the refused store was migrated");
}

#[test]
fn a_wait_beyond_the_maximum_is_invalid_arguments_not_a_panic() {
    let (ws, _base) = old_store();
    let out = cc(ws.path()).args(["--wait", "2147484", "repos"]).output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    let e = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!e.contains("panicked"), "{e}");
    assert!(!out.status.success() && s.contains("invalid_arguments") && s.contains("3600"), "{s}\n{e}");
}

#[test]
fn a_huge_env_wait_is_clamped_with_a_note() {
    let (ws, _base) = old_store();
    let out = cc(ws.path()).env("COMPAREGRAPH_WAIT", "99999999").arg("repos").output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    let e = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!e.contains("panicked"), "{e}");
    assert!(out.status.success(), "{s}\n{e}");
    assert!(e.contains("3600"), "the clamp must be said: {e}");
}

#[test]
fn a_malformed_env_wait_is_refused() {
    let (ws, _base) = old_store();
    for bad in ["abc", "-1"] {
        let out = cc(ws.path()).env("COMPAREGRAPH_WAIT", bad).arg("repos").output().unwrap();
        let s = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(!out.status.success() && s.contains("invalid_arguments") && s.contains("COMPAREGRAPH_WAIT"), "{bad}: {s}");
    }
}

#[test]
fn wait_zero_does_not_claim_it_waited() {
    let (ws, base) = old_store();
    std::fs::create_dir_all(base.path().join("b/src")).unwrap();
    std::fs::write(base.path().join("b/src/x.ts"), "export const y = 1;\n").unwrap();
    cc(ws.path()).arg("repos").assert().success(); // migrate first
    let holder = rusqlite::Connection::open(db(ws.path())).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let out = cc(ws.path()).args(["--wait", "0", "add"]).arg(base.path().join("b")).args(["--name", "b"]).output().unwrap();
    holder.execute_batch("ROLLBACK").unwrap();
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(!out.status.success(), "{s}");
    assert!(!s.contains("after 0 s"), "{s}");
    assert!(s.contains("did not wait"), "{s}");
}
