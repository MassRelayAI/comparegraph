//! `map` ranks a re-export shim's target, not the shim (item 10 of docs/reports/2026-11-07-twelve-harness-field-run.md).
//! The fixture is the field-run case: ADK keeps `flows/llm_flows/compaction.py`, a module whose body is only
//! `from ... import ...` and `__all__`, beside the real implementation. Built end to end: add, build (reference
//! provider), family add, map.
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
| context.compaction | context | Compaction | Compacts the event history [kinds: file] | a compactor | basics | 2 |\n";

/// the shim: imports and `__all__`, nothing else (the ADK compatibility module's shape)
const SHIM: &str = "\"\"\"Kept for compatibility; moved to apps.\"\"\"\nfrom ...apps.compaction_impl import summarize_events\n\
from ...apps.compaction_impl import WindowConfig\n\n__all__ = [\"summarize_events\", \"WindowConfig\"]\n";
const REAL: &str = "class WindowConfig:\n    size = 10\n\n\ndef summarize_events(events):\n    return events[:WindowConfig.size]\n";
const MAIN: &str = "from pkg.flows.llm_flows.compaction import summarize_events\n\n\ndef main():\n    summarize_events([])\n";

fn files(shim: &'static str) -> Vec<(&'static str, &'static str)> {
    vec![
        ("pkg/__init__.py", ""),
        ("pkg/flows/__init__.py", ""),
        ("pkg/flows/llm_flows/__init__.py", ""),
        ("pkg/apps/__init__.py", ""),
        ("pkg/flows/llm_flows/compaction.py", shim),
        ("pkg/apps/compaction_impl.py", REAL),
        ("pkg/main.py", MAIN),
    ]
}

fn mapped(files: &[(&str, &str)]) -> Value {
    let ws = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), PACK).unwrap();
    cc(ws.path())
        .args(["family", "add"])
        .arg(pack.path())
        .assert()
        .success();
    let clone = clone_with(files);
    add_repo(ws.path(), clone.path(), Some("adk"), false).unwrap();
    build_repo(ws.path(), "adk", "reference", &BuildOptions::default()).unwrap();
    let out = cc(ws.path())
        .args([
            "map",
            "adk",
            "fam",
            "--feature",
            "context.compaction",
            "--limit",
            "3",
        ])
        .assert()
        .success();
    serde_json::from_slice(&out.get_output().stdout).unwrap()
}

fn symbols(v: &Value) -> Vec<Value> {
    v["data"]["features"][0]["symbols"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn the_target_takes_the_shim_slot_and_the_shim_says_shim_of() {
    let syms = symbols(&mapped(&files(SHIM)));
    let qns: Vec<&str> = syms
        .iter()
        .map(|s| s["qualified_name"].as_str().unwrap())
        .collect();
    let at = |q: &str| {
        qns.iter()
            .position(|x| *x == q)
            .unwrap_or_else(|| panic!("{q} not in {qns:?}"))
    };
    let (target, shim) = (
        at("pkg/apps/compaction_impl.py"),
        at("pkg/flows/llm_flows/compaction.py"),
    );
    // the shim name-matches better (`compaction` against `compaction_impl`), so on its own it ranks above its target;
    // the target takes that slot, at the shim's score, and the shim follows it
    assert_eq!(shim, target + 1, "{qns:?}");
    assert!(syms[target]["shim_of"].is_null(), "{}", syms[target]);
    assert_eq!(syms[target]["score"], syms[shim]["score"], "{qns:?}");
    let shims: Vec<&Value> = syms.iter().filter(|s| !s["shim_of"].is_null()).collect();
    assert_eq!(shims.len(), 1, "the shim is shown once: {qns:?}");
    assert_eq!(
        shims[0]["qualified_name"],
        "pkg/flows/llm_flows/compaction.py"
    );
    assert_eq!(shims[0]["shim_of"], "pkg/apps/compaction_impl.py");
    assert_eq!(
        qns.iter()
            .filter(|q| **q == "pkg/apps/compaction_impl.py")
            .count(),
        1,
        "{qns:?}"
    );
    // the shim takes no slot of --limit 3: three non-shim candidates plus the shim
    assert_eq!(
        syms.iter().filter(|s| s["shim_of"].is_null()).count(),
        3,
        "{qns:?}"
    );
}

#[test]
fn a_module_with_code_of_its_own_is_not_a_shim() {
    // the same imports plus a function: a real module, ranked as before
    let own = "from ...apps.compaction_impl import summarize_events\n\n\ndef compaction_window():\n    return summarize_events([])\n";
    let syms = symbols(&mapped(&files(own)));
    assert!(syms.iter().all(|s| s["shim_of"].is_null()), "{syms:?}");
}

#[test]
fn the_shim_ranking_is_deterministic() {
    let a = mapped(&files(SHIM))["data"]["features"].to_string();
    assert!(
        a.contains("\"shim_of\":\"pkg/apps/compaction_impl.py\""),
        "{a}"
    );
    assert_eq!(a, mapped(&files(SHIM))["data"]["features"].to_string());
}
