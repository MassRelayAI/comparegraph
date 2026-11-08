//! Field run 2026-11-07, items 2 and 4: `--read-hits` names that carry a comma, a space or a newline survive; non-code
//! parser nodes (css/scss/html) stay out of a read set by default; a `none` refusal is structured (`none_refused`) and
//! carries the full required `hits_read` list and the `searched` form it was computed for; a `searched` that is a
//! superset of the absent-check terms is accepted.
use comparegraph::application::{add::add_repo, build::build_repo};
use comparegraph::providers::BuildOptions;
use serde_json::Value;
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
    for args in [
        vec!["init", "-q"],
        vec!["add", "."],
        vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "x"],
    ] {
        assert!(Command::new("git").arg("-C").arg(d.path()).args(&args).status().unwrap().success());
    }
    d
}
fn cc(root: &Path) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(root);
    c
}
fn json(a: assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&a.get_output().stdout).unwrap()
}
fn lines(out: &[u8]) -> Vec<Value> {
    String::from_utf8_lossy(out).lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}
const PACK: &str = "---\nfamily: fam\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n\
| retry.retry | retry | Retry | Failed calls are retried | a retry loop | basics | 1 |\n";

fn ws_with(files: &[(&str, &str)]) -> (tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), PACK).unwrap();
    cc(ws.path()).args(["family", "add"]).arg(pack.path()).assert().success();
    let a = clone_with(files);
    add_repo(ws.path(), a.path(), Some("a"), false).unwrap();
    build_repo(ws.path(), "a", "reference", &BuildOptions::default()).unwrap();
    (ws, a)
}
fn names(v: &Value) -> Vec<String> {
    v["data"]["read_set"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["qualified_name"].as_str().unwrap().to_string())
        .collect()
}
fn check(ws: &Path, extra: &[&str]) -> Value {
    json(cc(ws).args(["absent-check", "a", "fam", "retry.retry"]).args(extra).assert().success())
}
/// `claim a retry.retry none --author me --searched <s>` with one `--read-hits` per name
fn none_each(ws: &Path, searched: &str, read: &[String]) -> assert_cmd::assert::Assert {
    let mut c = cc(ws);
    c.args(["claim", "a", "retry.retry", "none", "--author", "me", "--searched", searched]);
    for r in read {
        c.arg("--read-hits").arg(r);
    }
    c.assert()
}

// ---- item 2: names with a comma, a space or a newline ----

const ODD: &[(&str, &str)] = &[
    ("conf/a.yaml", "\"retry policy, fast\": 1\n"),
    ("src/a.ts", "export class Retry { 'retry me'() { return 1; } }\n"),
];

#[test]
fn a_repeated_read_hits_flag_keeps_names_with_a_comma_or_a_space() {
    let (ws, _a) = ws_with(ODD);
    let read = names(&check(ws.path(), &["--searched", "retry"]));
    assert!(read.iter().any(|n| n.contains(", ")), "a comma name is a candidate: {read:?}");
    assert!(read.iter().any(|n| n.contains(' ') && !n.contains(',')), "a space name is a candidate: {read:?}");
    let v = json(none_each(ws.path(), "retry", &read).success());
    assert_eq!(v["data"]["confidence"], "observed", "{v}");
    assert!(v["data"]["note"].as_str().unwrap().contains("retry policy, fast"), "{v}");
}

#[test]
fn batch_hits_read_and_mcp_read_hits_keep_a_comma_name_whole() {
    let (ws, _a) = ws_with(ODD);
    let read = names(&check(ws.path(), &["--searched", "retry"]));
    let row = serde_json::json!({"repo":"a","feature":"retry.retry","verdict":"none","author":"me","searched":"retry","hits_read":read});
    let path = ws.path().join("b.jsonl");
    std::fs::write(&path, format!("{row}\n")).unwrap();
    let a = cc(ws.path()).args(["claim", "--batch"]).arg(&path).assert().success();
    let out = lines(&a.get_output().stdout);
    assert_eq!(out[0]["ok"], true, "{out:?}");
    let req = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"comparegraph_claim","arguments":{
        "repo":"a","feature":"retry.retry","verdict":"none","author":"me2","searched":"retry","read_hits":read}}});
    let out = cc(ws.path()).arg("mcp").write_stdin(req.to_string()).assert().success().get_output().stdout.clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("read and rejected"), "{text}");
}

#[test]
fn a_name_holding_a_newline_survives_the_flag() {
    // a quoted yaml key may span a line; whatever the parser names it, the read set's own name is accepted whole
    let (ws, _a) = ws_with(&[("conf/a.yaml", "? \"retry\n  now\"\n: 1\n"), ("src/a.py", "def retry_x():\n    return 1\n")]);
    let read = names(&check(ws.path(), &["--searched", "retry"]));
    assert!(read.iter().any(|n| n.contains('\n')), "a newline name is a candidate: {read:?}");
    json(none_each(ws.path(), "retry", &read).success());
}

