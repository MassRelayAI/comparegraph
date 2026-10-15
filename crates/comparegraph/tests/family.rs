use assert_cmd::Command;
use comparegraph::application::family::{add_family, parse_pack};

const HEAD: &str = "---\nfamily: agent-harness\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n";
const R1: &str =
    "| routing.retry | routing | Retry | Failed calls are retried | a retry loop | basics | 2 |\n";
const R2: &str = "| routing.fallback | routing | Fallback | Another provider is tried | a fallback list | common | 1 |\n";
const R3: &str =
    "| memory.recall | memory | Recall | Past turns are recalled | a store read | extra | 3 |\n";

fn cc(root: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(root);
    c
}
fn pack(rows: &[&str]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("rows.md"), format!("{HEAD}{}", rows.concat())).unwrap();
    d
}
fn err_of(rows: &[&str]) -> String {
    parse_pack(&format!("{HEAD}{}", rows.concat()))
        .unwrap_err()
        .to_string()
}

#[test]
fn parses_a_valid_pack() {
    let p = parse_pack(&format!("{HEAD}{R1}{R2}{R3}")).unwrap();
    assert_eq!(p.family, "agent-harness");
    assert_eq!(p.rows.len(), 3);
    assert_eq!(p.rows[2].group, "memory");
    assert_eq!(p.rows[0].slug, "retry");
    assert_eq!(p.rows[0].weight, 2);
}

#[test]
fn refuses_bad_rows_naming_the_row() {
    assert!(err_of(&[R1, R1]).contains("row 2") && err_of(&[R1, R1]).contains("duplicate"));
    let e = err_of(&[R1, "| a.b | a | T |  | c | basics | 1 |\n"]);
    assert!(e.contains("row 2") && e.contains("definition"), "{e}");
    let e = err_of(&["| a.b | a | T | d | c | wild | 1 |\n"]);
    assert!(e.contains("row 1") && e.contains("tier"), "{e}");
    let e = err_of(&[R1, R2, "| a.b | a | T | d | c | basics | 4 |\n"]);
    assert!(e.contains("row 3") && e.contains("weight"), "{e}");
    let e = err_of(&[R1, "| a.b | a | T | d | c | basics |\n"]);
    assert!(e.contains("row 2") && e.contains("cells"), "{e}");
    let e = err_of(&["| ab | a | T | d | c | basics | 1 |\n"]);
    assert!(e.contains("row 1") && e.contains("id"), "{e}");
}

