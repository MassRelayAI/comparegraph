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
| routing.retry | routing | Retry | Failed calls are retried | a retry loop | basics | 2 |\n\
| memory.recall | memory | Recall | Past turns | a store read | extra | 1 |\n";
const SRC: &str = "def retry_with_backoff():\n    return 1\n\ndef recall_turns():\n    return 2\n";

/// root `r` plus comparators `b`, `c`; all carry the same source.
fn setup() -> (tempfile::TempDir, Vec<tempfile::TempDir>, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), PACK).unwrap();
    cc(ws.path())
        .args(["family", "add"])
        .arg(pack.path())
        .assert()
        .success();
    let mut keep = vec![];
    for (n, root) in [("r", true), ("c", false), ("b", false)] {
        let d = clone_with(&[("a.py", SRC)]);
        add_repo(ws.path(), d.path(), Some(n), root).unwrap();
        build_repo(ws.path(), n, "reference", &BuildOptions::default()).unwrap();
        keep.push(d);
    }
    (ws, keep, pack)
}
fn claim(ws: &std::path::Path, repo: &str, feat: &str, verdict: &str) {
    let mut a = vec!["claim", repo, feat, verdict, "--author", "me"];
    if verdict == "has" || verdict == "partial" {
        a.extend(["--at", "a.py:1-2"]);
    }
    cc(ws).args(a).assert().success();
}
fn row<'a>(v: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    v["data"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .unwrap()
}

#[test]
fn decide_twice_keeps_both_and_the_second_supersedes() {
    let (ws, _k, _p) = setup();
    let d = |disp: &str| {
        json(
            cc(ws.path())
                .args(["decide", "b", disp, "--why", "because", "--by", "me"])
                .assert()
                .success(),
        )
    };
    let first = d("park");
    let second = d("adopt");
    assert_eq!(second["data"]["supersedes_id"], first["data"]["id"]);
    assert_eq!(second["data"]["scope"], "repo");
    assert_eq!(second["evidence"]["confidence"], "observed");
    let latest = json(
        cc(ws.path())
            .args(["decisions", "--repo", "b"])
            .assert()
            .success(),
    );
    let items = latest["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["disposition"], "adopt");
    let hist = json(
        cc(ws.path())
            .args(["decisions", "--history"])
            .assert()
            .success(),
    );
    assert_eq!(hist["data"]["items"].as_array().unwrap().len(), 2);
}

#[test]
fn decide_scopes_are_independent_and_inputs_are_checked() {
    let (ws, _k, _p) = setup();
    cc(ws.path())
        .args(["decide", "b", "adopt", "--why", "w", "--by", "me"])
        .assert()
        .success();
    let f = json(
        cc(ws.path())
            .args([
                "decide",
                "b",
                "reject",
                "--why",
                "w",
                "--by",
                "me",
                "--feature",
                "routing.retry",
            ])
            .assert()
            .success(),
    );
    assert_eq!(f["data"]["scope"], "feature");
    assert!(f["data"]["supersedes_id"].is_null(), "{f}");
    let v = json(
        cc(ws.path())
            .args(["decide", "b", "adopt", "--why", "  ", "--by", "me"])
            .assert()
            .failure(),
    );
    assert_eq!(v["error"]["code"], "invalid_arguments");
    let v = json(
        cc(ws.path())
            .args(["decide", "b", "maybe", "--why", "w", "--by", "me"])
            .assert()
            .failure(),
    );
    assert_eq!(v["error"]["code"], "invalid_arguments");
    let v = json(
        cc(ws.path())
            .args([
                "decide",
                "b",
                "adopt",
                "--why",
                "w",
                "--by",
                "me",
                "--feature",
                "nope.x",
            ])
            .assert()
            .failure(),
    );
    assert_eq!(v["error"]["code"], "not_found");
}

#[test]
fn matrix_prevalence_excludes_the_root_and_marks_gaps_and_differentiators() {
    let (ws, _k, _p) = setup();
    claim(ws.path(), "b", "routing.retry", "has");
    cc(ws.path())
        .args(["decide", "b", "adopt", "--why", "w", "--by", "me"])
        .assert()
        .success();
    cc(ws.path()).arg("relate").assert().success();
    claim(ws.path(), "r", "memory.recall", "has"); // only the root has recall
    let v = json(cc(ws.path()).args(["matrix", "fam"]).assert().success());
    assert_eq!(v["evidence"]["confidence"], "derived");
    let cols: Vec<&str> = v["data"]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["repo"].as_str().unwrap())
        .collect();
    assert_eq!(cols, ["r", "b", "c"], "{v}");
    assert_eq!(v["data"]["columns"][1]["decision"], "adopt");
    assert!(v["data"]["columns"][1]["relatedness"].is_string(), "{v}");
    let retry = row(&v, "routing.retry");
    assert_eq!(retry["cells"], serde_json::json!(["-", "has", "-"]));
    assert_eq!(retry["prevalence"], 0.5);
    assert_eq!(retry["gap"], true);
    assert_eq!(retry["differentiator"], false);
    let recall = row(&v, "memory.recall");
    assert_eq!(recall["cells"], serde_json::json!(["has", "-", "-"]));
    assert_eq!(recall["prevalence"], 0.0);
    assert_eq!(recall["gap"], false);
    assert_eq!(recall["differentiator"], true);
}

