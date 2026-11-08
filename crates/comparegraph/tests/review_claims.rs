//! Review findings on `claim`, `claim --batch` and `matrix --reach` (main at d8c40af): the head `--at-commit` is checked
//! against, concurrent single claims, the batch resume skip, batch limitations, the matcher's stack, prose files,
//! `--out-of-subset` paths and a stale cell's reach.
use std::path::Path;
use std::process::Command;

const AGENT: &str = "# agent loop\n\
import os\n\
\n\
def run_turn(msg):\n\
    return compact(msg)\n\
\n\
def compact(msg):\n\
    return msg[:10]\n";

const PACK: &str = "---\nfamily: fam\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n\
| routing.retry | routing | Retry with backoff | Failed calls are retried | a retry loop | basics | 2 |\n\
| memory.recall | memory | Recall | Past turns | a store read | extra | 1 |\n";

fn cc(root: &Path) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(root).env_remove("COMPAREGRAPH_WAIT");
    c
}
fn json(a: assert_cmd::assert::Assert) -> serde_json::Value {
    serde_json::from_slice(&a.get_output().stdout).unwrap()
}
fn lines(out: &[u8]) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(out)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}
fn git(d: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(d)
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{args:?}");
    String::from_utf8(o.stdout).unwrap().trim().to_string()
}
fn commit(d: &Path, msg: &str) -> String {
    git(d, &["add", "."]);
    git(d, &["commit", "-q", "--allow-empty", "-m", msg]);
    git(d, &["rev-parse", "HEAD"])
}

/// (workspace, clone, pack): a git clone at one commit, added and built.
fn setup() -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), PACK).unwrap();
    cc(ws.path()).args(["family", "add"]).arg(pack.path()).assert().success();
    let a = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(a.path().join("src")).unwrap();
    std::fs::write(a.path().join("src/agent.py"), AGENT).unwrap();
    std::fs::write(a.path().join("LICENSE"), "MIT License\n\nCopyright (c) 2026 S\n").unwrap();
    git(a.path(), &["init", "-q"]);
    commit(a.path(), "c1");
    cc(ws.path()).arg("add").arg(a.path()).args(["--name", "a"]).assert().success();
    cc(ws.path()).args(["build", "a"]).assert().success();
    (ws, a, pack)
}
fn claim(ws: &Path, extra: &[&str]) -> assert_cmd::assert::Assert {
    let mut c = cc(ws);
    c.args(["claim", "a", "routing.retry"]).args(extra);
    c.assert()
}
fn refused(ws: &Path, extra: &[&str]) -> String {
    let v = json(claim(ws, extra).failure());
    assert_eq!(v["error"]["code"], "invalid_arguments", "{extra:?}: {v}");
    v["error"]["message"].as_str().unwrap().to_string()
}
fn limitation_codes(v: &serde_json::Value) -> Vec<String> {
    let mut c: Vec<String> = v["evidence"]["limitations"]
        .as_array()
        .map(|a| a.iter().map(|l| l["code"].as_str().unwrap_or_default().to_string()).collect())
        .unwrap_or_default();
    c.sort();
    c
}
fn batch(ws: &Path, rows: &[String]) -> Vec<serde_json::Value> {
    let path = ws.join("rows.jsonl");
    std::fs::write(&path, rows.join("\n") + "\n").unwrap();
    let o = cc(ws).args(["claim", "--batch"]).arg(&path).output().unwrap();
    lines(&o.stdout)
}

// ---- 1: --at-commit is checked against the head the latest build read, not the head `add` saw

