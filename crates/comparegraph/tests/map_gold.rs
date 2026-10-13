//! Floor test on the committed mapping gold (`fixtures/gold-map/<clone>/features.jsonl`).
//! Runs the scorer's self-test always; the clone scoring runs when `COMPAREGRAPH_STUDY` points at a directory holding the
//! read-only project copies (skipped loudly otherwise). Scoring is `scripts/map-score.sh`; floors are `fixtures/gold-map/FLOORS.json`:
//! a file with only a `_note` makes the test print the numbers and pass, so the first honest run sets the floors.
//! Floor shape: {"<clone>": {"map_p5","map_r5","absent_precision","absent_recall"}, "pooled": {same}}; a floor key that is missing is not checked.
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn scorer_selftest() {
    let out = Command::new("python3")
        .args(["-I", "scripts/lib/goldmap.py", "--selftest"])
        .current_dir(repo())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

#[test]
fn map_gold_floors_hold() {
    let study = match std::env::var_os("COMPAREGRAPH_STUDY") {
        Some(s) if !s.is_empty() => PathBuf::from(s),
        _ => {
            eprintln!("map gold floors SKIPPED: COMPAREGRAPH_STUDY is not set (a dir holding the project copies)");
            return;
        }
    };
    let repo = repo();
    let base = repo.join("fixtures/gold-map");
    // an unreadable or unparseable floors file must fail the test: a silent null would check nothing
    let floors_path = base.join("FLOORS.json");
    let floors: Value = serde_json::from_str(
        &std::fs::read_to_string(&floors_path).unwrap_or_else(|e| panic!("cannot read {}: {e}", floors_path.display())),
    )
    .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", floors_path.display()));
    let family = std::env::var("COMPAREGRAPH_MAP_FAMILY").unwrap_or_else(|_| "code-graph-tools".into());
    let pack = repo.join(format!("families/{family}/rows.md"));
    let mut clones: Vec<String> = std::fs::read_dir(&base)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("features.jsonl").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    clones.sort();
    if clones.is_empty() {
        eprintln!("map gold floors SKIPPED: no fixtures/gold-map/<clone>/features.jsonl yet");
        return;
    }
    let (mut failures, mut pooled) = (Vec::new(), [0u64; 6]); // cands, cands_ok, golds, golds_hit, called, called_none
    let (mut none_total, mut none_called) = (0u64, 0u64);
    for c in &clones {
        let out = Command::new("bash")
            .arg(repo.join("scripts/map-score.sh"))
            .arg(study.join(c))
            .arg(base.join(c).join("features.jsonl"))
            .args(["--family", &family, "--pack"])
            .arg(&pack)
            .args(["--binary", env!("CARGO_BIN_EXE_comparegraph")])
            .output()
            .unwrap();
        assert!(out.status.success(), "{c}: {}", String::from_utf8_lossy(&out.stderr));
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        eprintln!("{c}: {}", v);
        let g = |p: &str, k: &str| v[p][k].as_u64().unwrap_or(0);
        pooled[0] += g("map", "cands");
        pooled[1] += g("map", "cands_ok");
        pooled[2] += g("map", "golds");
        pooled[3] += g("map", "golds_hit");
        pooled[4] += g("absent", "called");
        pooled[5] += g("absent", "called_none");
        none_total += g("absent", "none");
        none_called += g("absent", "none_called");
        for (key, val) in [
            ("map_p5", &v["map"]["p5"]),
            ("map_r5", &v["map"]["r5"]),
            ("absent_precision", &v["absent"]["precision"]),
            ("absent_recall", &v["absent"]["recall"]),
        ] {
            if let (Some(f), Some(x)) = (floors[c][key].as_f64(), val.as_f64()) {
                if x < f {
                    failures.push(format!("{c} {key} {x} < floor {f}"));
                }
            }
        }
    }
    let ratio = |a: u64, b: u64| if b == 0 { None } else { Some(a as f64 / b as f64) };
    eprintln!(
        "pooled: map_p5 {:?} map_r5 {:?} absent_precision {:?} absent_recall {:?}",
        ratio(pooled[1], pooled[0]),
        ratio(pooled[3], pooled[2]),
        ratio(pooled[5], pooled[4]),
        ratio(none_called, none_total)
    );
    for (key, val) in [
        ("map_p5", ratio(pooled[1], pooled[0])),
        ("map_r5", ratio(pooled[3], pooled[2])),
        ("absent_precision", ratio(pooled[5], pooled[4])),
        ("absent_recall", ratio(none_called, none_total)),
    ] {
        if let (Some(f), Some(x)) = (floors["pooled"][key].as_f64(), val) {
            if x < f {
                failures.push(format!("pooled {key} {x:.4} < floor {f}"));
            }
        }
    }
    assert!(failures.is_empty(), "map gold floors broken: {failures:?}");
}
