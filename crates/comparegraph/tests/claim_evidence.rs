//! Field-run asks 7, 8, 11, 14 and 15 (docs/reports/2026-11-07-twelve-harness-field-run.md): what a citation must be
//! before a claim is written: `--at-commit`, `--anchor`, `--expect`, the comment/blank/docstring refusal and the
//! `out_of_subset` qualifier.
use comparegraph::application::{add::add_repo, build::build_repo};
use comparegraph::providers::BuildOptions;
use std::process::Command;

const AGENT: &str = "# agent loop\n\
\"\"\"Module docstring\n\
spanning lines.\"\"\"\n\
import os\n\
\n\
def run_turn(msg):\n\
    \"\"\"Run one turn.\"\"\"\n\
    return compact(msg)\n\
\n\
def compact(msg):\n\
    return msg[:10]\n";

const LIB: &str = "//! crate docs\n\
use std::fmt;\n\
\n\
/* a block\n\
 * comment\n\
 */\n\
pub fn glob() -> &'static str {\n\
    \"src/**/*.rs\"\n\
}\n\
#[derive(Debug)]\n\
pub struct S;\n";

const PACK: &str = "---\nfamily: fam\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n\
| routing.retry | routing | Retry with backoff | Failed calls are retried | a retry loop | basics | 2 |\n\
| memory.recall | memory | Recall | Past turns | a store read | extra | 1 |\n";

fn write_tree(d: &std::path::Path) {
    for (p, c) in [("src/agent.py", AGENT), ("src/lib.rs", LIB)] {
        let f = d.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, c).unwrap();
    }
    std::fs::write(d.join("LICENSE"), "MIT License\n\nCopyright (c) 2026 S\n").unwrap();
}

fn git(d: &std::path::Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(d)
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{args:?}");
    String::from_utf8(o.stdout).unwrap().trim().to_string()
}

fn cc(root: &std::path::Path) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(root);
    c
}
fn json(a: assert_cmd::assert::Assert) -> serde_json::Value {
    serde_json::from_slice(&a.get_output().stdout).unwrap()
}

