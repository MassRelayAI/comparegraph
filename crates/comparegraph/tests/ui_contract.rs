//! The probe/CompareGraph contract (CMP-UI-1-lanes.md section 10): `fixtures/ui-contract/` is the output of the UI probe (`node tools/ui/contract.mjs`)
//! on its own fixture pages, copied byte-for-byte. Every `ui` verb must read it with ZERO `invalid_evidence` limitations. No staging data in it.
use assert_cmd::Command;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/ui-contract")
}
fn run_in(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::cargo_bin("comparegraph").unwrap().current_dir(dir).arg("--pretty").arg("ui").args(args).arg("--ui-root").arg(".").output().unwrap();
    (out.status.code().unwrap(), String::from_utf8(out.stdout).unwrap())
}
fn ui(args: &[&str]) -> (i32, String, Value) {
    let (c, s) = run_in(&root(), args);
    let v = serde_json::from_str(&s).unwrap_or_else(|e| panic!("not json ({e}): {s}"));
    (c, s, v)
}
/// Exit 0 and no `invalid_evidence` anywhere in the answer.
fn clean(args: &[&str]) -> Value {
    let (c, s, v) = ui(args);
    assert_eq!(c, 0, "{args:?} exited {c}: {s}");
    assert!(!s.contains("invalid_evidence"), "{args:?} reported invalid evidence: {s}");
    v
}
/// A temp copy of a contract root holding only the run of `product`.
fn product_root(src: &Path, product: &str) -> tempfile::TempDir {
    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let (p, q) = (e.path(), to.join(e.file_name()));
            if e.file_type().unwrap().is_dir() { copy_dir(&p, &q) } else { std::fs::copy(&p, &q).unwrap(); }
        }
    }
    let d = tempfile::tempdir().unwrap();
    copy_dir(src, d.path());
    for e in std::fs::read_dir(d.path().join("ui/runs")).unwrap().flatten() {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(e.path()).unwrap()).unwrap();
        if v["inputs"]["product"] != product {
            std::fs::remove_file(e.path()).unwrap();
        }
    }
    d
}
fn run_id(product: &str, label: &str) -> String {
    // evidence is append-only: run `a` is the contract root, run `b` the second root fixtures/ui-contract-b
    let dir = if label == "b" { root().join("../ui-contract-b/ui/runs") } else { root().join("ui/runs") };
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(e.path()).unwrap()).unwrap();
        if v["inputs"]["product"] == product && v["inputs"]["runLabel"] == label {
            return v["runId"].as_str().unwrap().to_string();
        }
    }
    panic!("no run {product}/{label}");
}

