//! Every file that repeats the crate version agrees with Cargo.toml, so a release tag cannot disagree with a manifest.

use serde_json::Value;

fn repo(path: &str) -> String {
    std::fs::read_to_string(format!("{}/../../{path}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn manifests_repeat_the_crate_version() {
    let v = env!("CARGO_PKG_VERSION");
    let server: Value = serde_json::from_str(&repo("server.json")).unwrap();
    assert_eq!(server["version"], v, "server.json");
    let plugin: Value = serde_json::from_str(&repo(".claude-plugin/plugin.json")).unwrap();
    assert_eq!(plugin["version"], v, ".claude-plugin/plugin.json");
    let market: Value = serde_json::from_str(&repo(".claude-plugin/marketplace.json")).unwrap();
    assert_eq!(market["plugins"][0]["version"], v, ".claude-plugin/marketplace.json");
    let changelog = repo("CHANGELOG.md");
    assert!(changelog.contains(&format!("\n## [{v}] - ")), "CHANGELOG.md has no dated section for {v}");
    assert!(changelog.contains(&format!("[{v}]: https://github.com/massrelay-ai/comparegraph/releases/tag/v{v}")), "CHANGELOG.md link for {v}");
}
