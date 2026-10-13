use comparegraph::application::{add::add_repo, build::build_repo};
use comparegraph::providers::BuildOptions;
use std::process::Command;

fn clone_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let f = d.path().join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, c).unwrap();
    }
    std::fs::write(
        d.path().join("LICENSE"),
        "MIT License\n\nCopyright (c) 2026 S\n",
    )
    .unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "."],
        vec![
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "x",
        ],
    ] {
        assert!(Command::new("git")
            .arg("-C")
            .arg(d.path())
            .args(&args)
            .status()
            .unwrap()
            .success());
    }
    d
}
fn cc(root: &std::path::Path) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(root);
    c
}
fn json(a: assert_cmd::assert::Assert) -> serde_json::Value {
    serde_json::from_slice(&a.get_output().stdout).unwrap()
}
const PACK: &str = "---\nfamily: fam\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n\
| routing.retry | routing | Retry with backoff | Failed calls are retried | a retry loop | basics | 2 |\n\
| memory.recall | memory | Recall | Past turns | a store read | extra | 1 |\n";

fn setup() -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), PACK).unwrap();
    cc(ws.path())
        .args(["family", "add"])
        .arg(pack.path())
        .assert()
        .success();
    let a = clone_with(&[(
        "src/net/a.py",
        "def retry_with_backoff():\n    return 1\n\ndef recall_turns():\n    return 2\n",
    )]);
    add_repo(ws.path(), a.path(), Some("a"), false).unwrap();
    build_repo(ws.path(), "a", "reference", &BuildOptions::default()).unwrap();
    (ws, a, pack)
}
fn claim(ws: &std::path::Path, extra: &[&str]) -> assert_cmd::assert::Assert {
    let mut c = cc(ws);
    c.args(["claim", "a", "routing.retry"]);
    c.args(extra);
    c.assert()
}

#[test]
fn has_with_resolving_location_is_derived_with_a_node() {
    let (ws, _a, _p) = setup();
    let v = json(
        claim(
            ws.path(),
            &["has", "--author", "me", "--at", "src/net/a.py:1-2"],
        )
        .success(),
    );
    assert_eq!(v["evidence"]["confidence"], "derived", "{v}");
    assert_eq!(v["data"]["verdict"], "has");
    let l = &v["data"]["locations"][0];
    assert!(l["node_id"].is_i64(), "{v}");
    assert_eq!(l["path"], "src/net/a.py");
    let one = json(
        claim(
            ws.path(),
            &["partial", "--author", "me", "--at", "src/net/a.py:4"],
        )
        .success(),
    );
    assert_eq!(one["data"]["locations"][0]["line_start"], 4);
    assert_eq!(one["data"]["locations"][0]["line_end"], 4);
}

#[test]
fn has_without_at_or_author_is_refused() {
    let (ws, _a, _p) = setup();
    let v = json(claim(ws.path(), &["has", "--author", "me"]).failure());
    assert_eq!(v["error"]["code"], "invalid_arguments", "{v}");
    claim(ws.path(), &["unknown"]).failure();
}

#[test]
#[cfg(unix)]
fn unsafe_and_unresolvable_paths_are_refused_and_nothing_is_written() {
    let (ws, a, _p) = setup();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "x\ny\n").unwrap();
    std::os::unix::fs::symlink(outside.path().join("secret.txt"), a.path().join("link.txt"))
        .unwrap();
    for at in [
        "../x:1",
        "/etc/passwd:1",
        "src/../../x:1",
        "link.txt:1-2",
        "src/net/missing.py:1",
        "src/net/a.py:1-99",
        "src/net/a.py:0-1",
        "src/net/a.py:3-2",
    ] {
        let v = json(claim(ws.path(), &["has", "--author", "me", "--at", at]).failure());
        assert_eq!(v["ok"], false, "{at}");
    }
    let v = json(cc(ws.path()).args(["claims"]).assert().success());
    assert_eq!(v["data"]["items"].as_array().unwrap().len(), 0, "{v}");
}

