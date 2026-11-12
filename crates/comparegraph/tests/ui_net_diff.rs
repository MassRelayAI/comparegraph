//! REA-CG: `ui net-diff` reads the probe's `net.behaviour` (net-behaviour/1), `net.shapes` and `net.completeness` (REA-A) and reports requests
//! added / removed / changed; `ui verify` never calls a truncated capture identical. Records here are in exactly the probe's format.
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
fn root() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    copy_dir(&fx("ui-contract"), d.path());
    d
}
fn cg(dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = Command::cargo_bin("comparegraph").unwrap().current_dir(dir).arg("--pretty").arg("ui").args(args).output().unwrap();
    let s = String::from_utf8(out.stdout).unwrap();
    (out.status.code().unwrap(), serde_json::from_str(&s).unwrap_or_else(|e| panic!("not json ({e}): {s}")))
}
fn edit(root: &Path, rel: &str, f: impl Fn(&mut Value)) {
    let p = root.join("ui").join(rel);
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    f(&mut v);
    std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap()).unwrap();
}
const OURS: &str = "cluttered/room/populated/1440x900-member.json";
const REF: &str = "clean/room/populated/1440x900-ref.json";

fn sha(n: u32) -> String {
    format!("{n:064x}")
}
/// One transaction as the probe writes it: [request, outcome], a fingerprint, a null parent.
fn tx(n: u32, method: &str, path: &str, status: Option<u16>) -> Value {
    let outcome = status.map_or(json!({"kind": "pending"}), |s| json!({"kind": "response", "status": s}));
    json!({ "events": [{"kind": "request", "method": method, "path": path, "type": "xhr", "party": "first"}, outcome], "fingerprint": sha(n), "parent": null })
}
fn behaviour(txs: Vec<Value>) -> Value {
    json!({ "schema": "net-behaviour/1", "normalization": {"rules": [], "sha256": sha(900)}, "transactions": txs, "coverage": {"status": "complete", "reasons": []}, "sha256": sha(901) })
}
fn complete() -> Value {
    json!({ "status": "complete_within_window", "conditions": ["complete_within_window"], "policy_filtered_sections": [], "attach_limited_sections": [],
            "truncated_sections": [], "unavailable_sections": [], "excluded": [],
            "dropped_events": {"scripts": 0, "network_requests": 0, "console_events": 0, "websocket_connections": 0, "websocket_frames": 0, "webmcp_tools": 0, "timeline_events": 0, "total": 0} })
}
fn truncated() -> Value {
    let mut c = complete();
    c["status"] = json!("truncated");
    c["conditions"] = json!(["truncated"]);
    c["truncated_sections"] = json!(["network_requests"]);
    c["dropped_events"]["network_requests"] = json!(3);
    c["dropped_events"]["total"] = json!(3);
    c
}
fn set_net(root: &Path, rel: &str, b: Option<Value>, comp: Option<Value>, shapes: Option<Value>) {
    edit(root, rel, |e| {
        let mut n = json!({ "requests": 4, "bytes": 1, "byType": {"xhr": 4}, "failed": [], "thirdPartyHosts": 0 });
        if let Some(b) = &b {
            n["behaviour"] = b.clone();
        }
        if let Some(c) = &comp {
            n["completeness"] = c.clone();
        }
        if let Some(s) = &shapes {
            n["shapes"] = s.clone();
        }
        e["net"] = n;
    });
}
fn run(d: &Path) -> (i32, Value) {
    cg(d, &["net-diff", "--ours", "cluttered", "--ref", "clean", "--screen", "room", "--state", "populated", "--device", "1440x900"])
}
fn shape(path: &str, digest: u32, keys: &[&str]) -> Value {
    json!({ "method": "GET", "path": path, "status": 200, "digest": sha(digest), "shape": {"root_type": "object", "keys": keys} })
}

#[test]
fn added_removed_changed_and_shape_from_the_probes_records() {
    let d = root();
    set_net(d.path(), OURS, Some(behaviour(vec![tx(1, "GET", "/api/me", Some(200)), tx(2, "GET", "/api/only-ours", Some(200)), tx(3, "GET", "/api/x", Some(200)), tx(4, "GET", "/api/s", Some(200))])), Some(complete()), Some(json!([shape("/api/s", 40, &["a"])])));
    set_net(d.path(), REF, Some(behaviour(vec![tx(1, "GET", "/api/me", Some(200)), tx(5, "GET", "/api/only-ref", Some(404)), tx(6, "GET", "/api/x", Some(500)), tx(4, "GET", "/api/s", Some(200))])), Some(complete()), Some(json!([shape("/api/s", 41, &["a", "b"])])));
    let (c, v) = run(d.path());
    assert_eq!(c, 0, "{v}");
    assert_eq!(v["data"]["source"], "net-behaviour/1");
    assert_eq!(v["data"]["status"], "changed");
    let s = &v["data"]["summary"];
    assert_eq!((s["added"].clone(), s["removed"].clone(), s["changed"].clone(), s["same"].clone(), s["shape"].clone()), (json!(1), json!(1), json!(1), json!(2), json!(1)), "{v}");
    let items = v["data"]["captures"][0]["items"].as_array().unwrap();
    let kinds: Vec<(&str, &str)> = items.iter().map(|i| (i["kind"].as_str().unwrap(), i["key"].as_str().unwrap())).collect();
    assert_eq!(
        kinds,
        vec![("added", "GET /api/only-ours [xhr]"), ("removed", "GET /api/only-ref [xhr]"), ("shape", "GET /api/s"), ("changed", "GET /api/x [xhr]")],
        "{v}"
    );
    let ch = items.iter().find(|i| i["kind"] == "changed").unwrap();
    assert_eq!((ch["ours"].clone(), ch["ref"].clone()), (json!("status 200"), json!("status 500")));
}

