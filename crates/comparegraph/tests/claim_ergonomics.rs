//! CG-2 claim ergonomics: exact-hash re-anchor, expect_literal, a vacuous expect, the `read_hits` alias, --dry-run and a
//! closed pipe. Every test drives the binary.
use comparegraph::application::{add::add_repo, build::build_repo};
use comparegraph::providers::BuildOptions;
use std::process::Command;

fn commit_all(d: &std::path::Path, msg: &str) {
    for args in [
        vec!["add", "."],
        vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", msg],
    ] {
        assert!(Command::new("git").arg("-C").arg(d).args(&args).status().unwrap().success());
    }
}
fn clone_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let f = d.path().join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, c).unwrap();
    }
    std::fs::write(d.path().join("LICENSE"), "MIT License\n\nCopyright (c) 2026 S\n").unwrap();
    assert!(Command::new("git").arg("-C").arg(d.path()).args(["init", "-q"]).status().unwrap().success());
    commit_all(d.path(), "x");
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
    cc(ws.path()).args(["family", "add"]).arg(pack.path()).assert().success();
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
fn lines(out: &[u8]) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(out).lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}
fn history(ws: &std::path::Path) -> Vec<serde_json::Value> {
    json(cc(ws).args(["claims", "--history"]).assert().success())["data"]["items"].as_array().unwrap().clone()
}
fn rebuild(ws: &std::path::Path) {
    build_repo(ws, "a", "reference", &BuildOptions::default()).unwrap();
}

const V1: &str = "def retry_with_backoff():\n    x = compute()\n    return x\n\ndef recall_turns():\n    return 2\n";

fn moved_ws() -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
    let (ws, a, p) = setup();
    std::fs::write(a.path().join("src/net/a.py"), V1).unwrap();
    commit_all(a.path(), "v1");
    rebuild(ws.path());
    claim(ws.path(), &["has", "--author", "me", "--at", "src/net/a.py:1-3"]).success();
    (ws, a, p)
}

// ---- 1. exact-hash re-anchor

#[test]
fn reanchor_carries_a_moved_unchanged_block_by_hash_and_appends() {
    let (ws, a, _p) = moved_ws();
    let old = history(ws.path());
    std::fs::write(a.path().join("src/net/a.py"), format!("import os\n\nVERSION = 1\n\n{V1}")).unwrap();
    commit_all(a.path(), "v2 moves the block down four lines");
    rebuild(ws.path());
    let r = json(claim(ws.path(), &["--reanchor", "--author", "me"]).success());
    assert_eq!(r["data"]["locations"][0]["line_start"], 5, "{r}");
    assert_eq!(r["data"]["locations"][0]["line_end"], 7, "{r}");
    assert_eq!(r["data"]["supersedes_id"], old[0]["id"], "{r}");
    assert!(r["data"]["note"].as_str().unwrap().contains("re-anchored from claim"), "{r}");
    let h = history(ws.path());
    assert_eq!(h.len(), 2, "appended, not rewritten");
    assert_eq!(h[0]["locations"], old[0]["locations"], "the old claim is untouched");
}

#[test]
fn reanchor_refuses_a_changed_block_and_writes_nothing() {
    let (ws, a, _p) = moved_ws();
    std::fs::write(a.path().join("src/net/a.py"), V1.replace("compute()", "compute(2)")).unwrap();
    commit_all(a.path(), "v2 changes the block");
    rebuild(ws.path());
    let r = json(claim(ws.path(), &["--reanchor", "--author", "me"]).failure());
    assert_eq!(r["error"]["code"], "invalid_arguments", "{r}");
    assert!(r["error"]["message"].as_str().unwrap().contains("content hash"), "{r}");
    assert_eq!(history(ws.path()).len(), 1);
}

