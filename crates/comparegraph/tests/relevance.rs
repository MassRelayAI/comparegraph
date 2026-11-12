//! `relevance` (lane CG-STORE B1): factors always, a class only with the user's policy; deterministic; read-only.
mod common;
use common::*;
use std::io::Write;

const POLICY: &str = r#"{"schema":"relevance-policy/1","version":1,
 "weights":{"use":40,"pull":25,"recency":15,"band":10,"ui":10},
 "saturate":{"evidence_claims":2,"pull_rows":3},
 "recency":[{"max_days":30,"share":1.0},{"max_days":60,"share":0.53}],
 "classes":{"keep_min":45,"remove_max":15,"grace_days":14},
 "primaries":[{"repo":"b","domain":"retry","why":"D405","by":"t","date":"2026-11-12"}],
 "pins":[],
 "pull":{"file":"port.csv","repo_column":"comparator"},
 "aliases":{"file":"aliases.csv","from":"comparator","to":"clone"},
 "ui":{"file":"registry.json"},
 "retention":{"parse_cache_versions":1}}"#;

fn with_policy(w: &Ws) -> String {
    let d = w.root().join("pol");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("policy.json"), POLICY).unwrap();
    std::fs::write(d.join("port.csv"), "tracker,comparator\nT-1,A-Repo\nT-2,\"A-Repo\"\n").unwrap();
    std::fs::write(d.join("aliases.csv"), "comparator,clone\nA-Repo,a\n").unwrap();
    std::fs::write(d.join("registry.json"), r#"{"schema":"ui-registry/1","products":{"a":{"manifest":"proven"}}}"#).unwrap();
    d.join("policy.json").display().to_string()
}

#[test]
fn without_a_policy_every_repo_is_unrated_with_factors() {
    let w = Ws::new();
    let v = ok(w.root(), &["relevance"]);
    let items = v["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    for i in items {
        let want = if i["role"] == "root" { "ROOT" } else { "unrated" };
        assert_eq!(i["class"], want, "{i}");
        assert!(i["factors"]["evidence_claims"].is_number());
        assert!(i["score"].is_null());
    }
    let a = items.iter().find(|i| i["repo"] == "a").unwrap();
    assert_eq!(a["factors"]["evidence_claims"], 2);
    assert!(v["evidence"]["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "unrated"));
    assert_eq!(v["evidence"]["confidence"], "derived");
}

#[test]
fn an_unknown_key_is_invalid_policy() {
    let w = Ws::new();
    let p = w.root().join("bad.json");
    std::fs::write(&p, POLICY.replace("\"version\":1", "\"version\":1,\"wieghts\":{}")).unwrap();
    let e = err(w.root(), &["relevance", "--policy", p.to_str().unwrap()], "invalid_policy");
    assert!(e["error"]["message"].as_str().unwrap().contains("wieghts"));
}

#[test]
fn with_a_policy_classes_follow_the_rules_and_two_runs_are_byte_equal() {
    let w = Ws::new();
    let p = with_policy(&w);
    let one = cc(w.root()).args(["relevance", "--policy", &p]).output().unwrap().stdout;
    let two = cc(w.root()).args(["relevance", "--policy", &p]).output().unwrap().stdout;
    assert_eq!(one, two, "same store + same policy = same bytes");
    let v: serde_json::Value = serde_json::from_slice(&one).unwrap();
    let class = |n: &str| v["data"]["items"].as_array().unwrap().iter().find(|i| i["repo"] == n).unwrap()["class"].as_str().unwrap().to_string();
    assert_eq!(class("rootrepo"), "ROOT");
    assert_eq!(class("b"), "KEEP-PRIMARY");
    // a: 2/2 evidence (40) + 2/3 pull (16.67) + recent (15) + ui proven (10) = 81.67 -> KEEP
    assert_eq!(class("a"), "KEEP");
    let a = ok(w.root(), &["relevance", "--policy", &p, "--explain", "a"]);
    assert_eq!(a["data"]["repo"]["factors"]["pull_rows"], 2, "{a}");
    assert_eq!(a["data"]["repo"]["factors"]["ui"], "proven");
    assert_eq!(a["data"]["repo"]["terms"]["use"], 40.0);
    let inputs = &v["data"]["inputs"];
    assert!(inputs["policy_sha256"].as_str().unwrap().len() == 64);
    assert_eq!(inputs["files"].as_object().unwrap().len(), 3, "{inputs}");
    assert!(inputs["store_digest"].as_str().unwrap().len() == 64);
    // the first rule row is ROOT, then KEEP-PRIMARY, then KEEP
    let order: Vec<&str> = v["data"]["items"].as_array().unwrap().iter().map(|i| i["class"].as_str().unwrap()).collect();
    assert_eq!(order, ["ROOT", "KEEP-PRIMARY", "KEEP"]);
}

#[test]
fn as_of_moves_new_repos_out_of_grace_and_limits_are_bounded() {
    let w = Ws::new();
    let p = with_policy(&w);
    let now = ok(w.root(), &["relevance", "--policy", &p, "--explain", "a"]);
    assert_eq!(now["data"]["repo"]["factors"]["days_since_added"], 0);
    let later = ok(w.root(), &["relevance", "--policy", &p, "--as-of", "2099-01-01", "--explain", "a"]);
    assert!(later["data"]["repo"]["factors"]["days_since_cited"].as_i64().unwrap() > 60);
    err(w.root(), &["relevance", "--limit", "501"], "invalid_arguments");
    let page = ok(w.root(), &["relevance", "--limit", "1"]);
    assert_eq!(page["page"]["total"], 3);
    assert_eq!(page["page"]["next"], "1");
    assert!(page["evidence"]["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "truncated"));
    let fam = ok(w.root(), &["relevance", "--family", "fam"]);
    assert_eq!(fam["data"]["items"].as_array().unwrap().len(), 3);
    err(w.root(), &["relevance", "--family", "nope"], "not_found");
}

#[test]
fn the_mcp_tool_answers_and_leaves_the_store_bytes_equal() {
    let w = Ws::new();
    w.conn().query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())).unwrap();
    let before = w.store_bytes();
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("comparegraph"))
        .arg("--root")
        .arg(w.root())
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        let i = child.stdin.as_mut().unwrap();
        writeln!(i, r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"comparegraph_relevance","arguments":{{"limit":2}}}}}}"#).unwrap();
        writeln!(i, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"comparegraph_governance_verify","arguments":{{}}}}}}"#).unwrap();
        writeln!(i, r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"comparegraph_repos","arguments":{{"no_port":true}}}}}}"#).unwrap();
    }
    drop(child.stdin.take());
    let o = child.wait_with_output().unwrap();
    let envs: Vec<serde_json::Value> = String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .map(|r| serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap())
        .collect();
    assert_eq!(envs.len(), 3);
    assert!(envs.iter().all(|e| e["ok"] == true), "{envs:?}");
    assert_eq!(envs[0]["data"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(envs[2]["data"]["items"][0]["name"], "b");
    assert_eq!(w.store_bytes(), before, "a read tool wrote to the store");
}

#[test]
fn the_shipped_example_policy_parses_and_names_the_d405_primaries() {
    let text = include_str!("../../../docs/examples/relevance-policy.d405.json");
    let p = comparegraph::domain::relevance::Policy::parse(text).unwrap();
    assert!(p.primary("hermes-agent").is_some() && p.primary("activepieces").is_some());
    assert_eq!(p.retention.unwrap().parse_cache_versions, 1);
}
