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
    let npm: Value = serde_json::from_str(&repo("npm/package.json")).unwrap();
    assert_eq!(npm["version"], v, "npm/package.json");
    let optional = npm["optionalDependencies"].as_object().unwrap();
    assert_eq!(optional.len(), 5, "npm/package.json optionalDependencies");
    for (name, dep) in optional {
        assert_eq!(dep, v, "npm/package.json optionalDependencies {name}");
    }
    let changelog = repo("CHANGELOG.md");
    assert!(changelog.contains(&format!("\n## [{v}] - ")), "CHANGELOG.md has no dated section for {v}");
    assert!(changelog.contains(&format!("[{v}]: https://github.com/MassRelayAI/comparegraph/releases/tag/v{v}")), "CHANGELOG.md link for {v}");
}

#[test]
fn version_flag_names_the_commit_it_was_built_from() {
    // loop 3 (E2): `--version` printed 0.1.3 for every commit
    let out = assert_cmd::Command::cargo_bin("comparegraph").unwrap().arg("--version").output().unwrap();
    let s = String::from_utf8(out.stdout).unwrap();
    let v = s.trim().strip_prefix("comparegraph ").unwrap_or(&s);
    let (base, sha) = v.split_once('+').unwrap_or_else(|| panic!("no +<sha> in `{v}`"));
    assert_eq!(base, env!("CARGO_PKG_VERSION"));
    assert!(sha == "unknown" || (sha.len() >= 7 && sha.bytes().all(|b| b.is_ascii_hexdigit())), "{v}");
}
