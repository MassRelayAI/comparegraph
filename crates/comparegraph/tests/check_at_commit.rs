//! `check` verifies a location against the git objects of the commit its run read (D451), never the clone's working HEAD:
//! a newer upstream never reddens it, a missing commit is `commit_unavailable`, and a hash that differs at the cited commit is stale.
mod common;
use common::*;

fn head_of(w: &Ws, n: &str) -> String {
    let o = std::process::Command::new("git").arg("-C").arg(w.clone_dir(n)).args(["rev-parse", "HEAD"]).output().unwrap();
    String::from_utf8(o.stdout).unwrap().trim().to_string()
}

#[test]
fn a_moved_head_is_checked_at_the_cited_commit_and_stays_green() {
    let w = Ws::new();
    let d = w.clone_dir("a");
    // upstream moves: an unrelated file, then the cited lines themselves change
    std::fs::write(d.join("other.py"), "x = 1\n").unwrap();
    commit_all(&d);
    std::fs::write(d.join("src/net/a.py"), "def retry_with_backoff():\n    return 999\n\ndef recall_turns():\n    return 2\n").unwrap();
    commit_all(&d);
    let v = ok(w.root(), &["check"]);
    assert_eq!(v["data"]["stale_locations"], 0, "{v}");
    assert_eq!(v["data"]["repos"], 3, "a check that prints nothing has not run: {v}");
    assert_eq!(v["data"]["behind_head"], 2, "two locations of `a` are read at a commit behind HEAD: {v}");
    assert!(v["data"]["clone_unavailable"].as_array().unwrap().is_empty(), "{v}");
}

#[test]
fn a_hash_that_differs_at_the_cited_commit_is_stale() {
    let w = Ws::new();
    let c = w.conn();
    c.execute("DROP TRIGGER location_no_update", []).unwrap();
    c.execute("UPDATE location SET content_hash = 'deadbeef' WHERE id = (SELECT MIN(id) FROM location)", []).unwrap();
    drop(c);
    let v = err(w.root(), &["check"], "check_failed");
    assert_eq!(v["data"]["stale_locations"], 1, "{v}");
}

#[test]
fn an_unreachable_commit_is_commit_unavailable_never_a_pass() {
    let w = Ws::new();
    let c = w.conn();
    c.execute("UPDATE run SET head = '1234567' WHERE repo_id = (SELECT id FROM repo WHERE name = 'a')", []).unwrap();
    drop(c);
    let v = err(w.root(), &["check"], "clone_unavailable");
    let cu = v["data"]["clone_unavailable"].as_array().unwrap();
    assert_eq!(cu.len(), 1, "{v}");
    assert_eq!(cu[0]["reason"], "commit_unavailable");
    assert_eq!(cu[0]["locations_not_checked"], 2);
}

#[test]
fn a_run_with_no_head_falls_back_to_the_working_tree() {
    let w = Ws::new();
    let c = w.conn();
    c.execute("UPDATE run SET head = NULL WHERE repo_id = (SELECT id FROM repo WHERE name = 'a')", []).unwrap();
    c.execute("UPDATE repo SET head = NULL WHERE name = 'a'", []).unwrap();
    drop(c);
    let _ = head_of(&w, "a");
    let v = ok(w.root(), &["check"]);
    assert_eq!(v["data"]["stale_locations"], 0, "{v}");
    std::fs::write(w.clone_dir("a").join("src/net/a.py"), "def retry_with_backoff():\n    return 5\n\ndef recall_turns():\n    return 2\n").unwrap();
    let v = err(w.root(), &["check"], "check_failed");
    assert_eq!(v["data"]["stale_locations"], 1, "{v}");
}
