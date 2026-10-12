use assert_cmd::Command;
use serde_json::Value;

fn cg() -> Command {
    Command::cargo_bin("comparegraph").unwrap()
}
fn live() -> String {
    String::from_utf8(cg().arg("catalog").assert().success().get_output().stdout.clone()).unwrap()
}

#[test]
fn catalog_has_17_tools_sorted_with_error_codes() {
    let text = live();
    assert!(text.ends_with("}\n") && text.contains("\n  \"tools\""));
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    let tools = v["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 17);
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    for t in tools {
        assert!(!t["description"].as_str().unwrap().is_empty());
        assert_eq!(t["inputSchema"]["type"], "object");
    }
    let codes: Vec<&str> = v["error_codes"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
    assert!(codes.contains(&"catalog_stale") && codes.contains(&"not_found"));
}

#[test]
fn committed_catalog_matches_live_and_check_goes_red_on_a_tampered_copy() {
    let committed = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/catalog.json");
    assert_eq!(std::fs::read_to_string(committed).unwrap(), live(), "regenerate: comparegraph catalog > docs/catalog.json");
    let ok = cg().args(["catalog", "--check", "--file", committed]).assert().success();
    let v: Value = serde_json::from_slice(&ok.get_output().stdout).unwrap();
    assert_eq!(v["ok"], true);

    let d = tempfile::tempdir().unwrap();
    let bad = d.path().join("catalog.json");
    std::fs::write(&bad, live().replacen("comparegraph_check", "comparegraph_chek", 1)).unwrap();
    let out = cg().args(["catalog", "--check", "--file"]).arg(&bad).assert().code(1);
    let v: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "catalog_stale");
    assert!(v["error"]["hint"].as_str().unwrap().contains("catalog > docs/catalog.json"));
}
