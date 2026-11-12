//! CMP-UI-1e: the network layer (`net` in ui-evidence/1, section 13) and verify across runs (`--root-a/--root-b`).
//! Roots are built from the probe's own contract fixture (`fixtures/ui-contract`) and from the golden fixture (`fixtures/ui`) in temp dirs.
use assert_cmd::Command;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn fx(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(name)
}
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let (p, q) = (e.path(), to.join(e.file_name()));
        if e.file_type().unwrap().is_dir() {
            copy_dir(&p, &q);
        } else {
            std::fs::copy(&p, &q).unwrap();
        }
    }
}
fn cg(dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = Command::cargo_bin("comparegraph").unwrap().current_dir(dir).arg("--pretty").arg("ui").args(args).output().unwrap();
    let s = String::from_utf8(out.stdout).unwrap();
    (out.status.code().unwrap(), serde_json::from_str(&s).unwrap_or_else(|e| panic!("not json ({e}): {s}")))
}
fn read(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}
fn write(p: &Path, v: &Value) {
    std::fs::write(p, serde_json::to_string_pretty(v).unwrap()).unwrap();
}
/// A copy of the contract fixture that holds ONLY the `cluttered` run labelled `label` (what a probe `--out <dir>/<label>` gives).
fn one_run_root(label: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    copy_dir(&fx("ui-contract"), d.path());
    let runs = d.path().join("ui/runs");
    for e in std::fs::read_dir(&runs).unwrap().flatten() {
        let v = read(&e.path());
        if !(v["inputs"]["product"] == "cluttered" && v["inputs"]["runLabel"] == label) {
            std::fs::remove_file(e.path()).unwrap();
        }
    }
    d
}
fn edit_evidence(root: &Path, rel: &str, f: impl Fn(&mut Value)) {
    let p = root.join("ui").join(rel);
    let mut v = read(&p);
    f(&mut v);
    write(&p, &v);
}
const ROOM: &str = "cluttered/room/populated/1440x900-member.json";
fn net(failed: Value, load_ms: u64, bytes: u64) -> Value {
    json!({ "requests": 4, "bytes": bytes, "byType": {"doc": 1, "xhr": 3}, "failed": failed, "slowest": {"path": "/api/x", "ms": load_ms}, "thirdPartyHosts": 0,
            "timing": {"domContentLoadedMs": 10, "loadMs": load_ms, "firstContentfulPaintMs": 20, "contentStartVisibleMs": 30} })
}
fn verify_roots(a: &Path, b: &Path) -> (i32, Value) {
    cg(a, &["verify", "--root-a", a.to_str().unwrap(), "--root-b", b.to_str().unwrap()])
}

#[test]
fn identical_roots_pass_and_one_differing_metric_fails() {
    let (a, b) = (one_run_root("a"), one_run_root("b"));
    let (c, v) = verify_roots(a.path(), b.path());
    assert_eq!(c, 0, "{v}");
    assert_eq!((v["data"]["identical"].clone(), v["data"]["compared"].as_u64().unwrap() >= 3), (json!(true), true));
    edit_evidence(b.path(), ROOM, |e| e["metrics"]["E1"]["value"] = json!(7));
    let (c, v) = verify_roots(a.path(), b.path());
    assert_eq!(c, 1, "{v}");
    assert_eq!(v["error"]["code"], "check_failed");
    let d = v["data"]["differences"].as_array().unwrap();
    assert_eq!(d.len(), 1, "{v}");
    assert_eq!((d[0]["kind"].clone(), d[0]["metric"].clone()), (json!("metric"), json!("E1")));
}