#[test]
fn every_ui_verb_reads_the_probes_own_output_without_invalid_evidence() {
    let p = clean(&["products"]);
    let items = p["data"]["items"].as_array().unwrap();
    let names: Vec<&str> = items.iter().map(|i| i["product"].as_str().unwrap()).collect();
    assert_eq!(names, ["clean", "cluttered"]);
    assert_eq!((items[0]["evidenceFiles"].as_u64(), items[1]["evidenceFiles"].as_u64()), (Some(2), Some(4)));
    assert_eq!((items[0]["flows"].as_u64(), items[0]["navs"].as_u64(), items[1]["navs"].as_u64()), (Some(1), Some(1), Some(1)));
    assert_eq!((items[0]["system"].clone(), items[1]["system"].clone()), (Value::Bool(true), Value::Bool(true)));

    let s = clean(&["screens", "cluttered"]);
    let screens: Vec<&str> = s["data"]["items"].as_array().unwrap().iter().map(|i| i["screen"].as_str().unwrap()).collect();
    assert_eq!(screens, ["inbox", "room"]);
    assert_eq!(s["data"]["items"][1]["archetype"], "conversation");

    let show = clean(&["show", "cluttered", "room"]);
    assert_eq!(show["data"]["viewport"], "1440x900");
    assert_eq!(show["data"]["metrics"][0]["id"], "E1");
    assert_eq!(show["data"]["metrics"][0]["pass"], false, "the cluttered page must come out red on E1");
    assert_eq!(show["data"]["image"]["present"], true);
    let green = clean(&["show", "clean", "room"]);
    assert_eq!(green["data"]["pass"], 12, "the clean page passes every metric");

    clean(&["show", "cluttered", "room", "--viewport", "390x844"]);
    clean(&["show", "cluttered", "room", "--state", "empty"]);
    let d = clean(&["diff", "room", "--ours", "cluttered", "--ref", "clean"]);
    assert!(!d["data"]["layoutDiff"].as_array().unwrap().is_empty(), "{d}");
    clean(&["system", "cluttered"]);
    clean(&["system", "clean"]);
    clean(&["flow", "clean", "open-it"]);
    clean(&["nav", "cluttered"]);
    clean(&["nav", "clean"]);
    clean(&["nav-diff", "--ours", "cluttered", "--ref", "clean"]);
    let j = clean(&["journey", "j1"]);
    assert_eq!(j["data"]["items"].as_array().unwrap().len(), 2);

    // CMP-UI-1g: the interaction graphs the probe crawled (read-only) on its own /g/ fixture site
    let g = clean(&["graph", "clean"]);
    let s = &g["data"]["summary"];
    assert_eq!((s["nodes"].as_u64(), s["orphans"].as_u64(), s["deadActions"].as_u64(), s["mutatingNotClicked"].as_u64(), s["cycles"].as_u64()), (Some(8), Some(1), Some(3), Some(3), Some(1)), "{g}");
    assert_eq!(g["data"]["orphans"], serde_json::json!(["flows-list"]), "the registry has flows-list (/g/orphan); no click reaches it");
    assert_eq!(g["data"]["start"], "room");
    assert_eq!(g["data"]["depth"]["unregistered:/g/hidden"], 2, "reachable only through the Details dialog");
    let p = clean(&["path", "clean", "room", "unregistered:/g/hidden"]);
    assert_eq!(p["data"]["clicks"], 2);
    assert_eq!(p["data"]["steps"][0]["effect"], "open-dialog");
    let gc = clean(&["graph", "cluttered"]);
    assert_eq!(gc["data"]["truncated"]["depth"], true, "the cluttered graph was crawled to depth 1 and says so");
    assert!(gc["evidence"]["limitations"].to_string().contains("graph_truncated"), "{gc}");
    let gd = clean(&["graph-diff", "--ours", "cluttered", "--ref", "clean"]);
    assert!(gd["data"]["depth"].as_array().unwrap().iter().any(|r| r["screen"] == "inbox"));
    assert!(!gd["data"]["diff"].as_array().unwrap().is_empty(), "{gd}");
    let gt = clean(&["graph-diff", "--ours", "clean", "--ref", "target", "--tree", "app"]);
    assert_eq!(gt["data"]["ref"]["product"], "target");

    // verify between the two probe runs of the same fixtures (two products, so check both)
    for product in ["cluttered", "clean"] {
        let (a, b) = (run_id(product, "a"), run_id(product, "b"));
        assert_ne!(a, b);
        // the probe never writes over another run's evidence, so run b lives in its own root; verify them as two roots (each holding only this product's run)
        let (ra, rb) = (product_root(&root(), product), product_root(&root().join("../ui-contract-b"), product));
        let out = Command::cargo_bin("comparegraph").unwrap().current_dir(ra.path()).args(["--pretty", "ui", "verify", "--root-a"]).arg(ra.path()).arg("--root-b").arg(rb.path()).arg("--integrity").output().unwrap();
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(out.status.code(), Some(0), "{product}: the two probe runs must verify identical, integrity checked: {v}");
        assert_eq!(v["data"]["identical"], true);
        assert_eq!(v["data"]["compared"], v["data"]["total"]);
        let cov = clean(&["coverage", &a]);
        assert_eq!(cov["data"]["complete"], true, "every coverage cell is explained: {cov}");
    }
}