#[test]
fn the_old_comma_form_still_works_for_plain_names() {
    let (ws, _a) = ws_with(&[("src/a.py", "def retry_x():\n    return 1\n\ndef retry_y():\n    return 2\n")]);
    let read = names(&check(ws.path(), &["--searched", "retry"]));
    assert!(read.len() >= 2, "{read:?}");
    json(none_each(ws.path(), "retry", &[read.join(",")]).success());
    json(none_each(ws.path(), "retry", &[read.join(" ")]).success());
}

// ---- item 2: non-code nodes stay out of the read set ----

const MIXED: &[(&str, &str)] = &[
    ("web/a.css", ".retry-btn, .retry-link { color: red }\n.retry .x { color: blue }\n"),
    ("src/a.py", "def retry_x():\n    return 1\n"),
];

#[test]
fn css_nodes_are_left_out_of_the_read_set_unless_asked_for() {
    let (ws, _a) = ws_with(MIXED);
    let v = check(ws.path(), &["--searched", "retry"]);
    let rs = &v["data"]["read_set"];
    assert!(names(&v).iter().all(|n| !n.starts_with("web/a.css")), "{v}");
    assert!(rs["non_code"].as_u64().unwrap() >= 2, "{v}");
    assert_eq!(v["data"]["absent"], false, "the channels still count css hits: {v}");
    let all = check(ws.path(), &["--searched", "retry", "--include-non-code"]);
    assert!(names(&all).iter().any(|n| n.starts_with("web/a.css::")), "{all}");
    assert_eq!(all["data"]["read_set"]["non_code"], 0, "{all}");
    // a none names the code candidates only; reading a css node as well is not a stranger
    let read = names(&v);
    let c = json(none_each(ws.path(), "retry", &read).success());
    assert!(c["data"]["note"].as_str().unwrap().contains("non-code (css/scss/html) candidates set aside"), "{c}");
    json(none_each(ws.path(), "retry", &names(&all)).success());
}

#[test]
fn a_clone_whose_only_hits_are_css_records_a_derived_none_with_searched() {
    let (ws, _a) = ws_with(&[("web/a.css", ".retry-btn { color: red }\n"), ("src/a.py", "def other():\n    return 1\n")]);
    let v = json(none_each(ws.path(), "retry", &[]).success());
    assert_eq!(v["data"]["confidence"], "derived", "{v}");
    let note = v["data"]["note"].as_str().unwrap();
    assert!(note.starts_with("searched (author-supplied): retry → 0 hits; ") && note.contains("non-code"), "{note}");
}

// ---- item 4: the refusal is data ----

const MANY: &str = "def retry_with_backoff():\n    return 1\n\ndef retry_policy():\n    return 3\n";

#[test]
fn a_graph_refusal_carries_the_exact_searched_and_every_name_to_read_and_following_it_records() {
    let (ws, _a) = ws_with(&[("src/net/a.py", MANY)]);
    let r = json(cc(ws.path()).args(["claim", "a", "retry.retry", "none", "--author", "me"]).assert().failure());
    assert_eq!(r["error"]["code"], "none_refused", "{r}");
    let d = &r["error"]["data"];
    assert_eq!(d["reason"], "graph_hits", "{r}");
    assert_eq!(d["searched"], "retry", "{r}");
    assert_eq!(d["terms"], serde_json::json!(["retry"]), "{r}");
    let hits: Vec<String> = serde_json::from_value(d["hits_read"].clone()).unwrap();
    assert_eq!(hits.len(), 2, "{r}");
    assert!(r["error"]["message"].as_str().unwrap().starts_with("the graph has"), "{r}");
    // following the data, unchanged, records the none
    let v = json(none_each(ws.path(), d["searched"].as_str().unwrap(), &hits).success());
    assert_eq!(v["data"]["confidence"], "observed", "{v}");
}

