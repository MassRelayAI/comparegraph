//! CMP-UI-1g: the interaction graph verbs (`ui graph`, `ui path`, `ui graph-diff`), against hand-written graphs whose every number is worked out by hand here.
use assert_cmd::Command;
use serde_json::{json, Value};
use std::path::Path;

fn cg(dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = Command::cargo_bin("comparegraph").unwrap().current_dir(dir).arg("ui").args(args).arg("--ui-root").arg(".").output().unwrap();
    let s = String::from_utf8(out.stdout).unwrap();
    (out.status.code().unwrap(), serde_json::from_str(&s).unwrap_or_else(|e| panic!("not json ({e}): {s}")))
}
fn node(id: &str) -> Value {
    json!({ "id": id, "state": "populated", "route": format!("/{id}"), "domHash": "a".repeat(64) })
}
fn edge(from: &str, kind: &str, name: &str, region: &str, to: &str, effect: &str) -> Value {
    json!({ "from": from, "action": { "kind": kind, "name": name, "region": region }, "to": to, "effect": effect })
}
fn put(root: &Path, product: &str, seat: &str, g: &Value) {
    let d = root.join("ui").join(product).join("graph");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join(format!("1440x900-{seat}.json")), g.to_string()).unwrap();
}
fn graph(product: &str, seat: &str, nodes: &[&str], edges: Vec<Value>, registry: &[&str]) -> Value {
    json!({ "schema": "ui-graph/1", "product": product, "version": "v1", "viewport": "1440x900", "seat": seat, "start": "home", "registryScreens": registry,
        "caps": { "depth": 6, "nodes": 300 }, "truncated": { "depth": false, "nodes": false, "unexpanded": [] },
        "nodes": nodes.iter().map(|n| node(n)).collect::<Vec<_>>(), "edges": edges,
        "derived": { "note": "ignored: CompareGraph recomputes" }, "probe": { "version": "1" }, "meta": { "capturedAt": "2026-11-12T10:00:00Z" } })
}
/// ours: home -> rooms, inbox, settings; rooms -> room; room -> (dialog Share) -> hidden; inbox has a dead button; home has a delete it never clicked.
fn ours() -> Value {
    graph(
        "ours",
        "member",
        &["home", "rooms", "room", "inbox", "settings", "hidden", "room#dialog:Share"],
        vec![
            edge("home", "menu", "Rooms", "sidebar", "rooms", "navigate"),
            edge("home", "menu", "Inbox", "sidebar", "inbox", "navigate"),
            edge("home", "menu", "Settings", "sidebar", "settings", "navigate"),
            edge("home", "button", "Delete workspace", "main", "unknown", "mutating-not-clicked"),
            edge("rooms", "row", "Open room", "main", "room", "navigate"),
            edge("rooms", "menu", "Home", "sidebar", "home", "navigate"),
            edge("room", "button", "Share", "main", "room#dialog:Share", "open-dialog"),
            edge("room", "back", "Back", "main", "rooms", "navigate"),
            edge("room#dialog:Share", "link", "Invite page", "panel", "hidden", "navigate"),
            edge("inbox", "button", "Mark all", "main", "inbox", "none"),
            edge("inbox", "link", "Settings", "main", "settings", "navigate"),
        ],
        &["home", "rooms", "room", "inbox", "settings", "hidden", "billing"],
    )
}
fn reference() -> Value {
    graph(
        "ref",
        "ref",
        &["home", "rooms", "room", "inbox", "settings"],
        vec![
            edge("home", "menu", "Rooms", "sidebar", "rooms", "navigate"),
            edge("home", "menu", "Inbox", "sidebar", "inbox", "navigate"),
            edge("home", "menu", "Settings", "sidebar", "settings", "navigate"),
            edge("home", "link", "Open the first room", "main", "room", "navigate"),
        ],
        &["home", "rooms", "room", "inbox", "settings"],
    )
}
fn root() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    put(d.path(), "ours", "member", &ours());
    put(d.path(), "ref", "ref", &reference());
    d
}

