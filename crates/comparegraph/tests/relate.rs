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

fn setup() -> (tempfile::TempDir, Vec<tempfile::TempDir>, tempfile::TempDir) {
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
    let b = clone_with(&[("src/x.py", "def paint_widget():\n    return 1\n")]);
    for (n, c) in [("a", &a), ("b", &b)] {
        add_repo(ws.path(), c.path(), Some(n), false).unwrap();
        build_repo(ws.path(), n, "reference", &BuildOptions::default()).unwrap();
    }
    (ws, vec![a, b], pack)
}

#[test]
fn map_finds_the_retry_symbol_as_a_candidate() {
    let (ws, _k, _p) = setup();
    let v = json(
        cc(ws.path())
            .args(["map", "a", "fam", "--feature", "routing.retry"])
            .assert()
            .success(),
    );
    assert_eq!(v["evidence"]["confidence"], "observed");
    let f = &v["data"]["features"][0];
    assert_eq!(f["id"], "routing.retry");
    let s = f["symbols"].as_array().unwrap();
    assert!(
        s.iter().any(|h| h["qualified_name"]
            .as_str()
            .unwrap()
            .contains("retry_with_backoff")
            && h["label"] == "candidate"),
        "{v}"
    );
}

#[test]
fn relate_bands_and_is_idempotent_per_run() {
    let (ws, _k, _p) = setup();
    let v = json(cc(ws.path()).args(["relate"]).assert().success());
    let items = v["data"]["items"].as_array().unwrap();
    let band = |r: &str| items.iter().find(|i| i["repo"] == r).unwrap()["band"].clone();
    assert_eq!(band("a"), "related");
    assert_eq!(band("b"), "unrelated");
    cc(ws.path()).args(["relate"]).assert().success();
    let n: i64 = rusqlite::Connection::open(ws.path().join(".comparegraph/comparegraph.db"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM relatedness", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 2);
}

#[test]
fn map_on_an_unbuilt_repo_is_not_found() {
    let (ws, _k, _p) = setup();
    let c = clone_with(&[("a.py", "def x():\n    pass\n")]);
    add_repo(ws.path(), c.path(), Some("raw"), false).unwrap();
    let a = cc(ws.path()).args(["map", "raw", "fam"]).assert().code(1);
    assert_eq!(json(a)["error"]["code"], "not_found");
    let a = cc(ws.path())
        .args(["relate", "--repo", "raw"])
        .assert()
        .code(1);
    assert_eq!(json(a)["error"]["code"], "not_found");
}

#[test]
fn relate_rescores_when_the_pack_changes_without_a_rebuild() {
    let (ws, _k, pack) = setup();
    let first = json(
        cc(ws.path())
            .args(["relate", "--repo", "a"])
            .assert()
            .success(),
    );
    let s1 = first["data"]["items"][0]["score"].as_f64().unwrap();
    let rows = format!(
        "{PACK}| zebra.qqqq | zebra | Qqqq thing | Nothing like it | nothing | extra | 1 |\n"
    );
    std::fs::write(pack.path().join("rows.md"), rows).unwrap();
    cc(ws.path())
        .args(["family", "add"])
        .arg(pack.path())
        .assert()
        .success();
    let second = json(
        cc(ws.path())
            .args(["relate", "--repo", "a"])
            .assert()
            .success(),
    );
    let s2 = second["data"]["items"][0]["score"].as_f64().unwrap();
    assert!(s2 < s1, "{s1} -> {s2}");
    let n: i64 = rusqlite::Connection::open(ws.path().join(".comparegraph/comparegraph.db"))
        .unwrap()
        .query_row(
            "SELECT COUNT(DISTINCT pack_hash) FROM relatedness",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 2);
    let stored = json(
        cc(ws.path())
            .args(["relate", "--repo", "a", "--stored"])
            .assert()
            .success(),
    );
    assert_eq!(stored["data"]["items"][0]["score"].as_f64().unwrap(), s2);
}
