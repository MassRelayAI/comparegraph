//! `check` is read-only by default; a missing clone or a moved head is `clone_unavailable` and writes nothing;
//! `--mark` writes flags, `--mark --recompute` clears sticky ones (lane CG-STORE A3).
mod common;
use common::*;
use std::io::Write;

#[test]
fn check_on_a_sound_store_is_green_and_says_so() {
    let w = Ws::new();
    let v = ok(w.root(), &["check"]);
    assert_eq!(v["data"]["stale_locations"], 0);
    assert_eq!(v["data"]["repos"], 3, "a check that prints nothing has not run: {v}");
    assert_eq!(v["data"]["chain_ok"], true);
    assert_eq!(v["data"]["deep"], false);
    let d = ok(w.root(), &["check", "--deep"]);
    assert_eq!(d["data"]["deep"], true);
    assert_eq!(d["data"]["drifted_runs"], 0);
}

#[test]
fn a_missing_clone_is_clone_unavailable_and_writes_nothing() {
    let w = Ws::new();
    // settle the WAL so the byte comparison sees only what check itself would write
    w.conn().query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())).unwrap();
    let before = w.store_bytes();
    std::fs::rename(w.clone_dir("a"), w.base.path().join("a-moved")).unwrap();
    let v = err(w.root(), &["check"], "clone_unavailable");
    let cu = v["data"]["clone_unavailable"].as_array().unwrap();
    assert_eq!(cu.len(), 1, "{v}");
    assert_eq!(cu[0]["repo"], "a");
    assert_eq!(cu[0]["reason"], "missing");
    assert_eq!(cu[0]["locations_not_checked"], 2);
    assert_eq!(v["data"]["stale_locations"], 0, "an absent clone is not drift");
    assert_eq!(w.store_bytes(), before, "check wrote to the store");
    assert_eq!(w.count("SELECT COUNT(*) FROM location WHERE stale = 1"), 0);
    // --mark writes nothing for an unavailable clone either
    let m = run(w.root(), &["check", "--mark", "--by", "t"]);
    assert_eq!(m["error"]["code"], "clone_unavailable");
    assert_eq!(w.count("SELECT COUNT(*) FROM location WHERE stale = 1"), 0);
    // back in place: green again
    std::fs::rename(w.base.path().join("a-moved"), w.clone_dir("a")).unwrap();
    ok(w.root(), &["check"]);
}

#[test]
fn a_clone_at_another_head_is_clone_unavailable() {
    let w = Ws::new();
    std::fs::write(w.clone_dir("a").join("src/net/b.py"), "def x():\n    return 9\n").unwrap();
    commit_all(&w.clone_dir("a"));
    let v = err(w.root(), &["check"], "clone_unavailable");
    assert_eq!(v["data"]["clone_unavailable"][0]["reason"], "head_moved");
    assert_eq!(w.count("SELECT COUNT(*) FROM location WHERE stale = 1"), 0);
}

#[test]
fn real_drift_is_reported_marked_with_mark_and_cleared_by_recompute() {
    let w = Ws::new();
    let f = w.clone_dir("a").join("src/net/a.py");
    let orig = std::fs::read_to_string(&f).unwrap();
    // drift without a new commit: the head matches, the lines do not
    std::fs::write(&f, orig.replace("return 1", "return 111")).unwrap();
    let v = err(w.root(), &["check"], "check_failed");
    assert_eq!(v["data"]["stale_locations"], 1);
    assert_eq!(w.count("SELECT COUNT(*) FROM location WHERE stale = 1"), 0, "default check is read-only");
    err(w.root(), &["check", "--mark"], "invalid_arguments");
    let m = run(w.root(), &["check", "--mark", "--by", "t"]);
    assert_eq!(m["data"]["marked"]["set_stale"], 1, "{m}");
    assert_eq!(w.count("SELECT COUNT(*) FROM location WHERE stale = 1"), 1);
    // the lines come back: the sticky flag is reported, then cleared by --recompute
    std::fs::write(&f, &orig).unwrap();
    let n = ok(w.root(), &["check"]);
    assert!(n["data"]["notes"].as_array().unwrap().iter().any(|x| x.as_str().unwrap().contains("--recompute")), "{n}");
    assert_eq!(w.count("SELECT COUNT(*) FROM location WHERE stale = 1"), 1, "plain check never clears");
    let r = ok(w.root(), &["check", "--mark", "--recompute", "--by", "t"]);
    assert_eq!(r["data"]["marked"]["cleared"], 1, "{r}");
    assert_eq!(w.count("SELECT COUNT(*) FROM location WHERE stale = 1"), 0);
    let g = ok(w.root(), &["governance", "verify"]);
    assert_eq!(g["data"]["actions"], 2, "both marks are in the action log: {g}");
}

#[test]
fn a_poisoned_store_returns_to_stale_zero_with_recompute() {
    // the state of the live store: flags set by a check that ran while the root was missing, lines unchanged
    let w = Ws::new();
    w.conn().execute("UPDATE location SET stale = 1", []).unwrap();
    let before = w.count("SELECT COUNT(*) FROM location WHERE stale = 1");
    assert!(before >= 3);
    let v = ok(w.root(), &["claims", "--repo", "a"]);
    assert!(v["data"]["items"].as_array().unwrap().iter().all(|c| c["confidence"] == "unknown"), "poisoned claims read unknown");
    let r = ok(w.root(), &["check", "--mark", "--recompute", "--by", "t"]);
    assert_eq!(r["data"]["marked"]["cleared"], before);
    assert_eq!(w.count("SELECT COUNT(*) FROM location WHERE stale = 1"), 0);
    assert_eq!(r["data"]["downgraded_claims"], 0);
}

#[test]
fn the_mcp_check_leaves_the_store_bytes_equal_even_with_a_missing_clone() {
    let w = Ws::new();
    w.conn().query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())).unwrap();
    std::fs::remove_dir_all(w.clone_dir("b")).unwrap();
    let before = w.store_bytes();
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("comparegraph"))
        .arg("--root")
        .arg(w.root())
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        let i = child.stdin.as_mut().unwrap();
        writeln!(i, r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"comparegraph_check","arguments":{{}}}}}}"#).unwrap();
    }
    drop(child.stdin.take());
    let o = child.wait_with_output().unwrap();
    let reply: serde_json::Value = serde_json::from_slice(o.stdout.split(|b| *b == b'\n').next().unwrap()).unwrap();
    let env: serde_json::Value = serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(env["error"]["code"], "clone_unavailable", "{env}");
    assert_eq!(w.store_bytes(), before, "the MCP check wrote to the store");
}
