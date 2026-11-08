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

// ---- `family propose <family> --from-claims <repo> --rows <jsonl>`: draft the ids a standard's scored rows lack ----

const FAM: &str = "---\nfamily: fam\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n| net.retry | net | Retry | retries a call | a retry loop | basics | 1 |\n| net.auth | net | Auth | checks auth | an auth check | common | 2 |\n";

/// The corpus plus family `fam` (net.retry, net.auth) and one stored claim of `one` on net.retry.
fn standard() -> (tempfile::TempDir, Vec<tempfile::TempDir>, tempfile::TempDir) {
    let (ws, keep) = corpus();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), FAM).unwrap();
    cc(ws.path())
        .args(["family", "add"])
        .arg(pack.path())
        .assert()
        .success();
    cc(ws.path())
        .args([
            "claim",
            "one",
            "net.retry",
            "has",
            "--family",
            "fam",
            "--author",
            "me",
            "--at",
            "src/net/a.py:1-2",
        ])
        .assert()
        .success();
    (ws, keep, pack)
}
const ROWS: &str = "\u{feff}{\"repo\":\"one\",\"feature\":\"net.retry\",\"verdict\":\"has\",\"author\":\"me\"}\n\
{\"repo\":\"one\",\"feature\":\"tools.tool-search\",\"verdict\":\"has\",\"author\":\"me\",\"note\":\"searches the | tool list\\nby name\",\"at\":[\"src/net/a.py:1-2\"]}\n\
\n\
{\"repo\":\"one\",\"feature\":\"Skill curator\",\"title\":\"Skill curator\",\"group\":\"skills\",\"verdict\":\"partial\",\"author\":\"me\"}\n\
{\"repo\":\"one\",\"feature\":\"tools.tool-search\",\"verdict\":\"has\",\"author\":\"other\"}\n\
{\"repo\":\"two\",\"feature\":\"net.only-two\",\"verdict\":\"has\",\"author\":\"me\"}\n\
{\"feature\":\"conformance probes\",\"verdict\":\"none\",\"author\":\"me\"}\n";

fn from_claims(
    ws: &std::path::Path,
    rows: &std::path::Path,
    out: &std::path::Path,
) -> assert_cmd::assert::Assert {
    cc(ws)
        .args(["family", "propose", "fam", "--from-claims", "one", "--rows"])
        .arg(rows)
        .arg("--out")
        .arg(out)
        .assert()
}

#[test]
fn from_claims_drafts_only_the_missing_ids_marked_as_drafts() {
    let (ws, _k, _p) = standard();
    let dir = tempfile::tempdir().unwrap();
    let rows = dir.path().join("rows.jsonl");
    std::fs::write(&rows, ROWS).unwrap();
    let out = tempfile::tempdir().unwrap();
    let a = from_claims(ws.path(), &rows, out.path()).success();
    let v: serde_json::Value = serde_json::from_slice(&a.get_output().stdout).unwrap();
    let d = &v["data"];
    assert_eq!(d["repo"], "one");
    assert_eq!(d["family"], "fam");
    assert_eq!(d["draft"], true);
    let ids: Vec<&str> = d["drafts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].as_str().unwrap())
        .collect();
    // sorted; the row without a repo is the standard's; two rows on one id draft it once; `two`'s row is not drafted
    assert_eq!(
        ids,
        [
            "draft.conformance-probes",
            "skills.skill-curator",
            "tools.tool-search"
        ],
        "{d}"
    );
    assert_eq!(d["covered"], serde_json::json!(["net.retry"]));
    assert_eq!(d["other_repo_rows"], 1);
    assert_eq!(d["stored_claims"], 1);
    let ts = d["drafts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "tools.tool-search")
        .unwrap();
    assert_eq!(ts["lines"], serde_json::json!([2, 5]));
    assert_eq!(ts["at"], serde_json::json!(["src/net/a.py:1-2"]));
    // the draft parses as a pack, every row is a candidate whose text says draft, and the note's pipe and newline are gone
    let path = d["draft_path"].as_str().unwrap();
    assert!(path.ends_with("rows.from-claims.draft.md"), "{path}");
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("\ndraft: true\n"), "{text}");
    let pack = parse_pack(&text).unwrap();
    assert_eq!(pack.family, "fam");
    assert_eq!(pack.rows.len(), 3);
    for r in &pack.rows {
        assert_eq!(r.tier, "candidate");
        assert!(r.definition.starts_with("draft: "), "{r:?}");
        assert!(r.counts_as_has.starts_with("draft: "), "{r:?}");
    }
    let ts = pack
        .rows
        .iter()
        .find(|r| r.id == "tools.tool-search")
        .unwrap();
    assert!(
        ts.definition.contains("searches the / tool list by name"),
        "{}",
        ts.definition
    );
    // deterministic: a second run writes the same bytes; rows.md is never written
    let first = std::fs::read(path).unwrap();
    from_claims(ws.path(), &rows, out.path()).success();
    assert_eq!(first, std::fs::read(path).unwrap());
    assert!(!out.path().join("rows.md").exists());
}

#[test]
fn from_claims_with_nothing_missing_writes_no_draft() {
    let (ws, _k, _p) = standard();
    let dir = tempfile::tempdir().unwrap();
    let rows = dir.path().join("rows.jsonl");
    std::fs::write(
        &rows,
        "{\"repo\":\"one\",\"feature\":\"net.auth\",\"verdict\":\"none\",\"author\":\"me\"}\n",
    )
    .unwrap();
    let out = tempfile::tempdir().unwrap();
    let a = from_claims(ws.path(), &rows, out.path()).success();
    let v: serde_json::Value = serde_json::from_slice(&a.get_output().stdout).unwrap();
    assert_eq!(v["data"]["drafts"], serde_json::json!([]));
    assert!(v["data"]["draft_path"].is_null());
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
}

#[test]
fn from_claims_refuses_bad_input_before_writing() {
    let (ws, _k, _p) = standard();
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let rows = dir.path().join("rows.jsonl");
    // a line that is not a row names its line number
    std::fs::write(&rows, "{\"repo\":\"one\",\"feature\":\"x.y\"}\nnot json\n").unwrap();
    let a = from_claims(ws.path(), &rows, out.path()).code(1);
    let s = String::from_utf8_lossy(&a.get_output().stdout).to_string();
    assert!(s.contains("line 2"), "{s}");
    // an unknown repo and an unknown family are not found
    std::fs::write(&rows, "{\"feature\":\"x.y\"}\n").unwrap();
    for (fam, repo) in [("fam", "nope"), ("nofam", "one")] {
        let a = cc(ws.path())
            .args(["family", "propose", fam, "--from-claims", repo, "--rows"])
            .arg(&rows)
            .arg("--out")
            .arg(out.path())
            .assert()
            .code(1);
        let s = String::from_utf8_lossy(&a.get_output().stdout).to_string();
        assert!(s.contains("not_found"), "{s}");
    }
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
    // --from-claims needs --rows, and does not mix with --repos
    cc(ws.path())
        .args(["family", "propose", "fam", "--from-claims", "one"])
        .assert()
        .failure();
    cc(ws.path())
        .args([
            "family",
            "propose",
            "fam",
            "--from-claims",
            "one",
            "--repos",
            "one",
            "--rows",
        ])
        .arg(&rows)
        .assert()
        .failure();
}

#[test]
fn from_claims_refuses_a_verdict_outside_the_four_naming_the_line() {
    let (ws, _k, _p) = standard();
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let rows = dir.path().join("rows.jsonl");
    std::fs::write(
        &rows,
        "{\"repo\":\"one\",\"feature\":\"x.a\",\"verdict\":\"has\"}\n{\"repo\":\"one\",\"feature\":\"x.b\",\"verdict\":\"has|partial\"}\n",
    )
    .unwrap();
    let a = from_claims(ws.path(), &rows, out.path()).code(1);
    let s = String::from_utf8_lossy(&a.get_output().stdout).to_string();
    assert!(s.contains("invalid_arguments") && s.contains("line 2") && s.contains("verdict"), "{s}");
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0, "nothing is written when a row is refused");
}