#[test]
fn at_commit_follows_the_head_of_the_latest_build() {
    let (ws, a, _p) = setup();
    let c1 = git(a.path(), &["rev-parse", "HEAD"]);
    std::fs::write(a.path().join("src/agent.py"), format!("{AGENT}\ndef extra():\n    return 1\n")).unwrap();
    let c2 = commit(a.path(), "c2");
    cc(ws.path()).args(["build", "a"]).assert().success();
    let at = ["has", "--author", "me", "--at", "src/agent.py:4-5", "--at-commit"];
    let v = json(claim(ws.path(), &[&at[..], &[c2.as_str()]].concat()).success());
    assert_eq!(v["data"]["at_commit"], c2.as_str(), "{v}");
    let m = refused(ws.path(), &[&at[..], &[c1.as_str()]].concat());
    assert!(m.contains(&c2[..7]), "{m}");
    // a commit that changes no parsed file reuses the run: the run's head moves with it
    let c3 = commit(a.path(), "c3");
    cc(ws.path()).args(["build", "a"]).assert().success();
    claim(ws.path(), &[&at[..], &[c3.as_str()]].concat()).success();
    refused(ws.path(), &[&at[..], &[c2.as_str()]].concat());
}

// ---- 2: concurrent single claims wait for the write lock instead of failing at once

#[test]
fn parallel_single_claims_all_succeed() {
    let (ws, _a, _p) = setup();
    let bin = assert_cmd::cargo::cargo_bin("comparegraph");
    let procs: Vec<_> = (0..4)
        .map(|k| {
            let bin = bin.clone();
            let ws = ws.path().to_path_buf();
            std::thread::spawn(move || {
                let mut failed = vec![];
                for n in 0..12 {
                    let o = Command::new(&bin)
                        .arg("--root")
                        .arg(&ws)
                        .env("COMPAREGRAPH_WAIT", "30")
                        .args(["claim", "a", if (k + n) % 2 == 0 { "routing.retry" } else { "memory.recall" }])
                        .args(["has", "--author", &format!("w{k}"), "--at", "src/agent.py:4-5", "--no-reach"])
                        .output()
                        .unwrap();
                    if !o.status.success() {
                        failed.push(String::from_utf8_lossy(&o.stdout).into_owned());
                    }
                }
                failed
            })
        })
        .collect();
    // a short holder of the write lock while the claims run
    let holder = rusqlite::Connection::open(ws.path().join(".comparegraph/comparegraph.db")).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    holder.execute_batch("COMMIT").unwrap();
    let failed: Vec<String> = procs.into_iter().flat_map(|p| p.join().unwrap()).collect();
    assert!(failed.is_empty(), "{} of 48 failed: {}", failed.len(), failed.join("\n"));
    let v = json(cc(ws.path()).args(["claims", "--history"]).assert().success());
    assert_eq!(v["data"]["items"].as_array().unwrap().len(), 48);
}

// ---- 3: the batch resume skip compares the cited content too

#[test]
fn a_batch_rerun_after_the_cited_lines_changed_writes_a_new_claim() {
    let (ws, a, _p) = setup();
    let row = vec!["{\"repo\":\"a\",\"feature\":\"routing.retry\",\"verdict\":\"has\",\"author\":\"me\",\"at\":[\"src/agent.py:4-5\"],\"no_reach\":true}".to_string()];
    let first = batch(ws.path(), &row);
    let h1 = first[0]["data"]["locations"][0]["content_hash"].clone();
    // unchanged: skipped
    let again = batch(ws.path(), &row);
    assert_eq!(again[0]["data"]["skipped"], true, "{}", again[0]);
    // the cited lines change (same span, same run): a new claim with the new hash
    std::fs::write(a.path().join("src/agent.py"), AGENT.replace("compact(msg)\n\ndef", "compact(msg[1:])\n\ndef")).unwrap();
    let edited = batch(ws.path(), &row);
    assert!(edited[0]["data"]["skipped"].is_null(), "{}", edited[0]);
    assert_ne!(edited[0]["data"]["locations"][0]["content_hash"], h1, "{}", edited[0]);
    assert_ne!(edited[0]["data"]["id"], first[0]["data"]["id"]);
    // and unchanged again: skipped
    let again = batch(ws.path(), &row);
    assert_eq!(again[0]["data"]["skipped"], true, "{}", again[0]);
}