#[test]
fn none_needs_searched_terms_that_miss() {
    let (ws, _a, _p) = setup();
    let v = json(claim(ws.path(), &["none", "--author", "me"]).failure());
    assert_eq!(v["error"]["code"], "invalid_arguments");
    // either term hits (OR rule)
    json(
        claim(
            ws.path(),
            &["none", "--author", "me", "--searched", "zzzunknown retry"],
        )
        .failure(),
    );
    let ok = json(
        claim(
            ws.path(),
            &[
                "none",
                "--author",
                "me",
                "--searched",
                "routing zzzunknown qqqmissing",
            ],
        )
        .success(),
    );
    assert!(
        ok["data"]["note"]
            .as_str()
            .unwrap()
            .starts_with("searched (author-supplied): routing zzzunknown qqqmissing → 0 hits"),
        "{ok}"
    );
}

#[test]
fn a_second_claim_supersedes_and_history_shows_both() {
    let (ws, _a, _p) = setup();
    claim(
        ws.path(),
        &["has", "--author", "me", "--at", "src/net/a.py:1-2"],
    )
    .success();
    let second = json(claim(ws.path(), &["unknown", "--author", "you"]).success());
    let first_id = second["data"]["supersedes_id"].as_i64().unwrap();
    let v = json(
        cc(ws.path())
            .args(["claims", "--repo", "a"])
            .assert()
            .success(),
    );
    assert_eq!(v["evidence"]["confidence"], "derived");
    let items = v["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["verdict"], "unknown");
    let h = json(
        cc(ws.path())
            .args([
                "claims",
                "--history",
                "--feature",
                "routing.retry",
                "--family",
                "fam",
            ])
            .assert()
            .success(),
    );
    let hi = h["data"]["items"].as_array().unwrap();
    assert_eq!(hi.len(), 2);
    assert!(hi
        .iter()
        .any(|c| c["id"] == first_id && c["verdict"] == "has"));
}

#[test]
fn a_stale_location_shows_the_claim_as_unknown() {
    let (ws, a, _p) = setup();
    claim(
        ws.path(),
        &["has", "--author", "me", "--at", "src/net/a.py:1-2"],
    )
    .success();
    std::fs::write(a.path().join("src/net/a.py"), "changed\nlines\n").unwrap();
    cc(ws.path()).arg("check").assert().failure();
    let v = json(cc(ws.path()).args(["claims"]).assert().success());
    assert_eq!(v["data"]["items"][0]["confidence"], "unknown", "{v}");
}

#[test]
fn unknown_feature_is_not_found() {
    let (ws, _a, _p) = setup();
    let v = json(
        cc(ws.path())
            .args(["claim", "a", "nope.nothing", "unknown", "--author", "me"])
            .assert()
            .failure(),
    );
    assert_eq!(v["error"]["code"], "not_found");
}

#[test]
fn a_retired_feature_refuses_claims() {
    let (ws, _a, pack) = setup();
    let slim = PACK.lines().take(7).collect::<Vec<_>>().join("\n") + "\n";
    std::fs::write(pack.path().join("rows.md"), slim).unwrap();
    cc(ws.path())
        .args(["family", "add"])
        .arg(pack.path())
        .assert()
        .success();
    let v = json(
        cc(ws.path())
            .args(["claim", "a", "memory.recall", "unknown", "--author", "me"])
            .assert()
            .failure(),
    );
    assert_eq!(v["error"]["code"], "invalid_arguments");
    assert!(
        v["error"]["message"].as_str().unwrap().contains("retired"),
        "{v}"
    );
}

#[test]
fn none_with_no_searchable_term_is_refused_and_nothing_is_written() {
    let (ws, _a, _p) = setup();
    for s in ["***", ","] {
        let v = json(claim(ws.path(), &["none", "--author", "me", "--searched", s]).failure());
        assert_eq!(v["error"]["code"], "invalid_arguments", "{v}");
        assert!(
            v["error"]["message"]
                .to_string()
                .contains("searchable term"),
            "{v}"
        );
    }
    let v = json(cc(ws.path()).args(["claims"]).assert().success());
    assert_eq!(v["data"]["items"].as_array().unwrap().len(), 0, "{v}");
}

#[test]
fn a_non_utf8_citation_is_invalid_arguments() {
    let (ws, a, _p) = setup();
    std::fs::write(a.path().join("bin.dat"), [0xff, 0xfe, 0x00, 0x0a, 0xff]).unwrap();
    let v = json(claim(ws.path(), &["has", "--author", "me", "--at", "bin.dat:1"]).failure());
    assert_eq!(v["error"]["code"], "invalid_arguments", "{v}");
    assert!(
        v["error"]["message"].to_string().contains("not UTF-8"),
        "{v}"
    );
}

#[test]
fn vault_shows_a_stale_claim_as_unknown_star() {
    let (ws, a, _p) = setup();
    claim(
        ws.path(),
        &["has", "--author", "me", "--at", "src/net/a.py:1-2"],
    )
    .success();
    std::fs::write(a.path().join("src/net/a.py"), "changed\nlines\n").unwrap();
    cc(ws.path()).arg("check").assert().failure();
    let out = tempfile::tempdir().unwrap();
    cc(ws.path())
        .args(["export", "vault", "--out"])
        .arg(out.path().join("v"))
        .assert()
        .success();
    let feat = std::fs::read_to_string(out.path().join("v/features/fam/routing-retry.md")).unwrap();
    assert!(
        feat.contains("unknown*") && !feat.contains("(stale)"),
        "{feat}"
    );
}

fn lines(out: &[u8]) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(out)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}
const BATCH4: &str = concat!(
    "{\"repo\":\"a\",\"feature\":\"routing.retry\",\"verdict\":\"has\",\"author\":\"me\",\"at\":[\"src/net/a.py:1-2\"]}\n",
    "{\"repo\":\"a\",\"feature\":\"memory.recall\",\"verdict\":\"has\",\"author\":\"me\",\"at\":[\"src/net/a.py:4\"]}\n",
    "\n",
    "{\"repo\":\"a\",\"feature\":\"routing.retry\",\"verdict\":\"unknown\",\"author\":\"me\"}\n",
    "{\"repo\":\"a\",\"feature\":\"memory.recall\",\"verdict\":\"has\",\"author\":\"me\",\"at\":[\"src/net/a.py:1-99\"]}\n",
);