#[test]
fn every_from_claims_draft_round_trips_through_parse_pack() {
    let (ws, _k, _p) = standard();
    // a repo whose name carries a pipe: it lands in the definition cell
    let c = clone_with(&[("src/a.py", "def f():\n    return 1\n")]);
    add_repo(ws.path(), c.path(), Some("odd|name"), false).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let rows = dir.path().join("rows.jsonl");
    std::fs::write(
        &rows,
        "{\"feature\":\"x.a\",\"title\":\"A | b\\nc\",\"verdict\":\" PARTIAL \",\"note\":\"n | m\",\"at\":[\"p|q:1\"]}\n\
         {\"feature\":\"Plain name\",\"group\":\"G | H\",\"verdict\":\"unknown\"}\n\
         {\"feature\":\"x.c\"}\n",
    )
    .unwrap();
    for repo in ["one", "odd|name"] {
        let out = tempfile::tempdir().unwrap();
        let a = cc(ws.path())
            .args(["family", "propose", "fam", "--from-claims", repo, "--rows"])
            .arg(&rows)
            .arg("--out")
            .arg(out.path())
            .assert()
            .success();
        let v: serde_json::Value = serde_json::from_slice(&a.get_output().stdout).unwrap();
        let path = v["data"]["draft_path"].as_str().unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        let pack = parse_pack(&text).unwrap_or_else(|e| panic!("{repo}: {e}\n{text}"));
        assert_eq!(pack.rows.len(), 3, "{text}");
    }
}

#[test]
fn family_add_refuses_a_draft_pack_unless_accepted() {
    let (ws, _k, _p) = standard();
    let dir = tempfile::tempdir().unwrap();
    let rows = dir.path().join("rows.jsonl");
    std::fs::write(&rows, ROWS).unwrap();
    let out = tempfile::tempdir().unwrap();
    let a = from_claims(ws.path(), &rows, out.path()).success();
    let v: serde_json::Value = serde_json::from_slice(&a.get_output().stdout).unwrap();
    // the draft renamed to rows.md and added as-is would replace fam's rows (net.retry and net.auth retired)
    std::fs::rename(v["data"]["draft_path"].as_str().unwrap(), out.path().join("rows.md")).unwrap();
    let a = cc(ws.path()).args(["family", "add"]).arg(out.path()).assert().code(1);
    let s = String::from_utf8_lossy(&a.get_output().stdout).to_string();
    assert!(s.contains("draft: true") && s.contains("--accept-draft"), "{s}");
    let show = cc(ws.path()).args(["family", "show", "fam"]).assert().success();
    let s = String::from_utf8_lossy(&show.get_output().stdout).to_string();
    assert!(s.contains("net.retry") && !s.contains("tools.tool-search"), "the refused draft changed the family: {s}");
    cc(ws.path()).args(["family", "add", "--accept-draft"]).arg(out.path()).assert().success();
}