#[test]
fn graph_numbers_are_worked_out_by_hand() {
    let d = root();
    let (code, v) = cg(d.path(), &["graph", "ours"]);
    assert_eq!(code, 0, "{v}");
    let (data, s) = (&v["data"], &v["data"]["summary"]);
    assert_eq!(data["start"], "home");
    // depths: home 0; rooms, inbox, settings 1; room 2; the Share dialog 3; hidden 4 (only through the dialog)
    assert_eq!(data["depth"], json!({ "home": 0, "hidden": 4, "inbox": 1, "room": 2, "room#dialog:Share": 3, "rooms": 1, "settings": 1 }));
    assert_eq!((s["nodes"].as_u64(), s["edges"].as_u64(), s["reachable"].as_u64(), s["maxDepth"].as_u64()), (Some(7), Some(11), Some(7), Some(4)));
    assert_eq!(s["meanDepth"].as_f64(), Some(1.71), "(0+1+1+1+2+3+4)/7"); // 12/7
    assert_eq!(data["orphans"], json!(["billing"]), "the registry has billing, no edge reaches it");
    assert_eq!(s["deadActions"], 1);
    assert_eq!(data["deadActions"][0]["name"], "Mark all");
    assert_eq!(data["deadActions"][0]["effect"], "none");
    assert_eq!(s["mutatingNotClicked"], 1);
    assert_eq!(s["cycles"], 1, "home -> rooms -> home");
    assert_eq!(data["duplicatePaths"], json!([{ "to": "rooms", "sources": ["home", "room"] }, { "to": "settings", "sources": ["home", "inbox"] }]), "rooms: the menu and Back; settings: home's menu and inbox");
    assert_eq!(data["actionsPerScreen"]["home"], 4);
    assert_eq!(s["actionsMax"], 4);
    assert_eq!(v["evidence"]["limitations"], json!([]));
}

#[test]
fn the_probes_own_derived_block_is_never_trusted() {
    let d = root();
    let mut g = ours();
    g["derived"] = json!({ "orphanCount": 99, "maxDepth": 99 });
    put(d.path(), "ours", "member", &g);
    let (_, v) = cg(d.path(), &["graph", "ours"]);
    assert_eq!(v["data"]["summary"]["maxDepth"], 4);
    assert_eq!(v["data"]["orphans"], json!(["billing"]));
}

