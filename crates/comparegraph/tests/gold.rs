//! Floor test on the committed gold labels (`fixtures/gold/<project>/calls.jsonl`).
//! Runs when `COMPAREGRAPH_STUDY` points at a directory holding read-only copies of the five projects; skipped otherwise.
//! Scoring is `scripts/gold-score.sh` (one scorer, shared with compare-engines.sh); floors are `fixtures/gold/FLOORS.json`.
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

const PROJECTS: [&str; 5] = ["graphmind", "srclight", "code-review-graph", "hono", "caddy"];

#[test]
fn gold_floors_hold() {
    let study = match std::env::var_os("COMPAREGRAPH_STUDY") {
        Some(s) if !s.is_empty() => PathBuf::from(s),
        _ => {
            eprintln!("gold floors skipped: COMPAREGRAPH_STUDY is not set (a dir holding the five project copies)");
            return;
        }
    };
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let floors: Value =
        serde_json::from_str(&std::fs::read_to_string(repo.join("fixtures/gold/FLOORS.json")).unwrap()).unwrap();
    let (mut answered, mut correct, mut repo_sites) = (0u64, 0u64, 0u64);
    let mut failures = Vec::new();
    for p in PROJECTS {
        let out = Command::new("bash")
            .arg(repo.join("scripts/gold-score.sh"))
            .arg(study.join(p))
            .arg(repo.join(format!("fixtures/gold/{p}/calls.jsonl")))
            .args(["--binary", env!("CARGO_BIN_EXE_comparegraph")])
            .output()
            .unwrap();
        assert!(out.status.success(), "{p}: {}", String::from_utf8_lossy(&out.stderr));
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        let (pr, re) = (v["precision"].as_f64().unwrap(), v["recall"].as_f64().unwrap());
        eprintln!("{p}: precision {pr} recall {re}");
        answered += v["answered"].as_u64().unwrap();
        correct += v["correct"].as_u64().unwrap();
        repo_sites += v["repo_target_sites"].as_u64().unwrap();
        let f = &floors[p];
        if pr < f["precision"].as_f64().unwrap() {
            failures.push(format!("{p} precision {pr} < floor {}", f["precision"]));
        }
        if re < f["recall"].as_f64().unwrap() {
            failures.push(format!("{p} recall {re} < floor {}", f["recall"]));
        }
    }
    let (pp, pr) = (correct as f64 / answered as f64, correct as f64 / repo_sites as f64);
    eprintln!("pooled: precision {pp:.4} recall {pr:.4}");
    let f = &floors["pooled"];
    if pp < f["precision"].as_f64().unwrap() {
        failures.push(format!("pooled precision {pp:.4} < floor {}", f["precision"]));
    }
    if pr < f["recall"].as_f64().unwrap() {
        failures.push(format!("pooled recall {pr:.4} < floor {}", f["recall"]));
    }
    assert!(failures.is_empty(), "gold floors broken: {failures:?}");
}