#[test]
fn reanchor_never_follows_the_hash_into_another_symbol() {
    let body = "    y = same_block()\n    return y\n";
    let v1 = format!("def retry_with_backoff():\n{body}\ndef other():\n{body}");
    let (ws, a, _p) = setup();
    std::fs::write(a.path().join("src/net/a.py"), &v1).unwrap();
    commit_all(a.path(), "v1");
    rebuild(ws.path());
    claim(ws.path(), &["has", "--author", "me", "--at", "src/net/a.py:2-3"]).success();
    // the cited symbol is gone; the same block lives on under `other`: a hash match alone must not carry the claim
    std::fs::write(a.path().join("src/net/a.py"), format!("def other():\n{body}")).unwrap();
    commit_all(a.path(), "v2 drops the cited symbol");
    rebuild(ws.path());
    let r = json(claim(ws.path(), &["--reanchor", "--author", "me"]).failure());
    assert!(r["error"]["message"].as_str().unwrap().contains("none of the symbol"), "{r}");
    assert_eq!(history(ws.path()).len(), 1);
    // and where the block is duplicated, the copy under the cited symbol is the one kept
    let v3 = format!("def other():\n{body}\ndef retry_with_backoff():\n{body}");
    std::fs::write(a.path().join("src/net/a.py"), v3).unwrap();
    commit_all(a.path(), "v3 reorders");
    rebuild(ws.path());
    let r = json(claim(ws.path(), &["--reanchor", "--author", "me"]).success());
    assert_eq!(r["data"]["locations"][0]["line_start"], 6, "{r}");
}

#[test]
fn reanchor_dry_run_writes_nothing() {
    let (ws, a, _p) = moved_ws();
    std::fs::write(a.path().join("src/net/a.py"), format!("# c\n{V1}")).unwrap();
    commit_all(a.path(), "v2");
    rebuild(ws.path());
    let r = json(claim(ws.path(), &["--reanchor", "--dry-run", "--author", "me"]).success());
    assert_eq!(r["data"]["dry_run"], true, "{r}");
    assert_eq!(r["data"]["locations"][0]["line_start"], 2, "{r}");
    assert_eq!(history(ws.path()).len(), 1);
}

// ---- 2. expect_literal

const LIT: &str = "def retry_with_backoff():\n    return a.b(c)[0]\n";

fn lit_ws() -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
    let (ws, a, p) = setup();
    std::fs::write(a.path().join("src/net/a.py"), LIT).unwrap();
    commit_all(a.path(), "lit");
    rebuild(ws.path());
    (ws, a, p)
}

#[test]
fn expect_literal_matches_regex_characters_as_text() {
    let (ws, _a, _p) = lit_ws();
    // as a regex `a.b(c)[0]` wants "a<any>bc0": it does not match the line
    claim(ws.path(), &["has", "--author", "me", "--at", "src/net/a.py:2", "--expect", "a.b(c)[0]"]).failure();
    let r = json(
        claim(ws.path(), &["has", "--author", "me", "--at", "src/net/a.py:2", "--expect", "a.b(c)[0]", "--expect-literal"])
            .success(),
    );
    assert_eq!(r["data"]["verdict"], "has", "{r}");
    // a batch row carries the same option
    let row = r#"{"repo":"a","feature":"memory.recall","verdict":"has","author":"me","at":["src/net/a.py:2"],"expect":["a.b(c)[0]"],"expect_literal":true}"#;
    let path = ws.path().join("lit.jsonl");
    std::fs::write(&path, row.to_string() + "\n").unwrap();
    let a = cc(ws.path()).args(["claim", "--batch"]).arg(&path).assert().success();
    assert_eq!(lines(&a.get_output().stdout)[0]["ok"], true);
}

// ---- 3. a vacuous expect

#[test]
fn a_vacuous_expect_is_refused_and_nothing_is_written() {
    let (ws, _a, _p) = lit_ws();
    for bad in ["", ".*", "^", ".+", "(?s).*"] {
        let r = json(claim(ws.path(), &["has", "--author", "me", "--at", "src/net/a.py:2", "--expect", bad]).failure());
        assert_eq!(r["error"]["code"], "invalid_arguments", "{bad:?}: {r}");
    }
    assert_eq!(history(ws.path()).len(), 0);
    // a real pattern still passes
    claim(ws.path(), &["has", "--author", "me", "--at", "src/net/a.py:2", "--expect", r"\bb\("]).success();
}

// ---- 4. the old field name