#[test]
fn a_batch_rerun_over_a_stale_live_claim_writes_a_new_claim() {
    let (ws, a, _p) = setup();
    let row = vec!["{\"repo\":\"a\",\"feature\":\"routing.retry\",\"verdict\":\"has\",\"author\":\"me\",\"at\":[\"src/agent.py:4-5\"],\"no_reach\":true}".to_string()];
    batch(ws.path(), &row);
    // `check` flags the location stale after an edit; the edit is then undone, so the hash agrees again
    std::fs::write(a.path().join("src/agent.py"), AGENT.replace("compact(msg)\n\ndef", "compact(msg[1:])\n\ndef")).unwrap();
    let _ = cc(ws.path()).arg("check").output().unwrap();
    std::fs::write(a.path().join("src/agent.py"), AGENT).unwrap();
    let v = json(cc(ws.path()).args(["claims"]).assert().success());
    assert_eq!(v["data"]["items"][0]["locations"][0]["stale"], true, "{v}");
    let out = batch(ws.path(), &row);
    assert!(out[0]["data"]["skipped"].is_null(), "{}", out[0]);
    assert_eq!(out[0]["data"]["locations"][0]["stale"], false, "{}", out[0]);
}

// ---- 4: batch rows carry the same evidence.limitations as a single claim, a skipped row too

#[test]
fn batch_rows_lift_limitations_like_a_single_claim() {
    let (ws, _a, _p) = setup();
    let single = json(
        claim(ws.path(), &["partial", "--author", "me", "--out-of-subset", "tests/t.py:1-4", "--at", "src/agent.py:2"]).success(),
    );
    let want = limitation_codes(&single);
    assert!(want.contains(&"out_of_subset".to_string()), "{single}");
    assert!(want.contains(&"citation_imports_only".to_string()), "{single}");
    let row = vec!["{\"repo\":\"a\",\"feature\":\"memory.recall\",\"verdict\":\"partial\",\"author\":\"me\",\"out_of_subset\":\"tests/t.py:1-4\",\"at\":[\"src/agent.py:2\"]}".to_string()];
    let out = batch(ws.path(), &row);
    assert_eq!(limitation_codes(&out[0]), want, "{}", out[0]);
    assert!(out[0]["data"].get("limitation_items").is_none(), "{}", out[0]);
    let out = batch(ws.path(), &row);
    assert_eq!(out[0]["data"]["skipped"], true, "{}", out[0]);
    assert_eq!(limitation_codes(&out[0]), want, "a skipped row keeps its qualifiers: {}", out[0]);
}

// ---- 5: a long line never overflows the matcher's stack

#[test]
fn a_long_line_is_matched_or_refused_with_an_envelope() {
    let (ws, a, _p) = setup();
    let long = "a".repeat(200_000);
    std::fs::write(a.path().join("src/big.py"), format!("x = '{long}'\ndef f():\n    return 1\n")).unwrap();
    for args in [
        vec!["has", "--author", "me", "--at", "src/big.py:1", "--expect", "a*b"],
        vec!["has", "--author", "me", "--anchor", "src/big.py::.*q"],
        vec!["has", "--author", "me", "--anchor", "src/big.py::(a|aa)*c"],
    ] {
        let o = cc(ws.path()).args(["claim", "a", "routing.retry"]).args(&args).output().unwrap();
        assert_eq!(o.status.code(), Some(1), "{args:?}: {:?} {}", o.status, String::from_utf8_lossy(&o.stderr));
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        assert_eq!(v["error"]["code"], "invalid_arguments", "{v}");
    }
    // a pattern that does match the long line is accepted
    claim(ws.path(), &["has", "--author", "me", "--at", "src/big.py:1", "--expect", "a+'$", "--no-reach"]).success();
}

// ---- 6: prose files are refused like comments