#[test]
fn equal_behaviour_is_unchanged_only_when_both_captures_are_complete() {
    let d = root();
    let same = || behaviour(vec![tx(1, "GET", "/api/me", Some(200)), tx(2, "GET", "/api/me", Some(200))]);
    set_net(d.path(), OURS, Some(same()), Some(complete()), None);
    set_net(d.path(), REF, Some(same()), Some(complete()), None);
    let (_, v) = run(d.path());
    assert_eq!((v["data"]["status"].clone(), v["data"]["summary"]["same"].clone()), (json!("unchanged"), json!(2)), "{v}");
    // a truncated capture can show a difference but never "unchanged"
    set_net(d.path(), REF, Some(same()), Some(truncated()), None);
    let (_, v) = run(d.path());
    assert_eq!(v["data"]["status"], "unknown", "{v}");
    assert!(v["data"]["captures"][0]["reasons"].to_string().contains("dropped"), "{v}");
    // normalisation rules that differ: not comparable
    let mut other = same();
    other["normalization"]["sha256"] = json!(sha(777));
    set_net(d.path(), REF, Some(other), Some(complete()), None);
    let (_, v) = run(d.path());
    assert_eq!(v["data"]["status"], "unknown", "{v}");
}

#[test]
fn falls_back_to_the_net_block_and_refuses_when_neither_side_has_one() {
    let d = root();
    for rel in [OURS, REF] {
        edit(d.path(), rel, |e| {
            e.as_object_mut().unwrap().remove("net");
        });
    }
    let (c, v) = run(d.path());
    assert_eq!(c, 1, "{v}");
    assert_eq!(v["error"]["code"], "not_found");
    edit(d.path(), OURS, |e| e["net"] = json!({"requests": 4, "bytes": 1, "byType": {"xhr": 4}, "failed": [{"status": 500, "path": "/api/boom"}], "thirdPartyHosts": 0}));
    edit(d.path(), REF, |e| e["net"] = json!({"requests": 4, "bytes": 1, "byType": {"xhr": 4}, "failed": [], "thirdPartyHosts": 0}));
    let (c, v) = run(d.path());
    assert_eq!(c, 0, "{v}");
    assert_eq!((v["data"]["source"].clone(), v["data"]["summary"]["added"].clone()), (json!("net"), json!(1)), "{v}");
}

#[test]
fn strict_on_anything_else() {
    let d = root();
    let mut b = behaviour(vec![tx(1, "GET", "/a", Some(200))]);
    b["transactions"][0]["extra"] = json!(1);
    set_net(d.path(), OURS, Some(b), Some(complete()), None);
    let (_, v) = run(d.path());
    assert!(v["evidence"]["limitations"].to_string().contains("invalid_evidence") || v["error"].is_object(), "an unknown field must be refused: {v}");
    let d = root();
    let mut b = behaviour(vec![tx(1, "GET", "/a", Some(200))]);
    b["transactions"][0]["fingerprint"] = json!("not-a-sha");
    set_net(d.path(), OURS, Some(b), Some(complete()), None);
    let (_, v) = run(d.path());
    assert!(v.to_string().contains("invalid_evidence") || v["error"].is_object(), "{v}");
}

#[test]
fn deterministic_and_bounded() {
    let d = root();
    let many: Vec<Value> = (0..400).map(|i| tx(1000 + i, "GET", &format!("/api/x{i}"), Some(200))).collect();
    set_net(d.path(), OURS, Some(behaviour(many)), Some(complete()), None);
    set_net(d.path(), REF, Some(behaviour(vec![])), Some(complete()), None);
    let (_, a) = run(d.path());
    let (_, b) = run(d.path());
    assert_eq!(a, b);
    assert_eq!(a["data"]["summary"]["added"], 400);
    assert_eq!(a["data"]["captures"][0]["items"].as_array().unwrap().len(), 200);
    assert!(a["evidence"]["limitations"].to_string().contains("net_items_capped"), "{a}");
}
