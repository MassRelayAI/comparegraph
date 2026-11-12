//! The action log and `governance verify` (lane CG-STORE B2): tamper with it and see it fail where it was touched.
mod common;
use common::*;

fn log(w: &Ws) -> std::path::PathBuf {
    w.root().join(".comparegraph/actions.jsonl")
}
/// Five entries from three write verbs.
fn with_entries(w: &Ws) {
    let out = tempfile::tempdir().unwrap();
    let o = out.path().to_str().unwrap().to_string();
    ok(w.root(), &["backup", "--claims-only", "--out", &o, "--by", "t"]);
    ok(w.root(), &["gc", "--parse-cache", "--apply", "--by", "t"]);
    ok(w.root(), &["gc", "--stale-runs", "--apply", "--by", "t"]);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    ok(w.root(), &["backup", "--claims-only", "--out", &o, "--by", "t"]);
    ok(w.root(), &["archive", "b", "--why", "agpl reference", "--by", "t", "--apply"]);
}

#[test]
fn an_empty_log_on_a_store_with_no_flags_verifies_and_says_so() {
    let w = Ws::new();
    let v = ok(w.root(), &["governance", "verify"]);
    assert_eq!(v["data"]["actions"], 0);
    assert_eq!(v["data"]["chain_ok"], true);
    assert!(v["data"]["file"].as_str().unwrap().ends_with("actions.jsonl"), "a check that prints nothing has not run: {v}");
}

#[test]
fn every_write_verb_appends_one_chained_entry() {
    let w = Ws::new();
    with_entries(&w);
    let v = ok(w.root(), &["governance", "verify"]);
    assert_eq!(v["data"]["actions"], 5, "{v}");
    assert_eq!(v["data"]["archived"], 1);
    let verbs: Vec<String> = std::fs::read_to_string(log(&w)).unwrap().lines().map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["verb"].as_str().unwrap().to_string()).collect();
    assert_eq!(verbs, ["backup", "gc-parse-cache", "gc-stale-runs", "backup", "archive"]);
    ok(w.root(), &["doctor"]);
}

#[test]
fn one_flipped_byte_in_line_three_fails_at_line_three() {
    let w = Ws::new();
    with_entries(&w);
    let t = std::fs::read_to_string(log(&w)).unwrap();
    let mut lines: Vec<String> = t.lines().map(String::from).collect();
    lines[2] = lines[2].replacen("\"by\":\"t\"", "\"by\":\"u\"", 1);
    std::fs::write(log(&w), lines.join("\n") + "\n").unwrap();
    let v = err(w.root(), &["governance", "verify"], "check_failed");
    assert_eq!(v["data"]["broken_at"]["line"], 3, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("line 3"));
    // check counts the break as a problem; a write verb refuses to append to a broken chain
    let c = err(w.root(), &["check"], "check_failed");
    assert!(c["data"]["problems"].as_array().unwrap().iter().any(|p| p.as_str().unwrap().starts_with("action log")));
    err(w.root(), &["gc", "--stale-runs", "--apply", "--by", "t"], "chain_broken");
}

#[test]
fn a_deleted_last_line_fails_on_the_head_kept_in_the_store() {
    let w = Ws::new();
    with_entries(&w);
    let t = std::fs::read_to_string(log(&w)).unwrap();
    let lines: Vec<&str> = t.lines().collect();
    std::fs::write(log(&w), lines[..lines.len() - 1].join("\n") + "\n").unwrap();
    let v = err(w.root(), &["governance", "verify"], "check_failed");
    let p = v["data"]["problems"].to_string();
    assert!(p.contains("head mismatch"), "{v}");
}

#[test]
fn an_archive_flag_with_no_log_entry_fails() {
    let w = Ws::new();
    ok(w.root(), &["governance", "verify"]);
    w.conn().execute("INSERT INTO meta(key, value) VALUES ('archived:a', 'deadbeef')", []).unwrap();
    let v = err(w.root(), &["governance", "verify"], "check_failed");
    assert!(v["data"]["problems"].to_string().contains("no matching archive entry"), "{v}");
}

#[test]
fn a_tampered_manifest_fails_verify() {
    let w = Ws::new();
    ok(w.root(), &["archive", "b", "--why", "x", "--by", "t", "--apply"]);
    let m = w.root().join(".comparegraph/archive/b.json");
    let s = std::fs::read_to_string(&m).unwrap();
    std::fs::write(&m, s.replace("\"why\": \"x\"", "\"why\": \"y\"")).unwrap();
    let v = err(w.root(), &["governance", "verify"], "check_failed");
    assert!(v["data"]["problems"].to_string().contains("no longer hashes"), "{v}");
}
