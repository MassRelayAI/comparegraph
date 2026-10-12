use comparegraph::application::family::parse_pack;
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
/// Three built repos; all share `retry` and `auth`; each has one unique term.
fn corpus() -> (tempfile::TempDir, Vec<tempfile::TempDir>) {
    let ws = tempfile::tempdir().unwrap();
    let mut keep = vec![];
    for (n, uniq) in [("one", "zebra"), ("two", "yak"), ("three", "gnu")] {
        let src = format!(
            "def retry_call():\n    return 1\n\ndef auth_check():\n    return 2\n\ndef {uniq}_thing():\n    return 3\n"
        );
        let c = clone_with(&[("src/net/a.py", &src)]);
        add_repo(ws.path(), c.path(), Some(n), false).unwrap();
        build_repo(ws.path(), n, "reference", &BuildOptions::default()).unwrap();
        keep.push(c);
    }
    (ws, keep)
}
fn run(ws: &std::path::Path, out: &std::path::Path, repos: &str) -> assert_cmd::assert::Assert {
    cc(ws)
        .args(["family", "propose", "fam", "--repos", repos, "--out"])
        .arg(out)
        .assert()
}

#[test]
fn draft_holds_shared_terms_not_unique_and_parses() {
    let (ws, _k) = corpus();
    let out = tempfile::tempdir().unwrap();
    let a = run(ws.path(), out.path(), "one,two,three").success();
    let v: serde_json::Value = serde_json::from_slice(&a.get_output().stdout).unwrap();
    assert_eq!(v["evidence"]["confidence"], "observed");
    let text = std::fs::read_to_string(out.path().join("rows.draft.md")).unwrap();
    assert!(text.contains("retry") && text.contains("auth"), "{text}");
    for u in ["zebra", "yak", "gnu"] {
        assert!(!text.contains(u), "{text}");
    }
    let pack = parse_pack(&text).unwrap();
    assert!(pack
        .rows
        .iter()
        .any(|r| r.slug == "retry" && r.tier == "candidate" && r.weight == 1));
    assert!(pack
        .rows
        .iter()
        .any(|r| r.definition == "draft: seen in 3 of 3 repos (one, three, two)"));
}

#[test]
fn two_runs_are_byte_identical() {
    let (ws, _k) = corpus();
    let out = tempfile::tempdir().unwrap();
    run(ws.path(), out.path(), "one,two,three").success();
    let a = std::fs::read(out.path().join("rows.draft.md")).unwrap();
    run(ws.path(), out.path(), "three,one,two").success();
    assert_eq!(a, std::fs::read(out.path().join("rows.draft.md")).unwrap());
}

#[test]
fn refuses_an_unbuilt_repo_and_never_touches_rows_md() {
    let (ws, _k) = corpus();
    let out = tempfile::tempdir().unwrap();
    std::fs::write(out.path().join("rows.md"), "mine").unwrap();
    run(ws.path(), out.path(), "one,nope").code(1);
    assert!(!out.path().join("rows.draft.md").exists());
    run(ws.path(), out.path(), "one,two").success();
    assert_eq!(
        std::fs::read_to_string(out.path().join("rows.md")).unwrap(),
        "mine"
    );
}

#[test]
fn one_repo_uses_min_repos_one_and_max_caps() {
    let (ws, _k) = corpus();
    let out = tempfile::tempdir().unwrap();
    let a = cc(ws.path())
        .args([
            "family", "propose", "fam", "--repos", "one", "--max", "2", "--out",
        ])
        .arg(out.path())
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&a.get_output().stdout).unwrap();
    assert_eq!(v["data"]["candidates"].as_array().unwrap().len(), 2);
    assert_eq!(v["data"]["min_repos"], 1);
}