/// (workspace, clone, pack); `with_git` false is a plain export with no head.
fn setup_with(with_git: bool) -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), PACK).unwrap();
    cc(ws.path())
        .args(["family", "add"])
        .arg(pack.path())
        .assert()
        .success();
    let a = tempfile::tempdir().unwrap();
    write_tree(a.path());
    if with_git {
        git(a.path(), &["init", "-q"]);
        git(a.path(), &["add", "."]);
        git(
            a.path(),
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "-m",
                "x",
            ],
        );
    }
    add_repo(ws.path(), a.path(), Some("a"), false).unwrap();
    build_repo(ws.path(), "a", "reference", &BuildOptions::default()).unwrap();
    (ws, a, pack)
}
fn setup() -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
    setup_with(true)
}
fn claim(ws: &std::path::Path, extra: &[&str]) -> assert_cmd::assert::Assert {
    let mut c = cc(ws);
    c.args(["claim", "a", "routing.retry"]);
    c.args(extra);
    c.assert()
}
fn refused(ws: &std::path::Path, extra: &[&str]) -> String {
    let v = json(claim(ws, extra).failure());
    assert_eq!(v["error"]["code"], "invalid_arguments", "{extra:?}: {v}");
    v["error"]["message"].as_str().unwrap().to_string()
}
fn claim_count(ws: &std::path::Path) -> usize {
    let v = json(cc(ws).args(["claims", "--history"]).assert().success());
    v["data"]["items"].as_array().unwrap().len()
}
fn limitation_kinds(v: &serde_json::Value) -> Vec<String> {
    v["evidence"]["limitations"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|l| l["code"].as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default()
}

// ---- item 14: blank, comment and docstring lines are refused unless --allow-comment

#[test]
fn a_bare_line_one_that_is_a_comment_is_refused() {
    let (ws, _a, _p) = setup();
    let m = refused(
        ws.path(),
        &["has", "--author", "me", "--at", "src/agent.py:1"],
    );
    assert!(
        m.contains("comment") && m.contains("--allow-comment"),
        "{m}"
    );
    for at in [
        "src/agent.py:5",   // blank
        "src/agent.py:2-3", // module docstring
        "src/agent.py:7",   // function docstring
        "src/agent.py:1-3", // comment + docstring
        "src/lib.rs:1",     // //! doc comment
        "src/lib.rs:4-6",   // block comment
        "src/lib.rs:5",     // inside the block comment
    ] {
        refused(ws.path(), &["has", "--author", "me", "--at", at]);
    }
    assert_eq!(claim_count(ws.path()), 0, "a refused claim writes nothing");
}

#[test]
fn a_span_with_code_or_allow_comment_is_accepted() {
    let (ws, _a, _p) = setup();
    for at in [
        "src/agent.py:6-8",
        "src/agent.py:5-6",
        "src/lib.rs:7-9",
        "src/lib.rs:8",
        "src/lib.rs:10",
    ] {
        claim(ws.path(), &["has", "--author", "me", "--at", at]).success();
    }
    let v = json(
        claim(
            ws.path(),
            &[
                "has",
                "--author",
                "me",
                "--at",
                "src/agent.py:1",
                "--allow-comment",
            ],
        )
        .success(),
    );
    assert_eq!(v["data"]["locations"][0]["line_start"], 1, "{v}");
}

#[test]
fn a_line_that_is_only_an_import_passes_with_a_limitation() {
    let (ws, _a, _p) = setup();
    let v = json(
        claim(
            ws.path(),
            &["has", "--author", "me", "--at", "src/agent.py:4"],
        )
        .success(),
    );
    assert!(
        limitation_kinds(&v).contains(&"citation_imports_only".to_string()),
        "{v}"
    );
    let v = json(
        claim(
            ws.path(),
            &["has", "--author", "me", "--at", "src/agent.py:4-8"],
        )
        .success(),
    );
    assert!(
        !limitation_kinds(&v).contains(&"citation_imports_only".to_string()),
        "{v}"
    );
}

// ---- item 11: --expect

#[test]
fn expect_is_checked_against_the_cited_span() {
    let (ws, _a, _p) = setup();
    claim(
        ws.path(),
        &[
            "has",
            "--author",
            "me",
            "--at",
            "src/agent.py:6-8",
            "--expect",
            "run_turn",
        ],
    )
    .success();
    claim(
        ws.path(),
        &[
            "has",
            "--author",
            "me",
            "--at",
            "src/agent.py:6-8",
            "--expect",
            r"compact\(msg\)$",
        ],
    )
    .success();
    let m = refused(
        ws.path(),
        &[
            "has",
            "--author",
            "me",
            "--at",
            "src/agent.py:10-11",
            "--expect",
            "def run_turn",
        ],
    );
    assert!(m.contains("does not match") && m.contains("line 6"), "{m}");
    let m = refused(
        ws.path(),
        &[
            "has",
            "--author",
            "me",
            "--at",
            "src/agent.py:6-8",
            "--expect",
            "no_such_symbol",
        ],
    );
    assert!(m.contains("nowhere in the file"), "{m}");
    let m = refused(
        ws.path(),
        &[
            "has",
            "--author",
            "me",
            "--at",
            "src/agent.py:6-8",
            "--expect",
            "(unclosed",
        ],
    );
    assert!(m.contains("--expect"), "{m}");
    assert_eq!(claim_count(ws.path()), 2);
}

#[test]
fn expect_pairs_with_at_in_order_or_one_applies_to_all() {
    let (ws, _a, _p) = setup();
    let two = ["--at", "src/agent.py:6-8", "--at", "src/agent.py:10-11"];
    let mut args = vec!["has", "--author", "me"];
    args.extend(two);
    args.extend(["--expect", "run_turn", "--expect", "compact"]);
    claim(ws.path(), &args).success();
    let mut args = vec!["has", "--author", "me"];
    args.extend(two);
    args.extend(["--expect", "compact"]);
    claim(ws.path(), &args).success();
    let mut args = vec!["has", "--author", "me"];
    args.extend(two);
    args.extend(["--expect", "compact", "--expect", "run_turn"]);
    let m = refused(ws.path(), &args);
    assert!(m.contains("src/agent.py:10-11"), "{m}");
    let m = refused(
        ws.path(),
        &[
            "has",
            "--author",
            "me",
            "--at",
            "src/agent.py:6-8",
            "--expect",
            "a",
            "--expect",
            "b",
            "--expect",
            "c",
        ],
    );
    assert!(m.contains("--expect"), "{m}");
}

// ---- item 8: --anchor

#[test]
fn anchor_resolves_to_one_line_and_is_stored_as_that_line() {
    let (ws, _a, _p) = setup();
    let v = json(
        claim(
            ws.path(),
            &[
                "has",
                "--author",
                "me",
                "--anchor",
                "src/agent.py::^def compact",
            ],
        )
        .success(),
    );
    let l = &v["data"]["locations"][0];
    assert_eq!(
        (
            l["path"].as_str(),
            l["line_start"].as_i64(),
            l["line_end"].as_i64()
        ),
        (Some("src/agent.py"), Some(10), Some(10)),
        "{v}"
    );
    // a regex with `::` inside it splits on the first `::`
    let v = json(
        claim(
            ws.path(),
            &[
                "has",
                "--author",
                "me",
                "--anchor",
                r"src/lib.rs::fn glob\(\)",
            ],
        )
        .success(),
    );
    assert_eq!(v["data"]["locations"][0]["line_start"], 7, "{v}");
    // anchors and --at mix
    let v = json(
        claim(
            ws.path(),
            &[
                "has",
                "--author",
                "me",
                "--at",
                "src/agent.py:6-8",
                "--anchor",
                "src/agent.py::return msg",
            ],
        )
        .success(),
    );
    assert_eq!(v["data"]["locations"].as_array().unwrap().len(), 2, "{v}");
    assert_eq!(v["data"]["locations"][1]["line_start"], 11, "{v}");
}

#[test]
fn anchor_with_zero_or_several_matches_is_refused() {
    let (ws, _a, _p) = setup();
    let m = refused(
        ws.path(),
        &[
            "has",
            "--author",
            "me",
            "--anchor",
            "src/agent.py::nothing_like_this",
        ],
    );
    assert!(m.contains("matches no line"), "{m}");
    let m = refused(
        ws.path(),
        &["has", "--author", "me", "--anchor", "src/agent.py::^def "],
    );
    assert!(
        m.contains("ambiguous") && m.contains("6") && m.contains("10"),
        "{m}"
    );
    let m = refused(
        ws.path(),
        &["has", "--author", "me", "--anchor", "src/agent.py"],
    );
    assert!(m.contains("path::regex"), "{m}");
    let m = refused(
        ws.path(),
        &["has", "--author", "me", "--anchor", "src/missing.py::x"],
    );
    assert!(m.contains("does not exist"), "{m}");
    // a match that is only a comment is not a citation unless --allow-comment
    let m = refused(
        ws.path(),
        &[
            "has",
            "--author",
            "me",
            "--anchor",
            "src/agent.py::agent loop",
        ],
    );
    assert!(m.contains("--allow-comment") && m.contains("1"), "{m}");
    let v = json(
        claim(
            ws.path(),
            &[
                "has",
                "--author",
                "me",
                "--anchor",
                "src/agent.py::agent loop",
                "--allow-comment",
            ],
        )
        .success(),
    );
    assert_eq!(v["data"]["locations"][0]["line_start"], 1, "{v}");
    // a comment mentioning the pattern does not make a code match ambiguous
    let v = json(
        claim(
            ws.path(),
            &[
                "has",
                "--author",
                "me",
                "--anchor",
                "src/agent.py::Run one turn|def run_turn",
            ],
        )
        .success(),
    );
    assert_eq!(v["data"]["locations"][0]["line_start"], 6, "{v}");
    assert_eq!(claim_count(ws.path()), 2);
}

// ---- item 7: --at-commit

#[test]
fn at_commit_must_agree_with_the_head_the_build_read() {
    let (ws, a, _p) = setup();
    let head = git(a.path(), &["rev-parse", "HEAD"]);
    let v = json(
        claim(
            ws.path(),
            &[
                "has",
                "--author",
                "me",
                "--at",
                "src/agent.py:6-8",
                "--at-commit",
                &head,
            ],
        )
        .success(),
    );
    assert_eq!(v["data"]["at_commit"], head.as_str(), "{v}");
    let all = json(cc(ws.path()).args(["claims"]).assert().success());
    assert_eq!(all["data"]["items"][0]["at_commit"], head.as_str(), "{all}");
    // a 7-character prefix agrees too
    claim(
        ws.path(),
        &[
            "has",
            "--author",
            "me",
            "--at",
            "src/agent.py:6-8",
            "--at-commit",
            &head[..7],
        ],
    )
    .success();
    let m = refused(
        ws.path(),
        &[
            "has",
            "--author",
            "me",
            "--at",
            "src/agent.py:6-8",
            "--at-commit",
            "0000000deadbeef",
        ],
    );
    assert!(m.contains(&head[..7]), "{m}");
    let m = refused(
        ws.path(),
        &[
            "has",
            "--author",
            "me",
            "--at",
            "src/agent.py:6-8",
            "--at-commit",
            "not-a-sha",
        ],
    );
    assert!(m.contains("hex"), "{m}");
    refused(
        ws.path(),
        &[
            "has",
            "--author",
            "me",
            "--at",
            "src/agent.py:6-8",
            "--at-commit",
            "abc12",
        ],
    );
}

#[test]
fn at_commit_on_a_clone_without_a_head_is_recorded_with_a_limitation() {
    let (ws, _a, _p) = setup_with(false);
    let sha = "a8b0bc1505aa";
    let v = json(
        claim(
            ws.path(),
            &[
                "has",
                "--author",
                "me",
                "--at",
                "src/agent.py:6-8",
                "--at-commit",
                sha,
            ],
        )
        .success(),
    );
    assert_eq!(v["data"]["at_commit"], sha, "{v}");
    assert!(
        limitation_kinds(&v).contains(&"at_commit_unverified".to_string()),
        "{v}"
    );
}

// ---- item 15: out_of_subset

#[test]
fn out_of_subset_records_the_upstream_path_without_an_at() {
    let (ws, _a, _p) = setup();
    let v = json(
        claim(
            ws.path(),
            &[
                "has",
                "--author",
                "me",
                "--out-of-subset",
                "tests/test_agent.py:3-9",
            ],
        )
        .success(),
    );
    assert_eq!(v["data"]["out_of_subset"], "tests/test_agent.py:3-9", "{v}");
    assert_eq!(v["data"]["confidence"], "inferred", "{v}");
    assert!(
        limitation_kinds(&v).contains(&"out_of_subset".to_string()),
        "{v}"
    );
    assert_eq!(v["data"]["locations"].as_array().unwrap().len(), 0);
    let all = json(cc(ws.path()).args(["claims"]).assert().success());
    assert_eq!(
        all["data"]["items"][0]["out_of_subset"], "tests/test_agent.py:3-9",
        "{all}"
    );
    // with an --at as well, the cited part is checked as usual and the claim is derived
    let v = json(
        claim(
            ws.path(),
            &[
                "partial",
                "--author",
                "me",
                "--at",
                "src/agent.py:6-8",
                "--out-of-subset",
                "tests/test_agent.py",
            ],
        )
        .success(),
    );
    assert_eq!(v["data"]["confidence"], "derived", "{v}");
}

#[test]
fn out_of_subset_refuses_a_path_in_the_tree_an_unsafe_path_and_other_verdicts() {
    let (ws, _a, _p) = setup();
    let m = refused(
        ws.path(),
        &["has", "--author", "me", "--out-of-subset", "src/agent.py:6"],
    );
    assert!(m.contains("--at"), "{m}");
    for p in ["../x.py", "/etc/passwd", "tests/x.py:zz", ""] {
        refused(ws.path(), &["has", "--author", "me", "--out-of-subset", p]);
    }
    refused(
        ws.path(),
        &["unknown", "--author", "me", "--out-of-subset", "tests/x.py"],
    );
    assert_eq!(claim_count(ws.path()), 0);
}

// ---- batch rows carry every new field

fn lines(out: &[u8]) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(out)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn batch_rows_accept_the_new_fields_and_resume_skips_them() {
    let (ws, a, _p) = setup();
    let head = git(a.path(), &["rev-parse", "HEAD"]);
    let rows = [
        format!("{{\"repo\":\"a\",\"feature\":\"routing.retry\",\"verdict\":\"has\",\"author\":\"me\",\"anchor\":[\"src/agent.py::^def compact\"],\"at\":[\"src/agent.py:6-8\"],\"expect\":[\"run_turn\"],\"at_commit\":\"{head}\"}}"),
        "{\"repo\":\"a\",\"feature\":\"memory.recall\",\"verdict\":\"partial\",\"author\":\"me\",\"out_of_subset\":\"tests/t.py:1-4\"}".to_string(),
        "{\"repo\":\"a\",\"feature\":\"memory.recall\",\"verdict\":\"has\",\"author\":\"me\",\"at\":[\"src/agent.py:1\"],\"allow_comment\":true}".to_string(),
        "{\"repo\":\"a\",\"feature\":\"routing.retry\",\"verdict\":\"has\",\"author\":\"me\",\"at\":[\"src/agent.py:1\"]}".to_string(),
    ];
    let path = ws.path().join("rows.jsonl");
    std::fs::write(&path, rows.join("\n") + "\n").unwrap();
    let a1 = cc(ws.path())
        .args(["claim", "--batch"])
        .arg(&path)
        .assert()
        .failure();
    let out = lines(&a1.get_output().stdout);
    assert_eq!(out[0]["ok"], true, "{}", out[0]);
    assert_eq!(out[0]["data"]["at_commit"], head.as_str());
    assert_eq!(
        out[0]["data"]["locations"][0]["line_start"], 6,
        "--at first, then anchors"
    );
    assert_eq!(out[0]["data"]["locations"][1]["line_start"], 10);
    assert_eq!(
        out[1]["data"]["out_of_subset"], "tests/t.py:1-4",
        "{}",
        out[1]
    );
    assert_eq!(out[2]["ok"], true, "{}", out[2]);
    assert_eq!(out[3]["ok"], false, "{}", out[3]);
    assert!(out[3]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("comment"));
    // the first row again, unchanged: it equals the live claim (anchor, expect and at_commit included) and skips
    std::fs::write(&path, rows[..1].join("\n") + "\n").unwrap();
    let out = lines(
        &cc(ws.path())
            .args(["claim", "--batch"])
            .arg(&path)
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(out[0]["data"]["skipped"], true, "{}", out[0]);
    // a different at_commit is a different claim
    let short = &head[..7];
    let row = rows[0].replace(&head, short);
    std::fs::write(&path, row + "\n").unwrap();
    let out = lines(
        &cc(ws.path())
            .args(["claim", "--batch"])
            .arg(&path)
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert!(out[0]["data"]["skipped"].is_null(), "{}", out[0]);
}

#[test]
fn a_store_created_before_the_claim_columns_opens_and_gains_them() {
    let (ws, _a, _p) = setup();
    claim(
        ws.path(),
        &["has", "--author", "me", "--at", "src/agent.py:6-8"],
    )
    .success();
    let db_path = ws.path().join(".comparegraph/comparegraph.db");
    let cols = |db: &rusqlite::Connection| -> Vec<String> {
        db.prepare("PRAGMA table_info(claim)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    {
        let db = rusqlite::Connection::open(&db_path).unwrap();
        db.execute_batch(
            "ALTER TABLE claim DROP COLUMN at_commit; ALTER TABLE claim DROP COLUMN out_of_subset",
        )
        .unwrap();
        assert!(!cols(&db).iter().any(|c| c == "at_commit"));
    }
    let all = json(cc(ws.path()).args(["claims"]).assert().success());
    assert_eq!(all["data"]["items"].as_array().unwrap().len(), 1, "{all}");
    let db = rusqlite::Connection::open(&db_path).unwrap();
    let c = cols(&db);
    assert!(
        c.iter().any(|c| c == "at_commit") && c.iter().any(|c| c == "out_of_subset"),
        "{c:?}"
    );
}