#[test]
fn the_old_read_hits_field_is_accepted_with_a_warning() {
    let (ws, _a, _p) = setup();
    let row = r#"{"repo":"a","feature":"routing.retry","verdict":"none","author":"me","searched":"routing zzzunknown qqqmissing","read_hits":["src/net/a.py::retry_with_backoff"]}"#;
    let path = ws.path().join("old.jsonl");
    std::fs::write(&path, row.to_string() + "\n").unwrap();
    let a = cc(ws.path()).args(["claim", "--batch"]).arg(&path).assert().success();
    let out = lines(&a.get_output().stdout);
    assert_eq!(out[0]["ok"], true, "{out:?}");
    assert!(out[0].to_string().contains("deprecated_field"), "{out:?}");
    assert!(String::from_utf8_lossy(&a.get_output().stderr).contains("old name of `hits_read`"));
    // both names at once is ambiguous
    let both = row.replace("\"read_hits\"", "\"hits_read\":[\"x\"],\"read_hits\"");
    std::fs::write(&path, both + "\n").unwrap();
    let a = cc(ws.path()).args(["claim", "--batch"]).arg(&path).assert().failure();
    assert_eq!(lines(&a.get_output().stdout)[0]["ok"], false);
}

// ---- 5. --dry-run

#[test]
fn batch_dry_run_reports_every_row_and_writes_nothing() {
    let (ws, _a, _p) = setup();
    let path = ws.path().join("d.jsonl");
    std::fs::write(
        &path,
        "{\"repo\":\"a\",\"feature\":\"routing.retry\",\"verdict\":\"has\",\"author\":\"me\",\"at\":[\"src/net/a.py:1-2\"]}\n",
    )
    .unwrap();
    let a = cc(ws.path()).args(["claim", "--batch"]).arg(&path).arg("--dry-run").assert().success();
    let out = lines(&a.get_output().stdout);
    assert_eq!(out[0]["ok"], true, "{out:?}");
    assert_eq!(out[1]["data"]["committed"], false, "{out:?}");
    assert_eq!(out[1]["data"]["dry_run"], true, "{out:?}");
    assert_eq!(history(ws.path()).len(), 0);
    // the same file, written for real, lands
    cc(ws.path()).args(["claim", "--batch"]).arg(&path).assert().success();
    assert_eq!(history(ws.path()).len(), 1);
}

#[test]
fn family_add_dry_run_reports_and_writes_nothing() {
    let ws = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), PACK).unwrap();
    let r = json(cc(ws.path()).args(["family", "add", "--dry-run"]).arg(pack.path()).assert().success());
    assert_eq!(r["data"]["dry_run"], true, "{r}");
    assert_eq!(r["data"]["added"].as_array().unwrap().len(), 2, "{r}");
    assert!(!ws.path().join(".comparegraph").exists(), "a dry run creates no store");
    cc(ws.path()).args(["family", "add"]).arg(pack.path()).assert().success();
    std::fs::write(
        pack.path().join("rows.md"),
        PACK.replace("| memory.recall | memory | Recall | Past turns | a store read | extra | 1 |\n", ""),
    )
    .unwrap();
    let r = json(cc(ws.path()).args(["family", "add", "--dry-run"]).arg(pack.path()).assert().success());
    assert_eq!(r["data"]["retired"], serde_json::json!(["memory.recall"]), "{r}");
    let shown = json(cc(ws.path()).args(["family", "show", "fam"]).assert().success());
    assert!(
        shown.to_string().contains("memory.recall") && !shown.to_string().contains("retired"),
        "nothing was retired: {shown}"
    );
}

// ---- 6. a closed pipe

#[test]
fn a_closed_pipe_ends_quietly() {
    let (ws, _a, _p) = setup();
    claim(ws.path(), &["has", "--author", "me", "--at", "src/net/a.py:1-2"]).success();
    let mut child = Command::new(assert_cmd::cargo::cargo_bin("comparegraph"))
        .arg("--root")
        .arg(ws.path())
        .arg("claims")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take()); // the reader (`head`) is gone before the first write
    let out = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("panicked"), "{err}");
    assert_eq!(out.status.code(), Some(0), "{err}");
}

// ---- round 2: atomic re-anchor, dry runs that write nothing, machine-carried labels

fn db_bytes(ws: &std::path::Path) -> Vec<u8> {
    std::fs::read(ws.join(".comparegraph").join("comparegraph.db")).unwrap()
}

#[cfg(unix)]
fn claim_with_env(ws: &std::path::Path, extra: &[&str], hook: &std::path::Path) -> std::process::Output {
    let mut c = cc(ws);
    c.args(["claim", "a", "routing.retry"]).args(extra);
    c.env("COMPAREGRAPH_TEST_REANCHOR_HOOK", hook).env("COMPAREGRAPH_WAIT", "1");
    c.output().unwrap()
}