#[test]
fn an_unread_refusal_lists_the_full_required_set_and_the_unread() {
    let (ws, _a) = ws_with(&[("src/net/a.py", MANY)]);
    let all = names(&check(ws.path(), &["--searched", "retry"]));
    let r = json(none_each(ws.path(), "retry", &all[..1]).failure());
    assert_eq!(r["error"]["code"], "none_refused", "{r}");
    let d = &r["error"]["data"];
    assert_eq!(d["reason"], "unread", "{r}");
    assert_eq!(d["hits_read"], serde_json::json!(all), "{r}");
    assert_eq!(d["unread"], serde_json::json!(all[1..]), "{r}");
    assert_eq!(d["searched"], "retry", "{r}");
    // the same data reaches a batch row
    let row = serde_json::json!({"repo":"a","feature":"retry.retry","verdict":"none","author":"me","searched":"retry","hits_read":&all[..1]});
    let path = ws.path().join("b.jsonl");
    std::fs::write(&path, format!("{row}\n")).unwrap();
    let a = cc(ws.path()).args(["claim", "--batch"]).arg(&path).assert().failure();
    let out = lines(&a.get_output().stdout);
    assert_eq!(out[0]["error"]["code"], "none_refused", "{out:?}");
    assert_eq!(out[0]["error"]["data"]["hits_read"], serde_json::json!(all), "{out:?}");
    // and an MCP call
    let req = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"comparegraph_claim","arguments":{
        "repo":"a","feature":"retry.retry","verdict":"none","author":"me","searched":"retry","read_hits":&all[..1]}}});
    let out = cc(ws.path()).arg("mcp").write_stdin(req.to_string()).assert().success().get_output().stdout.clone();
    let resp: Value = serde_json::from_slice(&out).unwrap();
    let env: Value = serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(env["error"]["data"]["unread"], serde_json::json!(all[1..]), "{env}");
}

#[test]
fn no_read_hits_and_stranger_refusals_are_structured() {
    let (ws, _a) = ws_with(&[("src/net/a.py", MANY)]);
    let all = names(&check(ws.path(), &["--searched", "retry"]));
    let r = json(none_each(ws.path(), "retry", &[]).failure());
    assert_eq!(r["error"]["data"]["reason"], "no_read_hits", "{r}");
    assert_eq!(r["error"]["data"]["hits_read"], serde_json::json!(all), "{r}");
    let mut read = all.clone();
    read.push("src/net/a.py::nothing_like_it".into());
    let r = json(none_each(ws.path(), "retry", &read).failure());
    assert_eq!(r["error"]["data"]["reason"], "strangers", "{r}");
    assert_eq!(r["error"]["data"]["strangers"], serde_json::json!(["src/net/a.py::nothing_like_it"]), "{r}");
}

/// 11 `retry_<n>` functions and one `jitter`: the search `retry jitter` ranks `jitter` (an exact name) into the top 10,
/// so its top 10 differs from the top 10 of `retry` alone.
fn superset_ws() -> (tempfile::TempDir, tempfile::TempDir) {
    let mut src = String::from("def jitter():\n    return 0\n\n");
    for n in 1..=11 {
        src.push_str(&format!("def retry_{n:02}():\n    return {n}\n\n"));
    }
    ws_with(&[("src/net/a.py", &src)])
}

#[test]
fn a_searched_superset_of_the_terms_is_judged_on_the_read_set_of_the_terms() {
    let (ws, _a) = superset_ws();
    let plain = check(ws.path(), &[]);
    assert_eq!(plain["data"]["terms"], serde_json::json!(["retry"]), "{plain}");
    let base = names(&plain);
    let wide = names(&check(ws.path(), &["--searched", "retry jitter"]));
    assert_ne!(base, wide, "the test needs the two read sets to differ");
    // the absent-check list (no --searched) is accepted under the superset `retry jitter`
    let v = json(none_each(ws.path(), "retry jitter", &base).success());
    let note = v["data"]["note"].as_str().unwrap();
    assert!(note.contains("judged on the read set of the terms \"retry\" (searched is a superset)"), "{note}");
    // the wide list is accepted too
    json(none_each(ws.path(), "retry jitter", &wide).success());
    // not a superset: a searched without the term `retry` is judged on its own read set only
    let r = json(
        cc(ws.path())
            .args(["claim", "a", "retry.retry", "none", "--author", "me", "--searched", "retried jitter"])
            .args(base.iter().flat_map(|n| ["--read-hits", n.as_str()]))
            .assert()
            .failure(),
    );
    assert_eq!(r["error"]["code"], "none_refused", "{r}");
}

#[test]
fn a_superset_refusal_names_the_terms_read_set() {
    let (ws, _a) = superset_ws();
    let base = names(&check(ws.path(), &[]));
    let r = json(none_each(ws.path(), "retry jitter", &base[..9]).failure());
    let d = &r["error"]["data"];
    assert_eq!(d["reason"], "unread", "{r}");
    assert_eq!(d["searched"], "retry jitter", "{r}");
    assert_eq!(d["hits_read"].as_array().unwrap().len(), 10, "{r}");
    assert_eq!(d["unread"], serde_json::json!([base[9]]), "{r}");
}