#[test]
fn batch_of_four_writes_three_and_reports_the_failing_row_like_the_single_path() {
    let (ws, _a, _p) = setup();
    let path = ws.path().join("rows.jsonl");
    std::fs::write(&path, BATCH4).unwrap();
    let a = cc(ws.path())
        .args(["claim", "--batch"])
        .arg(&path)
        .assert()
        .failure()
        .code(1);
    let out = lines(&a.get_output().stdout);
    assert_eq!(out.len(), 5, "{out:?}");
    assert_eq!(
        out.iter().map(|v| v["line"].clone()).take(4).collect::<Vec<_>>(),
        [1, 2, 4, 5].map(serde_json::Value::from)
    );
    assert_eq!(
        out[..4].iter().map(|v| v["ok"].as_bool().unwrap()).collect::<Vec<_>>(),
        [true, true, true, false]
    );
    assert!(out[..4].iter().all(|v| v["verb"] == "claim"));
    assert_eq!(out[0]["data"]["verdict"], "has");
    assert_eq!(out[4]["data"], serde_json::json!({"rows":4,"ok":3,"failed":1,"committed":true}));
    let single = json(
        claim(ws.path(), &["has", "--author", "me", "--at", "src/net/a.py:1-99"]).failure(),
    );
    // single-path message for the same citation on the same feature
    let single_msg = {
        let v = json(
            cc(ws.path())
                .args(["claim", "a", "memory.recall", "has", "--author", "me", "--at", "src/net/a.py:1-99"])
                .assert()
                .failure(),
        );
        v["error"]["message"].clone()
    };
    let _ = single;
    assert_eq!(out[3]["error"]["message"], single_msg, "{out:?}");
    assert_eq!(out[3]["error"]["code"], "invalid_arguments");
    let all = json(cc(ws.path()).args(["claims", "--history"]).assert().success());
    assert_eq!(all["data"]["items"].as_array().unwrap().len(), 3, "{all}");
}