#[test]
fn path_is_the_shortest_click_sequence_and_is_bounded() {
    let d = root();
    let (_, v) = cg(d.path(), &["path", "ours", "home", "hidden"]);
    assert_eq!(v["data"]["found"], true);
    assert_eq!(v["data"]["clicks"], 4);
    let names: Vec<&str> = v["data"]["steps"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Rooms", "Open room", "Share", "Invite page"]);
    assert_eq!(v["data"]["steps"][2]["effect"], "open-dialog");
    // bounded: three clicks is not enough, and the answer says so instead of guessing
    let (code, b) = cg(d.path(), &["path", "ours", "home", "hidden", "--max-len", "3"]);
    assert_eq!(code, 0);
    assert_eq!(b["data"]["found"], false);
    assert!(b["evidence"]["limitations"].to_string().contains("no_path"), "{b}");
    // a mutating-not-clicked edge is not a path
    let (_, m) = cg(d.path(), &["path", "ours", "home", "inbox"]);
    assert_eq!(m["data"]["clicks"], 1);
    // unknown node: not_found names what exists
    let (c, e) = cg(d.path(), &["path", "ours", "home", "nowhere"]);
    assert_eq!((c, e["error"]["code"].as_str()), (1, Some("not_found")), "{e}");
    let (c, e) = cg(d.path(), &["path", "ours", "home", "inbox", "--max-len", "99"]);
    assert_eq!((c, e["error"]["code"].as_str()), (1, Some("invalid_arguments")), "{e}");
}

#[test]
fn graph_diff_gives_depth_to_each_comparable_screen() {
    let d = root();
    let (code, v) = cg(d.path(), &["graph-diff", "--ours", "ours", "--ref", "ref"]);
    assert_eq!(code, 0, "{v}");
    let rows = v["data"]["depth"].as_array().unwrap();
    let row = |s: &str| rows.iter().find(|r| r["screen"] == s).unwrap_or_else(|| panic!("no row {s}: {rows:?}"));
    // room: ours 2 (home -> rooms -> room), ref 1 (home -> Open the first room)
    assert_eq!((row("room")["ours"].as_u64(), row("room")["ref"].as_u64(), row("room")["verdict"].as_str()), (Some(2), Some(1), Some("ours deeper by 1")));
    assert_eq!(row("inbox")["verdict"], "same");
    assert_eq!((row("hidden")["ours"].as_u64(), row("hidden")["verdict"].as_str()), (Some(4), Some("only ours")));
    assert!(rows.iter().all(|r| !r["screen"].as_str().unwrap().contains('#')), "dialogs are not comparable screens");
    assert_eq!(v["data"]["onlyOurs"], json!(["hidden"]));
    assert_eq!(v["data"]["orphans"]["ours"], json!(["billing"]));
    assert_eq!(v["data"]["orphans"]["ref"], json!([]));
    let diff = v["data"]["diff"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect::<Vec<_>>();
    assert!(diff.contains(&"max depth 4 vs 1") && diff.contains(&"orphans 1 vs 0") && diff.contains(&"dead actions 1 vs 0"), "{diff:?}");
}

#[test]
fn graph_diff_against_target_builds_the_intended_graph_from_target_nav() {
    let d = root();
    let target = json!({ "schema": "ui-nav/1", "product": "target", "trees": { "app": { "nodes": [
        { "label": "Home", "kind": "place", "level": 1, "target": "home" },
        { "label": "Rooms", "kind": "place", "level": 1, "target": "rooms" },
        { "label": "Inbox", "kind": "place", "level": 1, "target": "inbox" },
        { "label": "Settings", "kind": "place", "level": 1, "target": "settings" },
        { "label": "Room", "kind": "item", "level": 2, "target": "room" },
        { "label": "Billing", "kind": "item", "level": 2, "target": "billing" } ] } } });
    std::fs::write(d.path().join("ui/target-nav.json"), target.to_string()).unwrap();
    let (code, v) = cg(d.path(), &["graph-diff", "--ours", "ours", "--ref", "target"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["ref"]["product"], "target");
    let rows = v["data"]["depth"].as_array().unwrap();
    let get = |s: &str| rows.iter().find(|r| r["screen"] == s).unwrap().clone();
    // every place is one click from Home; a level-2 item hangs off the nearest level-1 place before it (Settings), so billing would be two clicks away
    assert_eq!(get("rooms")["ref"], 1);
    assert_eq!(get("billing")["verdict"], "only ref", "the intended graph has billing; ours does not reach it");
    assert_eq!(get("billing")["ours"], Value::Null);
    let (c, e) = cg(d.path(), &["graph-diff", "--ours", "ours", "--ref", "target", "--tree", "nope"]);
    assert_eq!((c, e["error"]["code"].as_str()), (1, Some("not_found")), "{e}");
}

#[test]
fn a_cut_crawl_says_so_and_a_broken_graph_is_listed_as_invalid() {
    let d = root();
    let mut g = ours();
    g["truncated"] = json!({ "depth": true, "nodes": false, "unexpanded": ["hidden"] });
    put(d.path(), "ours", "member", &g);
    let (_, v) = cg(d.path(), &["graph", "ours"]);
    assert!(v["evidence"]["limitations"].to_string().contains("graph_truncated"), "{v}");
    // an edge to a node that does not exist (and the crawl was not cut by the node cap) is invalid evidence, not a silent drop
    let mut bad = ours();
    bad["edges"].as_array_mut().unwrap().push(edge("home", "link", "Ghost", "main", "ghost", "navigate"));
    put(d.path(), "ours", "admin", &bad);
    let (_, v) = cg(d.path(), &["graph", "ours", "--seat", "admin"]);
    assert_eq!(v["ok"], false, "{v}");
    assert!(v.to_string().contains("not a node"), "the reason the file was refused is in the answer: {v}");
}

#[test]
fn graph_files_do_not_disturb_the_evidence_verbs() {
    let d = root();
    let (_, v) = cg(d.path(), &["products"]);
    let names: Vec<&str> = v["data"]["items"].as_array().unwrap().iter().map(|i| i["product"].as_str().unwrap()).collect();
    assert_eq!(names, ["ours", "ref"]);
    assert!(!v.to_string().contains("invalid_evidence"), "{v}");
}
