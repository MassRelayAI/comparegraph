//! ABS: the graph-backed absent verdict. Fixtures/typescript is built; features come from a small pack.
use comparegraph::application::{add::add_repo, build::build_repo};
use comparegraph::providers::BuildOptions;
use std::path::Path;
use std::process::Command;

fn copy_dir(from: &Path, to: &Path) {
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&t).unwrap();
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), t).unwrap();
        }
    }
}
fn git_commit(d: &Path) {
    for args in [
        vec!["init", "-q"],
        vec!["add", "."],
        vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "x"],
    ] {
        assert!(Command::new("git").arg("-C").arg(d).args(&args).status().unwrap().success());
    }
}
fn cc(root: &Path) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(root);
    c
}
fn json(a: assert_cmd::assert::Assert) -> serde_json::Value {
    serde_json::from_slice(&a.get_output().stdout).unwrap()
}
const PACK: &str = "---\nfamily: fam\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n\
| quantum.teleport | quantum | Teleport qubits | x | a teleporter | basics | 1 |\n\
| math.counter | math | Counter | x | a counter | basics | 1 |\n\
| naming.ountin | naming | Ountin | x | a mid-token path match | basics | 1 |\n\
| quality.tests | quality | Unit tests | x | tests | basics | 1 |\n";

fn setup() -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), PACK).unwrap();
    cc(ws.path()).args(["family", "add"]).arg(pack.path()).assert().success();
    let clone = tempfile::tempdir().unwrap();
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/typescript"), clone.path());
    // a file whose NAME carries the term mid-token ("counting" holds "ountin"): FTS prefix matching cannot see it
    std::fs::write(clone.path().join("src/counting.ts"), "export const x = 1;\n").unwrap();
    std::fs::write(
        clone.path().join("src/handlers.ts"),
        "export function loginUser() { return 1; }\nexport function logoutUser() { return 2; }\nexport function resetUser() { return 3; }\n",
    )
    .unwrap();
    std::fs::write(clone.path().join("LICENSE"), "MIT License\n\nCopyright (c) 2026 S\n").unwrap();
    git_commit(clone.path());
    add_repo(ws.path(), clone.path(), Some("ts"), false).unwrap();
    build_repo(ws.path(), "ts", "reference", &BuildOptions::default()).unwrap();
    (ws, clone, pack)
}
fn check(ws: &Path, feature: &str) -> serde_json::Value {
    json(cc(ws).args(["absent-check", "ts", "fam", feature]).assert().success())
}
fn claim_none(ws: &Path, feature: &str, extra: &[&str]) -> assert_cmd::assert::Assert {
    let mut c = cc(ws);
    c.args(["claim", "ts", feature, "none", "--author", "me"]).args(extra);
    c.assert()
}

#[test]
fn a_feature_nothing_matches_is_absent_and_the_claim_is_derived() {
    let (ws, _c, _p) = setup();
    let v = check(ws.path(), "quantum.teleport");
    assert_eq!(v["data"]["absent"], true, "{v}");
    assert_eq!(v["evidence"]["confidence"], "derived", "{v}");
    for ch in ["symbols", "paths", "communities", "kinds"] {
        assert_eq!(v["data"]["channels"][ch]["count"], 0, "{ch}: {v}");
    }
    let c = json(claim_none(ws.path(), "quantum.teleport", &[]).success());
    assert_eq!(c["data"]["verdict"], "none", "{c}");
    assert_eq!(c["evidence"]["confidence"], "derived", "{c}");
    let note = c["data"]["note"].as_str().unwrap();
    assert!(note.starts_with("graph-absent: terms="), "{note}");
    assert!(note.contains("channels=symbols:0,paths:0,communities:0,kinds:0"), "{note}");
}

#[test]
fn a_symbol_hit_refuses_the_claim_and_lists_the_hits() {
    let (ws, _c, _p) = setup();
    let v = check(ws.path(), "math.counter");
    assert_eq!(v["data"]["absent"], false, "{v}");
    assert_eq!(v["evidence"]["confidence"], "observed", "{v}");
    assert!(v["data"]["channels"]["symbols"]["count"].as_i64().unwrap() > 0, "{v}");
    assert!(!v["data"]["channels"]["symbols"]["hits"].as_array().unwrap().is_empty());
    let r = json(claim_none(ws.path(), "math.counter", &[]).failure());
    assert_eq!(r["error"]["code"], "invalid_arguments", "{r}");
    let m = r["error"]["message"].as_str().unwrap();
    assert!(m.contains("the graph has") && m.contains("--searched"), "{m}");
    let none = json(cc(ws.path()).args(["claims"]).assert().success());
    assert_eq!(none["data"]["items"].as_array().unwrap().len(), 0, "nothing written");
}

