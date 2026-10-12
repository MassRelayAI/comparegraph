use comparegraph::application::export::csv_field;
use comparegraph::application::{add::add_repo, build::build_repo};
use comparegraph::providers::BuildOptions;
use std::path::Path;
use std::process::Command;

fn clone_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let f = d.path().join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, c).unwrap();
    }
    std::fs::write(d.path().join("LICENSE"), "MIT License\n\nCopyright (c) 2026 S\n").unwrap();
    for args in [vec!["init", "-q"], vec!["add", "."], vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "x"]] {
        assert!(Command::new("git").arg("-C").arg(d.path()).args(&args).status().unwrap().success());
    }
    d
}
fn cc(root: &Path) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(root);
    c
}
const PACK: &str = "---\nfamily: fam\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n\
| routing.retry | routing | Retry | Failed calls are retried | a retry loop | basics | 2 |\n\
| memory.recall | memory | Recall | Past turns | a store read | extra | 1 |\n";

/// workspace with repos b (added first) and a, a decision whose `why` needs quoting
fn ws() -> (tempfile::TempDir, Vec<tempfile::TempDir>) {
    let ws = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), PACK).unwrap();
    cc(ws.path()).args(["family", "add"]).arg(pack.path()).assert().success();
    let mut keep = vec![];
    for (name, src) in [("b", "def b_fn():\n    return 1\n"), ("a", "def retry_with_backoff():\n    return 1\n\ndef caller():\n    return retry_with_backoff()\n")] {
        let c = clone_with(&[("src/m.py", src)]);
        add_repo(ws.path(), c.path(), Some(name), false).unwrap();
        build_repo(ws.path(), name, "reference", &BuildOptions::default()).unwrap();
        keep.push(c);
    }
    cc(ws.path()).args(["claim", "a", "routing.retry", "has", "--author", "me", "--at", "src/m.py:1-2"]).assert().success();
    cc(ws.path()).args(["decide", "a", "adopt", "--why", "good, \"fast\"\nsecond line", "--by", "me"]).assert().success();
    (ws, keep)
}
fn export(ws: &Path, out: &Path, extra: &[&str]) -> assert_cmd::assert::Assert {
    cc(ws).args(["export", "csv", "--out"]).arg(out).args(extra).assert()
}

#[test]
fn field_quoting_follows_rfc_4180() {
    assert_eq!(csv_field("plain"), "plain");
    assert_eq!(csv_field("a,b"), "\"a,b\"");
    assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
    assert_eq!(csv_field("a,\"b\"\nc"), "\"a,\"\"b\"\"\nc\"");
    assert_eq!(csv_field("cr\rhere"), "\"cr\rhere\"");
}

#[test]
fn export_writes_files_quotes_and_is_deterministic() {
    let (w, _k) = ws();
    let (o1, o2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    export(w.path(), o1.path(), &[]).success();
    export(w.path(), o2.path(), &[]).success();
    let mut names: Vec<String> = std::fs::read_dir(o1.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, ["claims.csv", "decisions.csv", "edges.csv", "matrix-fam.csv", "nodes.csv"]);
    for n in &names {
        assert_eq!(std::fs::read(o1.path().join(n)).unwrap(), std::fs::read(o2.path().join(n)).unwrap(), "{n} differs between runs");
    }
    let dec = std::fs::read_to_string(o1.path().join("decisions.csv")).unwrap();
    assert!(dec.starts_with("id,repo,scope,feature,disposition,why,"));
    assert!(dec.contains("\"good, \"\"fast\"\"\nsecond line\""), "{dec}");
    let nodes = std::fs::read_to_string(o1.path().join("nodes.csv")).unwrap();
    assert!(nodes.starts_with("repo,kind,name,qualified_name,path,line_start,line_end,language,content_hash\n"));
    let repos: Vec<&str> = nodes.lines().skip(1).map(|l| l.split(',').next().unwrap()).collect();
    let mut s = repos.clone();
    s.sort();
    assert_eq!(repos, s, "nodes ordered by repo");
    let edges = std::fs::read_to_string(o1.path().join("edges.csv")).unwrap();
    assert!(edges.starts_with("repo,kind,source,target,confidence\n") && edges.lines().count() > 1);
    let claims = std::fs::read_to_string(o1.path().join("claims.csv")).unwrap();
    assert!(claims.contains("routing.retry") && claims.contains("src/m.py:1-2"));
}

#[test]
fn matrix_csv_header_and_column_order() {
    let (w, _k) = ws();
    let o = tempfile::tempdir().unwrap();
    export(w.path(), o.path(), &[]).success();
    let m = std::fs::read_to_string(o.path().join("matrix-fam.csv")).unwrap();
    let mut lines = m.lines();
    // no root repo: columns are the repos by name
    assert_eq!(lines.next().unwrap(), "group,id,tier,weight,a,b,prevalence,gap,differentiator");
    let ids: Vec<&str> = lines.map(|l| l.split(',').nth(1).unwrap()).collect();
    assert_eq!(ids, ["memory.recall", "routing.retry"]);
    assert!(m.contains("routing,routing.retry,basics,2,has,-,"), "{m}");
}

#[test]
fn repo_filter_limits_rows() {
    let (w, _k) = ws();
    let o = tempfile::tempdir().unwrap();
    export(w.path(), o.path(), &["--repo", "b"]).success();
    let nodes = std::fs::read_to_string(o.path().join("nodes.csv")).unwrap();
    assert!(nodes.lines().skip(1).all(|l| l.starts_with("b,")) && nodes.lines().count() > 1);
    let claims = std::fs::read_to_string(o.path().join("claims.csv")).unwrap();
    assert_eq!(claims.lines().count(), 1, "repo b has no claims");
    export(w.path(), o.path(), &["--repo", "zzz"]).code(1);
}

#[test]
fn zero_nodes_is_refused() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone_with(&[("src/m.py", "def f():\n    return 1\n")]);
    add_repo(ws.path(), c.path(), Some("a"), false).unwrap(); // registered, never built
    let o = tempfile::tempdir().unwrap();
    let out = export(ws.path(), &o.path().join("x"), &[]).code(1);
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["error"]["code"], "empty");
}