#[test]
fn a_prose_file_line_is_refused_unless_allow_comment() {
    let (ws, a, _p) = setup();
    for f in ["README.md", "NOTES.txt", "doc.rst"] {
        std::fs::write(a.path().join(f), "The agent retries with backoff.\n").unwrap();
        let m = refused(ws.path(), &["has", "--author", "me", "--at", &format!("{f}:1")]);
        assert!(m.contains("prose"), "{f}: {m}");
        claim(ws.path(), &["has", "--author", "me", "--at", &format!("{f}:1"), "--allow-comment", "--no-reach"]).success();
    }
    std::fs::write(a.path().join("src/doc.py"), "def f():\n    'Retry with backoff.'\n    return 1\n").unwrap();
    let m = refused(ws.path(), &["has", "--author", "me", "--at", "src/doc.py:2"]);
    assert!(m.contains("docstring"), "{m}");
}

// ---- 7: --out-of-subset paths and reversed ranges

#[test]
fn out_of_subset_refuses_dirs_symlinks_and_reversed_ranges() {
    let (ws, a, _p) = setup();
    let oos = |p: &str| refused(ws.path(), &["partial", "--author", "me", "--out-of-subset", p]);
    assert!(oos("src").contains("directory"), "a directory in the tree");
    assert!(oos("x.py:9-3").contains("reversed"), "a reversed range");
    assert!(oos("x.py:0").contains("line"), "line 0");
    std::os::unix::fs::symlink(a.path().join("gone.py"), a.path().join("dangling.py")).unwrap();
    assert!(oos("dangling.py").contains("symlink"), "a dangling symlink");
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(outside.path().join("pkg")).unwrap();
    std::fs::write(outside.path().join("pkg/x.py"), "x = 1\n").unwrap();
    std::os::unix::fs::symlink(outside.path(), a.path().join("ext")).unwrap();
    assert!(oos("ext/pkg/x.py").contains("symlink"), "a symlink out of the clone");
    assert!(oos("ext/pkg/missing.py").contains("symlink"), "never followed out of the clone");
    // an --at reversed range is refused too
    let m = refused(ws.path(), &["has", "--author", "me", "--at", "src/agent.py:5-4"]);
    assert!(m.contains("reversed"), "{m}");
    // a path that is truly absent is still accepted
    claim(ws.path(), &["partial", "--author", "me", "--out-of-subset", "tests/t.py:1-4"]).success();
}

// ---- 9: a stale cell's reach is unknown for `matrix --reach`

const MAIN: &str = "def main():\n    return run_turn(\"hi\")\n\ndef run_turn(msg):\n    return msg.upper()\n";

#[test]
fn matrix_reach_treats_a_stale_cell_as_unknown() {
    let (ws, a, _p) = setup();
    std::fs::write(a.path().join("src/main.py"), MAIN).unwrap();
    cc(ws.path()).args(["build", "a"]).assert().success();
    let v = json(claim(ws.path(), &["has", "--author", "me", "--at", "src/main.py:5"]).success());
    assert_eq!(v["data"]["reach"]["status"], "reached", "{v}");
    let rows = |want: &str| {
        let m = json(cc(ws.path()).args(["matrix", "fam", "--reach", want]).assert().success());
        m["data"]["rows"].as_array().unwrap().clone()
    };
    assert_eq!(rows("reached").len(), 1);
    std::fs::write(a.path().join("src/main.py"), MAIN.replace("msg.upper()", "msg.lower()")).unwrap();
    let _ = cc(ws.path()).arg("check").output().unwrap();
    assert!(rows("reached").is_empty(), "a stale cell does not match --reach reached");
    let u = rows("unknown");
    assert_eq!(u.len(), 1, "{u:?}");
    assert_eq!(u[0]["reach"][0], "reached", "the stored reach is still shown: {u:?}");
    assert_eq!(u[0]["cells"][0], "unknown*");
}
