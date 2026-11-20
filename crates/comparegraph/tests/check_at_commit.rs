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
    assert_eq!(v["data"]["cited_commit_differs_from_head"], 2, "two locations of `a` are read at a commit behind HEAD: {v}");
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
fn a_clone_with_no_recorded_head_at_all_falls_back_to_the_working_tree() {
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

#[test]
fn a_clone_that_is_a_subfolder_of_a_git_repo_is_read_at_its_commit() {
    // the clone is mono/sub: the cited path is relative to the clone, the git root is one folder up
    let w = Ws::new();
    let mono = w.base.path().join("mono");
    let sub = mono.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("m.py"), "def retry_with_backoff():\n    return 1\n\ndef other():\n    return 2\n").unwrap();
    std::fs::write(sub.join("LICENSE"), "MIT License\n\nCopyright (c) 2026 S\n").unwrap();
    git(&mono, &["init", "-q"]);
    commit_all(&mono);
    ok(w.root(), &["add", sub.to_str().unwrap(), "--name", "mono"]);
    ok(w.root(), &["build", "mono"]);
    ok(w.root(), &["claim", "mono", "routing.retry", "has", "--author", "t", "--at", "m.py:1-2", "--no-reach"]);
    let v = ok(w.root(), &["check"]);
    assert_eq!(v["data"]["stale_locations"], 0, "{v}");
    std::fs::write(mono.join("elsewhere.txt"), "x\n").unwrap();
    commit_all(&mono);
    let v = ok(w.root(), &["check"]);
    assert_eq!(v["data"]["stale_locations"], 0, "{v}");
    assert!(v["data"]["clone_unavailable"].as_array().unwrap().is_empty(), "{v}");
}

#[test]
fn a_claim_citing_an_uncommitted_change_is_refused_and_never_goes_stale() {
    let w = Ws::new();
    let f = w.clone_dir("a").join("src/net/a.py");
    std::fs::write(&f, "def retry_with_backoff():\n    return 7\n\ndef recall_turns():\n    return 2\n").unwrap();
    let v = run(w.root(), &["claim", "a", "routing.retry", "partial", "--author", "t", "--at", "src/net/a.py:1-2", "--no-reach"]);
    assert_eq!(v["ok"], false, "a claim over a dirty file must be refused: {v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("uncommitted"), "{v}");
    // the refusal wrote nothing, so the next check has no new stale
    let c = ok(w.root(), &["check"]);
    assert_eq!(c["data"]["stale_locations"], 0, "{c}");
}

#[test]
fn a_run_with_no_recorded_head_is_named_unpinned_and_read_from_the_working_tree() {
    // the real live shape: run.head NULL, repo.head set (the run predates the column)
    let w = Ws::new();
    let c = w.conn();
    c.execute("UPDATE run SET head = NULL WHERE repo_id = (SELECT id FROM repo WHERE name = 'a')", []).unwrap();
    assert_eq!(w.count("SELECT COUNT(*) FROM repo WHERE name = 'a' AND head IS NOT NULL"), 1);
    drop(c);
    let v = ok(w.root(), &["check"]);
    assert_eq!(v["data"]["unpinned_runs"], 1, "{v}");
    assert_eq!(v["data"]["stale_locations"], 0, "{v}");
    assert!(v["data"]["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("no recorded commit")), "{v}");
}

#[test]
fn a_claim_on_a_run_whose_commit_is_not_in_the_clone_is_refused_not_read_from_the_working_tree() {
    let w = Ws::new();
    w.conn().execute("UPDATE run SET head = '1234567' WHERE repo_id = (SELECT id FROM repo WHERE name = 'a')", []).unwrap();
    let v = run(w.root(), &["claim", "a", "routing.retry", "has", "--author", "t", "--at", "src/net/a.py:1-2", "--no-reach"]);
    assert_eq!(v["ok"], false, "{v}");
    assert_eq!(v["error"]["code"], "clone_unavailable", "{v}");
    assert_eq!(v["error"]["data"]["reason"], "commit_unavailable", "{v}");
}