#[cfg(unix)]
#[test]
fn reanchor_is_atomic_a_rebuild_between_locate_and_write_cannot_change_the_hash() {
    use std::os::unix::fs::PermissionsExt;
    let (ws, a, _p) = moved_ws();
    let old = history(ws.path());
    std::fs::write(a.path().join("src/net/a.py"), format!("import os\n\n{V1}")).unwrap();
    commit_all(a.path(), "v2");
    rebuild(ws.path());
    // the hook runs between locating the spans and writing the claim: it moves the block again and rebuilds
    let hook = ws.path().join("hook.sh");
    let bin = assert_cmd::cargo::cargo_bin("comparegraph");
    let script = format!(
        "#!/bin/sh\ncd \"{c}\" || exit 0\nprintf 'import sys\\nimport re\\n' > tmp.py\ncat src/net/a.py >> tmp.py\nmv tmp.py src/net/a.py\ngit add src/net/a.py\ngit -c user.email=t@t -c user.name=t commit -q -m hook\n\"{b}\" --root \"{w}\" build a >/dev/null 2>&1 || true\n",
        c = a.path().display(),
        b = bin.display(),
        w = ws.path().display()
    );
    std::fs::write(&hook, script).unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = claim_with_env(ws.path(), &["--reanchor", "--author", "me"], &hook);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    if v["ok"] == true {
        // written: it must be exactly the located text
        assert_eq!(v["data"]["locations"][0]["content_hash"], old[0]["locations"][0]["content_hash"], "{v}");
    }
    let h = history(ws.path());
    for c in h.iter().skip(1) {
        assert_eq!(c["locations"][0]["content_hash"], old[0]["locations"][0]["content_hash"], "a claim carried other code: {c}");
    }
}

#[test]
fn dry_runs_never_create_a_store() {
    let ws = tempfile::tempdir().unwrap();
    let path = ws.path().join("d.jsonl");
    std::fs::write(&path, "{\"repo\":\"a\",\"feature\":\"x\",\"verdict\":\"unknown\",\"author\":\"me\"}\n").unwrap();
    cc(ws.path()).args(["claim", "--batch"]).arg(&path).arg("--dry-run").assert().failure();
    assert!(!ws.path().join(".comparegraph").exists(), "claim --batch --dry-run created a store");
    cc(ws.path()).args(["claim", "a", "x", "--reanchor", "--dry-run", "--author", "me"]).assert().failure();
    assert!(!ws.path().join(".comparegraph").exists(), "claim --reanchor --dry-run created a store");
}

#[test]
fn dry_runs_never_migrate_an_older_store() {
    let (ws, a, _p) = moved_ws();
    std::fs::write(a.path().join("src/net/a.py"), format!("# c\n{V1}")).unwrap();
    commit_all(a.path(), "v2");
    rebuild(ws.path());
    {
        let c = rusqlite::Connection::open(ws.path().join(".comparegraph").join("comparegraph.db")).unwrap();
        c.execute_batch("ALTER TABLE run DROP COLUMN head_source;").unwrap();
    }
    let before = db_bytes(ws.path());
    let path = ws.path().join("d.jsonl");
    std::fs::write(&path, "{\"repo\":\"a\",\"feature\":\"routing.retry\",\"verdict\":\"unknown\",\"author\":\"me\"}\n").unwrap();
    let r = cc(ws.path()).args(["claim", "--batch"]).arg(&path).arg("--dry-run").output().unwrap();
    assert!(!r.status.success());
    assert!(String::from_utf8_lossy(&r.stdout).contains("needs migration"), "{}", String::from_utf8_lossy(&r.stdout));
    assert_eq!(db_bytes(ws.path()), before, "batch dry run changed the store");
    let r = claim(ws.path(), &["--reanchor", "--dry-run", "--author", "me"]).failure();
    assert!(String::from_utf8_lossy(&r.get_output().stdout).contains("needs migration"));
    assert_eq!(db_bytes(ws.path()), before, "reanchor dry run changed the store");
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), PACK).unwrap();
    cc(ws.path()).args(["family", "add", "--dry-run"]).arg(pack.path()).assert().success();
    assert_eq!(db_bytes(ws.path()), before, "family add dry run changed the store");
}