#[test]
fn net_failed_is_gated_but_timings_and_bytes_are_not() {
    let (a, b) = (one_run_root("a"), one_run_root("b"));
    edit_evidence(a.path(), ROOM, |e| e["net"] = net(json!([{"status": 500, "path": "/api/boom"}]), 100, 5000));
    edit_evidence(b.path(), ROOM, |e| e["net"] = net(json!([{"status": 500, "path": "/api/boom"}]), 9000, 123));
    let (c, v) = verify_roots(a.path(), b.path());
    assert_eq!(c, 0, "timings and bytes differ but failed is equal: {v}");
    edit_evidence(b.path(), ROOM, |e| e["net"] = net(json!([]), 100, 5000));
    let (c, v) = verify_roots(a.path(), b.path());
    assert_eq!(c, 1, "{v}");
    let d = &v["data"]["differences"][0];
    assert_eq!((d["kind"].clone(), d["a"].clone(), d["b"].clone()), (json!("netFailed"), json!([{"status": 500, "path": "/api/boom"}]), json!([])));
    // net on one side only: not compared, said so
    edit_evidence(b.path(), ROOM, |e| { e.as_object_mut().unwrap().remove("net"); });
    let (c, v) = verify_roots(a.path(), b.path());
    assert_eq!(c, 0, "{v}");
    assert!(v["evidence"]["limitations"].to_string().contains("net_missing") || v.to_string().contains("net_missing"), "{v}");
}

#[test]
fn roots_are_matched_without_the_version() {
    let (a, b) = (one_run_root("a"), one_run_root("b"));
    // B is a later version of the same product: the keys in its manifest and files change, the files stay at the same paths
    let runs = b.path().join("ui/runs");
    let run_file = std::fs::read_dir(&runs).unwrap().flatten().next().unwrap().path();
    let mut run = read(&run_file);
    let caps: Vec<(String, String)> = run["captures"].as_object().unwrap().iter().map(|(k, f)| (k.clone(), f.as_str().unwrap().to_string())).collect();
    run["captures"] = json!({});
    for (k, f) in caps {
        let nk = k.replace("contract-1", "contract-2");
        run["captures"][nk.clone()] = json!(f);
        edit_evidence(b.path(), &f, |e| { e["version"] = json!("contract-2"); e["key"] = json!(nk.clone()); });
    }
    write(&run_file, &run);
    let (c, v) = verify_roots(a.path(), b.path());
    assert_eq!(c, 0, "{v}");
    assert_eq!(v["data"]["identical"], true);
}

#[test]
fn a_root_with_two_runs_the_same_folder_and_mixed_forms_are_refused() {
    let (a, b) = (one_run_root("a"), one_run_root("b"));
    let full = tempfile::tempdir().unwrap();
    copy_dir(&fx("ui-contract"), full.path());
    let (c, v) = verify_roots(full.path(), b.path());
    assert_eq!(c, 1, "{v}");
    assert_eq!(v["error"]["code"], "invalid_arguments");
    assert!(v["error"]["message"].as_str().unwrap().contains("holds 4 runs"), "{v}");
    let (c, v) = verify_roots(a.path(), a.path());
    assert_eq!(v["error"]["code"], "invalid_arguments", "{v}");
    assert!(c != 0 && v["error"]["message"].as_str().unwrap().contains("same folder"), "{v}");
    let out = Command::cargo_bin("comparegraph").unwrap().current_dir(a.path()).args(["ui", "verify", "r1", "r2", "--root-a", a.path().to_str().unwrap(), "--root-b", b.path().to_str().unwrap()]).output().unwrap();
    assert!(!out.status.success(), "run ids mixed with roots must be refused (clap)");
    let (c, v) = cg(a.path(), &["verify", "r1"]);
    assert!(c != 0 && v["error"]["code"] == "invalid_arguments", "{v}");
    let empty = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(empty.path().join("ui/runs")).unwrap();
    let (c, v) = verify_roots(empty.path(), b.path());
    assert!(c != 0 && v["error"]["code"] == "not_found", "{v}");
}

