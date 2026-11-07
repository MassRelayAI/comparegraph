//! Field run 2026-11-07, ask 12: a feature id is never cut in any output. Every verb that names a feature, and every
//! file an export writes, carries the whole id; an id longer than any column width is the probe.
use comparegraph::application::{add::add_repo, build::build_repo};
use comparegraph::providers::BuildOptions;
use std::process::Command;

/// 74 characters, longer than any fixed column; the slug part alone is 66.
const LONG: &str = "rk.secrets-never-in-transcript-and-never-in-the-trace-or-the-provider-log";
const SLUG: &str = "secrets-never-in-transcript-and-never-in-the-trace-or-the-provider-log";
const PREFIX: &str = "secrets-never-in-tr";

fn cc(root: &std::path::Path) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(root);
    c
}

fn clone_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let f = d.path().join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, c).unwrap();
    }
    std::fs::write(d.path().join("LICENSE"), "MIT License\n\nCopyright (c) 2026 S\n").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "."],
        vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "x"],
    ] {
        assert!(Command::new("git").arg("-C").arg(d.path()).args(&args).status().unwrap().success());
    }
    d
}

/// Every occurrence of the id's prefix in `text` continues into the whole slug; the count says it was seen at all.
fn whole_ids(where_: &str, text: &str) -> usize {
    let mut n = 0;
    for (i, _) in text.match_indices(PREFIX) {
        assert!(text[i..].starts_with(SLUG), "{where_}: a cut id at byte {i}: {:?}", &text[i..(i + 80).min(text.len())]);
        n += 1;
    }
    n
}

fn files_under(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            files_under(&p, out);
        } else if let Ok(s) = std::fs::read_to_string(&p) {
            out.push((p.display().to_string(), s));
        }
    }
}

#[test]
fn a_long_feature_id_is_whole_in_every_output() {
    let ws = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(
        pack.path().join("rows.md"),
        format!(
            "---\nfamily: agents\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n\
| {LONG} | rk | Secrets never in transcript | Secrets are redacted | a redaction pass | basics | 2 |\n\
| rk.retry | rk | Retry | Failed calls are retried | a retry loop | common | 1 |\n"
        ),
    )
    .unwrap();
    cc(ws.path()).args(["family", "add"]).arg(pack.path()).assert().success();
    let a = clone_with(&[("src/redact.py", "def redact_secrets(transcript):\n    return transcript\n")]);
    add_repo(ws.path(), a.path(), Some("a"), false).unwrap();
    build_repo(ws.path(), "a", "reference", &BuildOptions::default()).unwrap();
    cc(ws.path())
        .args(["claim", "a", LONG, "has", "--author", "me", "--at", "src/redact.py:1-2"])
        .assert()
        .success();
    cc(ws.path())
        .args(["decide", "a", "adopt", "--why", "w", "--by", "me", "--feature", LONG])
        .assert()
        .success();

    let verbs: Vec<Vec<&str>> = vec![
        vec!["family", "show", "agents"],
        vec!["claims"],
        vec!["claims", "--history"],
        vec!["decisions"],
        vec!["matrix", "agents"],
        vec!["map", "a", "agents"],
        vec!["map", "a", "agents", "--feature", LONG],
        vec!["absent-check", "a", "agents", LONG],
        vec!["export", "json"],
    ];
    for v in &verbs {
        for pretty in [false, true] {
            let mut c = cc(ws.path());
            if pretty {
                c.arg("--pretty");
            }
            let out = c.args(v).assert().success().get_output().stdout.clone();
            let s = String::from_utf8(out).unwrap();
            assert!(whole_ids(&v.join(" "), &s) > 0, "{}: the id is missing: {s}", v.join(" "));
        }
    }

    for what in ["csv", "vault"] {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join(what);
        cc(ws.path()).args(["export", what, "--out"]).arg(&out).assert().success();
        let mut files = Vec::new();
        files_under(&out, &mut files);
        let seen: usize = files.iter().map(|(p, s)| whole_ids(p, s)).sum();
        assert!(seen > 0, "export {what}: the id is in no file");
    }
}