#[test]
fn a_bad_pack_writes_nothing() {
    let ws = tempfile::tempdir().unwrap();
    let p = pack(&[R1, R1]);
    let a = cc(ws.path())
        .args(["family", "add"])
        .arg(p.path())
        .assert()
        .code(1);
    let v: serde_json::Value = serde_json::from_slice(&a.get_output().stdout).unwrap();
    assert_eq!(v["error"]["code"], "invalid_arguments");
    assert!(!ws.path().join(".comparegraph").exists());
    // and with a store present: still no family row
    let good = pack(&[R1]);
    cc(ws.path())
        .args(["family", "add"])
        .arg(good.path())
        .assert()
        .success();
    let bad = pack(&[R2, R2]);
    cc(ws.path())
        .args(["family", "add"])
        .arg(bad.path())
        .assert()
        .code(1);
    let s = cc(ws.path())
        .args(["family", "show", "agent-harness"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&s.get_output().stdout).unwrap();
    assert_eq!(
        v["data"]["groups"][0]["features"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn re_adding_without_a_feature_retires_it_and_keeps_claims() {
    let ws = tempfile::tempdir().unwrap();
    let p = pack(&[R1, R2, R3]);
    cc(ws.path())
        .args(["family", "add"])
        .arg(p.path())
        .assert()
        .success();
    // a claim row keyed to a feature (inserted directly: the claim verb arrives in Task 4)
    let db = rusqlite::Connection::open(ws.path().join(".comparegraph/comparegraph.db")).unwrap();
    db.execute_batch(
        "INSERT INTO repo(name, path, added_at) VALUES ('r', '/x', 't');
         INSERT INTO run(repo_id, provider, provider_version, started_at, status) VALUES (1, 'reference', '1', 't', 'done');",
    )
    .unwrap();
    let fid: i64 = db
        .query_row(
            "SELECT id FROM feature WHERE slug = 'memory.recall'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute(
        "INSERT INTO claim(repo_id, feature_id, run_id, verdict, confidence, author, created_at) VALUES (1, ?1, 1, 'has', 'observed', 'a', 't')",
        [fid],
    )
    .unwrap();
    drop(db);
    let p2 = pack(&[R1, R2]); // memory.recall removed
    let a = cc(ws.path())
        .args(["family", "add"])
        .arg(p2.path())
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&a.get_output().stdout).unwrap();
    assert_eq!(v["data"]["retired"], serde_json::json!(["memory.recall"]));
    assert_eq!(v["evidence"]["confidence"], "observed");
    let db = rusqlite::Connection::open(ws.path().join(".comparegraph/comparegraph.db")).unwrap();
    let (fid2, tier): (i64, String) = db
        .query_row(
            "SELECT id, tier FROM feature WHERE slug = 'memory.recall'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((fid2, tier.as_str()), (fid, "retired"));
    let n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM claim WHERE feature_id = ?1",
            [fid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
    // re-adding it brings it back as active
    cc(ws.path())
        .args(["family", "add"])
        .arg(p.path())
        .assert()
        .success();
    let t: String = db
        .query_row(
            "SELECT tier FROM feature WHERE slug = 'memory.recall'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(t, "extra");
}

#[test]
fn list_and_show_envelopes() {
    let ws = tempfile::tempdir().unwrap();
    let p = pack(&[R1, R2, R3]);
    let a = cc(ws.path())
        .args(["family", "add"])
        .arg(p.path())
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&a.get_output().stdout).unwrap();
    assert_eq!(v["verb"], "family");
    assert_eq!(v["data"]["features"], 3);
    assert_eq!(
        v["data"]["definition_path"],
        ".comparegraph/families/agent-harness/rows.md"
    );
    let l = cc(ws.path()).args(["family", "list"]).assert().success();
    let v: serde_json::Value = serde_json::from_slice(&l.get_output().stdout).unwrap();
    assert_eq!(v["data"]["items"][0]["name"], "agent-harness");
    assert_eq!(v["data"]["items"][0]["active"], 3);
    assert_eq!(v["data"]["items"][0]["description"], "d");
    assert_eq!(v["data"]["items"][0]["groups"], 2);
    let s = cc(ws.path())
        .args(["family", "show", "agent-harness"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&s.get_output().stdout).unwrap();
    assert_eq!(v["data"]["groups"][0]["group"], "memory");
    assert_eq!(v["data"]["description"], "d");
    assert_eq!(v["data"]["groups"][1]["features"][0]["title"], "Fallback");
    assert_eq!(
        v["data"]["groups"][1]["features"][0]["slug"],
        "routing.fallback"
    );
    let n = cc(ws.path())
        .args(["family", "show", "nope"])
        .assert()
        .code(1);
    let v: serde_json::Value = serde_json::from_slice(&n.get_output().stdout).unwrap();
    assert_eq!(v["error"]["code"], "not_found");
}

#[test]
fn doctor_without_a_workspace_is_ok_with_a_null_store() {
    let ws = tempfile::tempdir().unwrap();
    let a = cc(ws.path()).arg("doctor").assert().success();
    let v: serde_json::Value = serde_json::from_slice(&a.get_output().stdout).unwrap();
    assert_eq!(v["ok"], true);
    assert!(v["data"]["store"].is_null());
    assert_eq!(v["data"]["providers"][0]["name"], "reference");
    assert!(!ws.path().join(".comparegraph").exists());
}

#[test]
fn add_family_function_is_usable_directly() {
    let ws = tempfile::tempdir().unwrap();
    let p = pack(&[R1]);
    let r = add_family(ws.path(), p.path()).unwrap();
    assert_eq!(r.added, vec!["routing.retry".to_string()]);
}

#[test]
fn two_groups_may_share_a_slug() {
    let ws = tempfile::tempdir().unwrap();
    let a = "| routing.retry | routing | Retry | r | c | basics | 1 |\n";
    let b = "| memory.retry | memory | Retry mem | r | c | basics | 1 |\n";
    let p = pack(&[a, b]);
    let r = add_family(ws.path(), p.path()).unwrap();
    assert_eq!(r.features, 2);
    let s = comparegraph::application::family::show_family(ws.path(), "agent-harness").unwrap();
    assert_eq!(s.groups.len(), 2);
    assert!(err_of(&[a, a]).contains("duplicate"));
}

const HEAD8: &str = "---\nfamily: agent-harness\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight | synonyms |\n|---|---|---|---|---|---|---|---|\n";

#[test]
fn the_synonyms_column_is_optional_and_stored() {
    // seven columns: loads, no synonyms
    let p = parse_pack(&format!("{HEAD}{R1}")).unwrap();
    assert_eq!(p.rows[0].synonyms, "");
    // eight columns: loads, the cell is kept (and may be empty)
    let text = format!(
        "{HEAD8}| routing.retry | routing | Retry | Failed calls are retried | a retry loop | basics | 2 | reattempt, backoff |\n\
         | memory.recall | memory | Recall | Past turns | a store read | extra | 1 |  |\n"
    );
    let p = parse_pack(&text).unwrap();
    assert_eq!(p.rows[0].synonyms, "reattempt, backoff");
    assert_eq!(p.rows[1].synonyms, "");
    // stored in the feature row
    let ws = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("rows.md"), &text).unwrap();
    add_family(ws.path(), d.path()).unwrap();
    let db = rusqlite::Connection::open(ws.path().join(".comparegraph/comparegraph.db")).unwrap();
    let s: String = db
        .query_row("SELECT synonyms FROM feature WHERE slug = 'routing.retry'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(s, "reattempt, backoff");
    // a row of the wrong width under either header is refused naming the row
    let e = parse_pack(&format!("{HEAD8}| routing.retry | routing | Retry | d | c | basics | 2 |\n")).unwrap_err().to_string();
    assert!(e.contains("row 1") && e.contains("cells"), "{e}");
}

#[test]
fn a_ninth_column_or_a_misnamed_eighth_fails() {
    let nine = "---\nfamily: agent-harness\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight | synonyms | extra |\n|---|---|---|---|---|---|---|---|---|\n\
                | routing.retry | routing | Retry | d | c | basics | 2 | s | x |\n";
    let e = parse_pack(nine).unwrap_err().to_string();
    assert!(e.contains("table header must be"), "{e}");
    let wrong = HEAD8.replace("synonyms", "aliases");
    let e = parse_pack(&format!("{wrong}| routing.retry | routing | Retry | d | c | basics | 2 | s |\n")).unwrap_err().to_string();
    assert!(e.contains("table header must be"), "{e}");
}

const SNAP: &str = ".comparegraph/families/agent-harness/rows.md";
fn jv(a: &assert_cmd::assert::Assert) -> serde_json::Value {
    serde_json::from_slice(&a.get_output().stdout).unwrap()
}
fn sha256_hex(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn add_snapshots_the_pack_and_records_hash_and_source() {
    let ws = tempfile::tempdir().unwrap();
    let p = pack(&[R1, R2]);
    let a = cc(ws.path()).args(["family", "add"]).arg(p.path()).assert().success();
    let v = jv(&a);
    let snap = ws.path().join(SNAP);
    let text = std::fs::read_to_string(&snap).unwrap();
    assert_eq!(text, std::fs::read_to_string(p.path().join("rows.md")).unwrap());
    assert_eq!(v["data"]["definition_path"], SNAP);
    let h = v["data"]["definition_hash"].as_str().unwrap();
    assert_eq!(h.len(), 64);
    assert!(h.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    assert_eq!(h, sha256_hex(text.as_bytes()));
    let src = p.path().canonicalize().unwrap().to_string_lossy().to_string();
    assert_eq!(v["data"]["source_path"], src);
    let s = jv(&cc(ws.path()).args(["family", "show", "agent-harness"]).assert().success());
    assert_eq!(s["data"]["definition_hash"], h);
    assert_eq!(s["data"]["source_path"], src);
    assert_eq!(s["data"]["snapshot_present"], true);
    let l = jv(&cc(ws.path()).args(["family", "list"]).assert().success());
    assert_eq!(l["data"]["items"][0]["snapshot_present"], true);
    assert_eq!(l["data"]["items"][0]["definition_hash"], h);
}

#[test]
fn re_import_overwrites_the_snapshot_and_changes_the_hash() {
    let ws = tempfile::tempdir().unwrap();
    let p = pack(&[R1]);
    let h1 = jv(&cc(ws.path()).args(["family", "add"]).arg(p.path()).assert().success())["data"]["definition_hash"].clone();
    let p2 = pack(&[R1, R2]);
    let h2 = jv(&cc(ws.path()).args(["family", "add"]).arg(p2.path()).assert().success())["data"]["definition_hash"].clone();
    assert_ne!(h1, h2);
    let text = std::fs::read_to_string(ws.path().join(SNAP)).unwrap();
    assert!(text.contains("routing.fallback"));
    assert_eq!(h2.as_str().unwrap(), sha256_hex(text.as_bytes()));
}

#[test]
fn a_missing_snapshot_shows_in_show_and_check_and_an_edited_one_adds_a_note() {
    let ws = tempfile::tempdir().unwrap();
    let p = pack(&[R1]);
    cc(ws.path()).args(["family", "add"]).arg(p.path()).assert().success();
    let ok = jv(&cc(ws.path()).args(["check"]).assert().success());
    assert_eq!(ok["data"]["family_snapshots_missing"], 0);
    // edited after import: a note, not a problem
    let snap = ws.path().join(SNAP);
    let mut t = std::fs::read_to_string(&snap).unwrap();
    t.push_str("\nedited\n");
    std::fs::write(&snap, t).unwrap();
    let ed = jv(&cc(ws.path()).args(["check"]).assert().success());
    assert!(ed["data"]["notes"].to_string().contains("family agent-harness: the pack snapshot was edited after import"), "{ed}");
    // removed: a problem
    std::fs::rename(&snap, ws.path().join("moved.md")).unwrap();
    let s = jv(&cc(ws.path()).args(["family", "show", "agent-harness"]).assert().success());
    assert_eq!(s["data"]["snapshot_present"], false);
    let c = jv(&cc(ws.path()).args(["check"]).assert().code(1));
    assert_eq!(c["data"]["family_snapshots_missing"], 1);
    assert!(c["data"]["problems"].to_string().contains(&format!(
        "family agent-harness: the pack snapshot {SNAP} is missing; re-run family add <dir>"
    )), "{c}");
}

#[test]
fn a_store_created_before_the_snapshot_columns_opens_and_gains_them() {
    let ws = tempfile::tempdir().unwrap();
    let p = pack(&[R1]);
    cc(ws.path()).args(["family", "add"]).arg(p.path()).assert().success();
    let db_path = ws.path().join(".comparegraph/comparegraph.db");
    let cols = |db: &rusqlite::Connection| -> Vec<String> {
        db.prepare("PRAGMA table_info(family)").unwrap()
            .query_map([], |r| r.get::<_, String>(1)).unwrap()
            .map(Result::unwrap).collect()
    };
    {
        let db = rusqlite::Connection::open(&db_path).unwrap();
        db.execute_batch("ALTER TABLE family DROP COLUMN definition_hash; ALTER TABLE family DROP COLUMN source_path;").unwrap();
        db.execute("UPDATE family SET definition_path = '/old/abs/dir'", []).unwrap();
        let c = cols(&db);
        assert!(!c.iter().any(|x| x == "definition_hash" || x == "source_path"), "{c:?}");
    }
    let s = jv(&cc(ws.path()).args(["family", "show", "agent-harness"]).assert().success());
    assert_eq!(s["data"]["definition_hash"], "");
    assert_eq!(s["data"]["source_path"], "");
    assert_eq!(s["data"]["snapshot_present"], false);
    let db = rusqlite::Connection::open(&db_path).unwrap();
    let c = cols(&db);
    assert!(c.iter().any(|x| x == "definition_hash") && c.iter().any(|x| x == "source_path"), "{c:?}");
    cc(ws.path()).args(["family", "add"]).arg(p.path()).assert().success();
    let s = jv(&cc(ws.path()).args(["family", "show", "agent-harness"]).assert().success());
    assert_eq!(s["data"]["snapshot_present"], true);
    assert_eq!(s["data"]["definition_hash"].as_str().unwrap().len(), 64);
}

#[test]
fn a_bad_pack_writes_no_snapshot() {
    let ws = tempfile::tempdir().unwrap();
    let good = pack(&[R1]);
    cc(ws.path()).args(["family", "add"]).arg(good.path()).assert().success();
    let before = std::fs::read_to_string(ws.path().join(SNAP)).unwrap();
    let bad = pack(&[R2, R2]);
    cc(ws.path()).args(["family", "add"]).arg(bad.path()).assert().code(1);
    assert_eq!(std::fs::read_to_string(ws.path().join(SNAP)).unwrap(), before);
    let ws2 = tempfile::tempdir().unwrap();
    cc(ws2.path()).args(["family", "add"]).arg(bad.path()).assert().code(1);
    assert!(!ws2.path().join(".comparegraph").exists());
}