#[test]
fn evidence_with_a_field_outside_the_contract_is_still_refused() {
    let d = tempfile::tempdir().unwrap();
    let from = root().join("ui/clean/room/populated/1440x900-ref.json");
    let to = d.path().join("ui/clean/room/populated");
    std::fs::create_dir_all(&to).unwrap();
    let mut ev: Value = serde_json::from_str(&std::fs::read_to_string(&from).unwrap()).unwrap();
    std::fs::write(to.join("1440x900-ref.json"), ev.to_string()).unwrap();
    let (_, ok) = run_in(d.path(), &["products"]);
    assert!(!ok.contains("invalid_evidence"), "{ok}");
    ev["surprise"] = Value::Bool(true);
    std::fs::write(to.join("1440x900-ref.json"), ev.to_string()).unwrap();
    let (_, bad) = run_in(d.path(), &["products"]);
    assert!(bad.contains("invalid_evidence") && bad.contains("surprise"), "{bad}");
}

fn products_with(edit: impl Fn(&mut Value), file: &str) -> String {
    let d = tempfile::tempdir().unwrap();
    let to = d.path().join("ui/clean/room/populated");
    std::fs::create_dir_all(&to).unwrap();
    let mut ev: Value = serde_json::from_str(&std::fs::read_to_string(root().join("ui/clean/room/populated/1440x900-ref.json")).unwrap()).unwrap();
    edit(&mut ev);
    std::fs::write(to.join(file), ev.to_string()).unwrap();
    run_in(d.path(), &["products"]).1
}

#[test]
fn the_joined_key_must_agree_with_the_fields_it_joins() {
    let bad = products_with(|e| e["key"] = Value::String("clean@contract-1/room/populated/1440x900/ref/en-US/dark".into()), "1440x900-ref.json");
    assert!(bad.contains("invalid_evidence") && bad.contains("key"), "{bad}");
}

#[test]
fn the_path_carries_locale_and_theme_unless_both_are_the_defaults() {
    // theme dark in the default-named file: refused; in the suffixed file: read
    let wrong = products_with(|e| { e["theme"] = Value::String("dark".into()); e["key"] = Value::String("clean@contract-1/room/populated/1440x900/ref/en-US/dark".into()); }, "1440x900-ref.json");
    assert!(wrong.contains("invalid_evidence"), "{wrong}");
    let right = products_with(|e| { e["theme"] = Value::String("dark".into()); e["key"] = Value::String("clean@contract-1/room/populated/1440x900/ref/en-US/dark".into()); }, "1440x900-ref-en-US-dark.json");
    assert!(!right.contains("invalid_evidence") && right.contains("evidenceFiles\": 1"), "{right}");
}

#[test]
fn the_design_lanes_real_coverage_files_are_read_with_zero_invalid_entries() {
    // section 11: ui/registry.json (120 screens), archetypes.json, journeys.json (definitions), map.csv (7 columns), fixture.json are the design lane's, unedited
    let m = clean(&["matrix", "budgets"]);
    let items = m["data"]["items"].as_array().unwrap();
    assert!(items.len() >= 3, "the budgets family has mapped rows: {m}");
    assert_eq!(m["data"]["products"], serde_json::json!(["clean", "cluttered"]));
    let show = clean(&["show", "cluttered", "room"]);
    assert_eq!(show["data"]["archetype"], "conversation");
    assert!(show["data"]["metrics"][0]["archetypeLimit"].is_object(), "limits come from archetypes.json: {}", show["data"]["metrics"][0]);
    let j = clean(&["journey", "j1"]);
    assert_eq!(j["data"]["title"], "Open the thing");
    let map = std::fs::read_to_string(root().join("ui/map.csv")).unwrap();
    assert!(map.starts_with("family,row_id,product,screen,route,task,note"));
    assert!(map.contains("massrelay-ops"));
}
