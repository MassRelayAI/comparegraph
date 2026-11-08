//! `map` ranks by graph position, not name match alone (item 1 of docs/reports/2026-11-07-twelve-harness-field-run.md).
//! The fixture is the field-run case: `loop` on openai-agents returned `_get_running_loop`, a helper private caches call,
//! above the loop an entry point runs. Built end to end: add, build (reference provider), family add, map.
use comparegraph::application::{add::add_repo, build::build_repo};
use comparegraph::providers::BuildOptions;
use serde_json::Value;
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

const PACK: &str = "---\nfamily: fam\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n\
| runtime.loop | runtime | Agent loop (pair row) | The provider-specific turn loop [kinds: function] | a loop | basics | 2 |\n";

const MAIN: &str = "from app.runner import run_loop\n\n\ndef main():\n    run_loop()\n";
const RUNNER: &str = "def run_loop():\n    return 1\n";
const CACHE: &str = "def _get_running_loop():\n    return None\n\n\n\
def _cache_a():\n    return _get_running_loop()\n\n\n\
def _cache_b():\n    return _get_running_loop()\n\n\n\
def _cache_c():\n    return _get_running_loop()\n\n\n\
def pair_row_specific_cache():\n    return 0\n";

fn mapped() -> Value {
    let ws = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), PACK).unwrap();
    cc(ws.path())
        .args(["family", "add"])
        .arg(pack.path())
        .assert()
        .success();
    let clone = clone_with(&[
        ("app/main.py", MAIN),
        ("app/runner.py", RUNNER),
        ("app/ws_cache.py", CACHE),
    ]);
    add_repo(ws.path(), clone.path(), Some("agents"), false).unwrap();
    build_repo(ws.path(), "agents", "reference", &BuildOptions::default()).unwrap();
    let out = cc(ws.path())
        .args(["map", "agents", "fam", "--feature", "runtime.loop"])
        .assert()
        .success();
    serde_json::from_slice(&out.get_output().stdout).unwrap()
}

#[test]
fn a_reached_loop_outranks_a_lexical_match_and_says_why() {
    let v = mapped();
    let d = &v["data"];
    assert!(d["entry_points"].as_u64().unwrap() >= 1, "{d}");
    let f = &d["features"][0];
    let syms = f["symbols"].as_array().unwrap();
    let rank: Vec<&str> = syms
        .iter()
        .map(|s| s["qualified_name"].as_str().unwrap())
        .collect();
    let at = |suffix: &str| {
        rank.iter()
            .position(|q| q.ends_with(suffix))
            .unwrap_or_else(|| panic!("{suffix} not in {rank:?}"))
    };
    let (reached, lexical) = (at("::run_loop"), at("::_get_running_loop"));
    assert!(
        reached < lexical,
        "the structurally reached symbol must rank first: {rank:?}"
    );
    assert_eq!(syms[reached]["reach"], "reached", "{}", syms[reached]);
    assert_eq!(syms[reached]["entry_hops"], 1);
    assert_eq!(syms[lexical]["reach"], "unreached", "{}", syms[lexical]);
    assert!(syms[lexical]["entry_hops"].is_null());
    for s in syms {
        assert!(s["community_peers"].is_u64(), "{s}");
    }
    // pack bookkeeping is not a term, though a symbol here carries those words
    let terms: Vec<&str> = f["terms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    for w in ["pair", "row", "specific"] {
        assert!(!terms.contains(&w), "{w} in {terms:?}");
    }
    assert!(terms.contains(&"loop"), "{terms:?}");
}

#[test]
fn the_ranking_is_deterministic() {
    // the envelope may carry timings; the ranked features must be byte-identical
    let features = |v: Value| v["data"]["features"].to_string();
    let a = features(mapped());
    assert!(a.contains("run_loop"), "{a}");
    assert_eq!(a, features(mapped()));
}