#[test]
fn batch_dash_reads_stdin() {
    let (ws, _a, _p) = setup();
    let a = cc(ws.path())
        .args(["claim", "--batch", "-"])
        .write_stdin("{\"repo\":\"a\",\"feature\":\"routing.retry\",\"verdict\":\"unknown\",\"author\":\"me\"}\n")
        .assert()
        .success();
    let out = lines(&a.get_output().stdout);
    assert_eq!(out.len(), 2);
    assert_eq!(out[1]["data"]["ok"], 1);
}

#[test]
fn batch_reports_a_malformed_json_line_and_continues() {
    let (ws, _a, _p) = setup();
    let input = "not json\n{\"repo\":\"a\",\"feature\":\"routing.retry\",\"verdict\":\"unknown\",\"author\":\"me\"}\n";
    let a = cc(ws.path())
        .args(["claim", "--batch", "-"])
        .write_stdin(input)
        .assert()
        .failure()
        .code(1);
    let out = lines(&a.get_output().stdout);
    assert_eq!(out[0]["line"], 1);
    assert_eq!(out[0]["ok"], false);
    assert_eq!(out[0]["error"]["code"], "invalid_arguments");
    assert!(out[0]["error"]["message"].as_str().unwrap().contains("line 1"));
    assert_eq!(out[1]["ok"], true);
    assert_eq!(out[2]["data"]["failed"], 1);
}

#[test]
fn batch_rolls_back_a_row_that_fails_mid_write_and_keeps_the_rest() {
    let (ws, _a, _p) = setup();
    // a trigger that aborts the second claim's location insert, after its claim row is already in: the savepoint must
    // undo that claim row, and the rows around it must land
    let db = ws.path().join(".comparegraph").join("comparegraph.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER forced_failure BEFORE INSERT ON location \
         WHEN (SELECT COUNT(*) FROM claim) = 2 BEGIN SELECT RAISE(ABORT, 'forced mid-write failure'); END;",
    )
    .unwrap();
    drop(conn);
    let rows = concat!(
        "{\"repo\":\"a\",\"feature\":\"routing.retry\",\"verdict\":\"has\",\"author\":\"me\",\"at\":[\"src/net/a.py:1-2\"]}\n",
        "{\"repo\":\"a\",\"feature\":\"memory.recall\",\"verdict\":\"has\",\"author\":\"me\",\"at\":[\"src/net/a.py:4\"]}\n",
        "{\"repo\":\"a\",\"feature\":\"memory.recall\",\"verdict\":\"unknown\",\"author\":\"me\"}\n",
    );
    let path = ws.path().join("rows.jsonl");
    std::fs::write(&path, rows).unwrap();
    let a = cc(ws.path()).args(["claim", "--batch"]).arg(&path).assert().failure().code(1);
    let out = lines(&a.get_output().stdout);
    assert_eq!(out.len(), 4, "{out:?}");
    assert_eq!(out.iter().map(|v| v["ok"].as_bool().unwrap()).collect::<Vec<_>>(), [true, false, true, false]);
    assert!(out[1]["error"]["message"].as_str().unwrap().contains("forced mid-write failure"), "{out:?}");
    assert_eq!(out[3]["data"], serde_json::json!({"rows":3,"ok":2,"failed":1,"committed":true}));
    let conn = rusqlite::Connection::open(&db).unwrap();
    let claims: i64 = conn.query_row("SELECT COUNT(*) FROM claim", [], |r| r.get(0)).unwrap();
    let has_without_location: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM claim c WHERE c.verdict = 'has' AND NOT EXISTS (SELECT 1 FROM location l WHERE l.claim_id = c.id)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!((claims, has_without_location), (2, 0));
}

#[test]
fn batch_on_a_directory_ends_after_one_failed_row_instead_of_looping() {
    let (ws, _a, _p) = setup();
    let a = cc(ws.path())
        .args(["claim", "--batch"])
        .arg(ws.path())
        .timeout(std::time::Duration::from_secs(20))
        .assert()
        .failure()
        .code(1);
    let out = lines(&a.get_output().stdout);
    assert_eq!(out.len(), 2, "{out:?}");
    assert_eq!(out[0]["error"]["code"], "io_error");
    assert_eq!(out[1]["data"]["committed"], true);
}