#[test]
fn a_path_only_hit_refuses() {
    let (ws, _c, _p) = setup();
    let v = check(ws.path(), "naming.ountin");
    assert_eq!(v["data"]["channels"]["symbols"]["count"], 0, "{v}");
    assert!(v["data"]["channels"]["paths"]["count"].as_i64().unwrap() > 0, "{v}");
    assert_eq!(v["data"]["absent"], false, "{v}");
    claim_none(ws.path(), "naming.ountin", &[]).failure();
}

#[test]
fn a_kind_only_hit_refuses() {
    let (ws, _c, _p) = setup();
    let v = check(ws.path(), "quality.tests");
    assert_eq!(v["data"]["channels"]["symbols"]["count"], 0, "{v}");
    assert!(v["data"]["channels"]["kinds"]["count"].as_i64().unwrap() > 0, "{v}");
    assert_eq!(v["data"]["absent"], false, "{v}");
    claim_none(ws.path(), "quality.tests", &[]).failure();
}

#[test]
fn a_community_keyword_alone_refuses() {
    // `user` is a keyword of the community (loginUser, logoutUser, resetUser) but a mid-token of every symbol, and no
    // path carries it: only the communities channel sees it.
    let (ws, _c, _p) = setup();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(
        pack.path().join("rows.md"),
        "---\nfamily: fam2\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n| acct.user | acct | User | x | x | basics | 1 |\n",
    )
    .unwrap();
    cc(ws.path()).args(["family", "add"]).arg(pack.path()).assert().success();
    let v = json(cc(ws.path()).args(["absent-check", "ts", "fam2", "acct.user"]).assert().success());
    assert_eq!(v["data"]["channels"]["symbols"]["count"], 0, "{v}");
    assert_eq!(v["data"]["channels"]["paths"]["count"], 0, "{v}");
    assert!(v["data"]["channels"]["communities"]["count"].as_i64().unwrap() > 0, "{v}");
    assert_eq!(v["data"]["absent"], false, "{v}");
    let r = json(
        cc(ws.path()).args(["claim", "ts", "acct.user", "none", "--author", "me"]).assert().failure(),
    );
    assert_eq!(r["error"]["code"], "invalid_arguments", "{r}");
}

#[test]
fn searched_still_records_the_manual_absence() {
    let (ws, _c, _p) = setup();
    let c = json(claim_none(ws.path(), "math.counter", &["--searched", "zzzunknown qqqmissing"]).success());
    assert!(c["data"]["note"].as_str().unwrap().starts_with("searched: zzzunknown qqqmissing → 0 hits"), "{c}");
}

#[test]
fn two_checks_give_the_same_json() {
    let (ws, _c, _p) = setup();
    for f in ["quantum.teleport", "math.counter", "quality.tests"] {
        let a = cc(ws.path()).args(["absent-check", "ts", "fam", f]).assert().success().get_output().stdout.clone();
        let b = cc(ws.path()).args(["absent-check", "ts", "fam", f]).assert().success().get_output().stdout.clone();
        let (mut a, mut b): (serde_json::Value, serde_json::Value) =
            (serde_json::from_slice(&a).unwrap(), serde_json::from_slice(&b).unwrap());
        a["cost"] = serde_json::Value::Null;
        b["cost"] = serde_json::Value::Null;
        assert_eq!(a, b);
    }
}

#[test]
fn mcp_exposes_the_check_and_cannot_record_a_graph_absence() {
    let (ws, _c, _p) = setup();
    let req = |id: u32, m: &str, p: serde_json::Value| {
        serde_json::json!({"jsonrpc":"2.0","id":id,"method":m,"params":p}).to_string()
    };
    let input = [
        req(1, "tools/list", serde_json::json!({})),
        req(2, "tools/call", serde_json::json!({"name":"comparegraph_absent_check","arguments":{"repo":"ts","family":"fam","feature":"quantum.teleport"}})),
        req(3, "tools/call", serde_json::json!({"name":"comparegraph_claim","arguments":{"repo":"ts","feature":"quantum.teleport","verdict":"none","author":"me"}})),
    ]
    .join("\n");
    let out = cc(ws.path()).arg("mcp").write_stdin(input).assert().success().get_output().stdout.clone();
    let lines: Vec<serde_json::Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let names: Vec<&str> = lines[0]["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"comparegraph_absent_check"), "{names:?}");
    let env: serde_json::Value = serde_json::from_str(lines[1]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(env["data"]["absent"], true, "{env}");
    // the claim tool cannot record a graph-backed absence: `none` over MCP needs --searched terms
    assert_eq!(lines[2]["result"]["isError"], true, "{}", lines[2]);
}