#[test]
fn a_reanchored_claim_is_inferred_and_says_it_was_carried() {
    let (ws, a, _p) = moved_ws();
    std::fs::write(a.path().join("src/net/a.py"), format!("# c\n{V1}")).unwrap();
    commit_all(a.path(), "v2");
    rebuild(ws.path());
    let r = json(claim(ws.path(), &["--reanchor", "--author", "me"]).success());
    assert_eq!(r["data"]["confidence"], "inferred", "{r}");
    assert_eq!(r["evidence"]["confidence"], "inferred", "{r}");
    assert!(r["evidence"]["limitations"].to_string().contains("reanchored"), "{r}");
}

#[test]
fn a_line_moved_past_a_return_is_carried_but_labelled() {
    let (ws, a, _p) = setup();
    std::fs::write(a.path().join("src/net/a.py"), "def retry_with_backoff():\n    y = compute()\n    return y\n").unwrap();
    commit_all(a.path(), "v1");
    rebuild(ws.path());
    claim(ws.path(), &["has", "--author", "me", "--at", "src/net/a.py:2"]).success();
    std::fs::write(a.path().join("src/net/a.py"), "def retry_with_backoff():\n    return 0\n    y = compute()\n").unwrap();
    commit_all(a.path(), "v2 the line is now dead code");
    rebuild(ws.path());
    let r = json(claim(ws.path(), &["--reanchor", "--author", "me"]).success());
    assert_eq!(r["data"]["confidence"], "inferred", "{r}");
    assert!(r["evidence"]["limitations"].to_string().contains("reanchored"), "{r}");
}

#[test]
fn a_file_level_citation_is_not_reanchored() {
    let (ws, a, _p) = setup();
    std::fs::write(a.path().join("src/net/a.py"), "VERSION = 1\nNAME = 'a'\n\ndef retry_with_backoff():\n    return 1\n").unwrap();
    commit_all(a.path(), "v1");
    rebuild(ws.path());
    claim(ws.path(), &["has", "--author", "me", "--at", "src/net/a.py:1"]).success();
    std::fs::write(a.path().join("src/net/a.py"), "# c\nVERSION = 1\nNAME = 'a'\n\ndef retry_with_backoff():\n    return 1\n").unwrap();
    commit_all(a.path(), "v2");
    rebuild(ws.path());
    let r = json(claim(ws.path(), &["--reanchor", "--author", "me"]).failure());
    assert!(r["error"]["message"].as_str().unwrap().contains("file scope"), "{r}");
    assert_eq!(history(ws.path()).len(), 1);
}

#[test]
fn reanchor_says_when_the_clone_is_ahead_of_its_build() {
    let (ws, a, _p) = moved_ws();
    std::fs::write(a.path().join("src/net/a.py"), format!("# c\n{V1}")).unwrap();
    commit_all(a.path(), "v2");
    rebuild(ws.path());
    std::fs::write(a.path().join("src/net/a.py"), format!("# c\n# d\n{V1}")).unwrap();
    commit_all(a.path(), "v3, not built");
    let r = json(claim(ws.path(), &["--reanchor", "--author", "me"]).failure());
    assert!(r["error"]["message"].as_str().unwrap().contains("ahead of its latest build"), "{r}");
}

#[test]
fn an_expect_every_nonblank_line_satisfies_is_refused_and_a_failed_literal_reads_as_written() {
    let (ws, _a, _p) = lit_ws();
    for bad in ["..", r"\S+", r"\w"] {
        let r = json(claim(ws.path(), &["has", "--author", "me", "--at", "src/net/a.py:1-2", "--expect", bad]).failure());
        assert!(r["error"]["message"].as_str().unwrap().contains("every non-blank line"), "{bad}: {r}");
    }
    let r = json(
        claim(ws.path(), &["has", "--author", "me", "--at", "src/net/a.py:2", "--expect", "a.b(c)[9]", "--expect-literal"]).failure(),
    );
    let m = r["error"]["message"].as_str().unwrap();
    assert!(m.contains("a.b(c)[9]") && !m.contains("\\."), "{m}");
    assert_eq!(history(ws.path()).len(), 0);
}