#[test]
fn without_a_root_there_is_no_gap_or_differentiator() {
    let ws = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), PACK).unwrap();
    cc(ws.path())
        .args(["family", "add"])
        .arg(pack.path())
        .assert()
        .success();
    let d = clone_with(&[("a.py", SRC)]);
    add_repo(ws.path(), d.path(), Some("b"), false).unwrap();
    build_repo(ws.path(), "b", "reference", &BuildOptions::default()).unwrap();
    claim(ws.path(), "b", "routing.retry", "has");
    let v = json(cc(ws.path()).args(["matrix", "fam"]).assert().success());
    let r = row(&v, "routing.retry");
    assert_eq!(r["prevalence"], 1.0);
    assert!(r["gap"].is_null() && r["differentiator"].is_null(), "{v}");
}

#[test]
fn editing_cited_lines_turns_the_cell_into_unknown_star_and_out_of_prevalence() {
    let (ws, k, _p) = setup();
    claim(ws.path(), "b", "routing.retry", "has");
    claim(ws.path(), "c", "routing.retry", "has");
    let before = json(cc(ws.path()).args(["matrix", "fam"]).assert().success());
    assert_eq!(row(&before, "routing.retry")["prevalence"], 1.0);
    // k = [r, c, b]; edit b's clone
    std::fs::write(k[2].path().join("a.py"), "changed\nlines\n").unwrap();
    let chk = json(cc(ws.path()).arg("check").assert().failure());
    assert_eq!(chk["data"]["downgraded_claims"], 1, "{chk}");
    let v = json(cc(ws.path()).args(["matrix", "fam"]).assert().success());
    let r = row(&v, "routing.retry");
    assert_eq!(
        r["cells"],
        serde_json::json!(["-", "unknown*", "has"]),
        "{v}"
    );
    assert_eq!(r["prevalence"], 0.5);
}

#[test]
fn matrix_lists_active_features_and_unknown_family_is_not_found() {
    let (ws, _k, _p) = setup();
    let v = json(cc(ws.path()).args(["matrix", "fam"]).assert().success());
    assert_eq!(v["data"]["rows"].as_array().unwrap().len(), 2);
    let v = json(cc(ws.path()).args(["matrix", "nope"]).assert().failure());
    assert_eq!(v["error"]["code"], "not_found");
}

#[test]
fn decide_feature_is_disambiguated_by_family() {
    let (ws, _k, _p) = setup();
    let other = tempfile::tempdir().unwrap();
    std::fs::write(
        other.path().join("rows.md"),
        PACK.replace("family: fam", "family: fam2"),
    )
    .unwrap();
    cc(ws.path())
        .args(["family", "add"])
        .arg(other.path())
        .assert()
        .success();
    let v = json(
        cc(ws.path())
            .args([
                "decide",
                "b",
                "adopt",
                "--why",
                "w",
                "--by",
                "me",
                "--feature",
                "routing.retry",
            ])
            .assert()
            .failure(),
    );
    assert_eq!(v["error"]["code"], "invalid_arguments");
    let v = json(
        cc(ws.path())
            .args([
                "decide",
                "b",
                "adopt",
                "--why",
                "w",
                "--by",
                "me",
                "--feature",
                "routing.retry",
                "--family",
                "fam2",
            ])
            .assert()
            .success(),
    );
    assert_eq!(v["data"]["scope"], "feature");
}