#[test]
fn batch_ignores_a_byte_order_mark_on_the_first_line() {
    let (ws, _a, _p) = setup();
    let path = ws.path().join("rows.jsonl");
    std::fs::write(
        &path,
        "\u{feff}{\"repo\":\"a\",\"feature\":\"routing.retry\",\"verdict\":\"unknown\",\"author\":\"me\"}\n",
    )
    .unwrap();
    let a = cc(ws.path()).args(["claim", "--batch"]).arg(&path).assert().success();
    let out = lines(&a.get_output().stdout);
    assert_eq!(out[0]["ok"], true, "{out:?}");
    assert_eq!(out[1]["data"]["ok"], 1);
}

const PACK_SYN: &str = "---\nfamily: fam\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight | synonyms |\n|---|---|---|---|---|---|---|---|\n\
| routing.retry | routing | Retry with backoff | Failed calls are retried | a retry loop | basics | 2 | reattempt |\n\
| memory.recall | memory | Recall | Past turns | a store read | extra | 1 |  |\n";

#[test]
fn a_declared_synonym_names_the_feature_in_searched_and_an_undeclared_word_does_not() {
    let (ws, _a, _p) = setup();
    let p2 = tempfile::tempdir().unwrap();
    std::fs::write(p2.path().join("rows.md"), PACK_SYN).unwrap();
    cc(ws.path()).args(["family", "add"]).arg(p2.path()).assert().success();
    // the synonym names the feature, the search finds nothing: recorded with the search in the note
    let ok = json(claim(ws.path(), &["none", "--author", "me", "--searched", "reattempt"]).success());
    let note = ok["data"]["note"].as_str().unwrap();
    assert!(note.starts_with("searched (author-supplied): reattempt → 0 hits"), "{note}");
    // an undeclared unrelated word is still refused
    let bad = json(claim(ws.path(), &["none", "--author", "me", "--searched", "zzzundeclared"]).failure());
    assert_eq!(bad["error"]["code"], "invalid_arguments", "{bad}");
    assert!(bad["error"]["message"].as_str().unwrap().contains("do not name the feature"), "{bad}");
    // the refusal lists the synonym among the words that do name it
    assert!(bad["error"]["message"].as_str().unwrap().contains("reattempt"), "{bad}");
}

#[test]
fn a_store_created_before_the_synonyms_column_opens_and_gains_it() {
    let ws = tempfile::tempdir().unwrap();
    let p = tempfile::tempdir().unwrap();
    std::fs::write(p.path().join("rows.md"), PACK_SYN).unwrap();
    cc(ws.path()).args(["family", "add"]).arg(p.path()).assert().success();
    let db_path = ws.path().join(".comparegraph/comparegraph.db");
    {
        let db = rusqlite::Connection::open(&db_path).unwrap();
        db.execute_batch("ALTER TABLE feature DROP COLUMN synonyms").unwrap();
        let cols: Vec<String> = db
            .prepare("PRAGMA table_info(feature)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(!cols.iter().any(|c| c == "synonyms"), "{cols:?}");
    }
    // any verb opens it; the column is back, empty, and the pack can be added again to fill it
    cc(ws.path()).args(["family", "show", "fam"]).assert().success();
    let has = |db: &rusqlite::Connection| {
        db.prepare("PRAGMA table_info(feature)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(Result::unwrap)
            .any(|c| c == "synonyms")
    };
    let db = rusqlite::Connection::open(&db_path).unwrap();
    assert!(has(&db));
    let s: String = db.query_row("SELECT synonyms FROM feature WHERE slug = 'routing.retry'", [], |r| r.get(0)).unwrap();
    assert_eq!(s, "");
    let v: String = db.query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get(0)).unwrap();
    assert_eq!(v, "1");
    cc(ws.path()).args(["family", "add"]).arg(p.path()).assert().success();
    let s: String = db.query_row("SELECT synonyms FROM feature WHERE slug = 'routing.retry'", [], |r| r.get(0)).unwrap();
    assert_eq!(s, "reattempt");
}