#[test]
fn the_old_two_run_id_form_refuses_runs_that_share_an_evidence_file() {
    let root = fx("ui-contract");
    let ids: Vec<String> = std::fs::read_dir(root.join("ui/runs")).unwrap().flatten().filter_map(|e| {
        let v = read(&e.path());
        (v["inputs"]["product"] == "cluttered").then(|| v["runId"].as_str().unwrap().to_string())
    }).collect();
    assert_eq!(ids.len(), 2);
    let (c, v) = cg(&root, &["verify", &ids[0], &ids[1], "--ui-root", "."]);
    assert_eq!(c, 1, "{v}");
    assert_eq!(v["error"]["code"], "invalid_arguments");
    let m = v["error"]["message"].as_str().unwrap();
    assert!(m.contains("same evidence file") && m.contains("--root-a"), "{m}");
}

/// A copy of the golden fixture whose cluttered/room capture has a `net` with one failed call.
fn net_fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    copy_dir(&fx("ui"), d.path());
    edit_evidence(d.path(), "cluttered/room/populated/1440x900-member.json", |e| e["net"] = net(json!([{"status": 500, "path": "/api/relay/rankings"}]), 1200, 20480));
    edit_evidence(d.path(), "clean/room/populated/1440x900-ref.json", |e| e["net"] = net(json!([]), 300, 2048));
    d
}

#[test]
fn show_and_diff_print_a_net_line_and_matrix_net_lists_failing_screens() {
    let d = net_fixture();
    let (_, s) = cg(d.path(), &["show", "cluttered", "room", "--ui-root", "."]);
    let line = s["data"]["netLine"].as_str().unwrap();
    assert!(line.contains("4 requests, 20 KB") && line.contains("1 failed (500 /api/relay/rankings)") && line.contains("load 1200 ms"), "{line}");
    assert_eq!(s["data"]["net"]["failed"][0]["path"], "/api/relay/rankings");
    let (_, df) = cg(d.path(), &["diff", "room", "--ours", "cluttered", "--ref", "clean", "--ui-root", "."]);
    let l = df["data"]["netLine"].as_str().unwrap();
    assert!(l.starts_with("ours: 4 requests") && l.contains("| ref: 4 requests") && l.contains("0 failed"), "{l}");
    let (_, plain) = cg(d.path(), &["matrix", "surfaces", "--ui-root", "."]);
    assert!(plain["data"].get("failedCalls").is_none());
    let (c, m) = cg(d.path(), &["matrix", "surfaces", "--net", "--ui-root", "."]);
    assert_eq!(c, 0, "{m}");
    let f = m["data"]["failedCalls"].as_array().unwrap();
    assert_eq!(f.len(), 1, "{m}");
    assert_eq!((f[0]["product"].clone(), f[0]["screen"].clone(), f[0]["failed"][0]["status"].clone()), (json!("cluttered"), json!("room"), json!(500)));
}

#[test]
fn a_net_with_a_query_string_in_a_path_or_an_unknown_field_is_invalid_evidence() {
    let d = net_fixture();
    edit_evidence(d.path(), "cluttered/room/populated/1440x900-member.json", |e| e["net"]["failed"][0]["path"] = json!("/api/x?token=secret"));
    let (_, v) = cg(d.path(), &["products", "--ui-root", "."]);
    assert!(v.to_string().contains("invalid_evidence") && v.to_string().contains("query"), "{v}");
    edit_evidence(d.path(), "cluttered/room/populated/1440x900-member.json", |e| { e["net"]["failed"][0]["path"] = json!("/api/x"); e["net"]["headers"] = json!({}); });
    let (_, v) = cg(d.path(), &["products", "--ui-root", "."]);
    assert!(v.to_string().contains("invalid_evidence") && v.to_string().contains("headers"), "{v}");
}

#[test]
fn the_probes_contract_fixture_carries_net_and_show_prints_it() {
    let (c, s) = cg(&fx("ui-contract"), &["show", "cluttered", "room", "--ui-root", "."]);
    assert_eq!(c, 0, "{s}");
    assert!(s["data"]["netLine"].as_str().unwrap().contains("1 requests"), "{s}");
    assert_eq!(s["data"]["net"]["failed"], json!([]));
    assert!(!s.to_string().contains("invalid_evidence"), "{s}");
}
