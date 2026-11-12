//! CMP-UI-LOOP iteration 1: engine fixes found by comparing the first real slice runs (ours admin/guest, 2026-11-12).
//! Every test was seen red on the code before its fix. Roots are copies of the contract fixture, edited into the SHAPE the real slice files have
//! (no registry.json / archetypes.json in the run root, `archetype` named on each capture, E1 null with "no <main>", nav targets all `unknown`).
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
    let out = Command::cargo_bin("comparegraph").unwrap().current_dir(dir).arg("--pretty").arg("ui").args(args).arg("--ui-root").arg(".").output().unwrap();
    let s = String::from_utf8(out.stdout).unwrap();
    (out.status.code().unwrap(), serde_json::from_str(&s).unwrap_or_else(|e| panic!("not json ({e}): {s}")))
}
fn read(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}
fn edit(p: &Path, f: impl Fn(&mut Value)) {
    let mut v = read(p);
    f(&mut v);
    std::fs::write(p, serde_json::to_string_pretty(&v).unwrap()).unwrap();
}
fn root() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    copy_dir(&fx("ui-contract"), d.path());
    d
}
fn metric<'a>(v: &'a Value, id: &str) -> &'a Value {
    v["data"]["metrics"].as_array().unwrap().iter().find(|m| m["id"] == id).unwrap()
}
const ROOM: &str = "ui/cluttered/room/populated/1440x900-member.json";

/// A slice-shaped capture: the probe's E1 is null ("no <main>: not measurable") and its limit stands next to it.
fn slice_shape(d: &Path) {
    edit(&d.join(ROOM), |e| {
        e["metrics"]["E1"] = json!({ "value": null, "limit": 2, "pass": false, "detail": ["no <main>: not measurable"] });
        e["archetype"] = json!("conversation");
    });
}

mod unmeasured {
    use super::*;
    #[test]
    fn a_null_metric_is_unmeasured_never_a_fail_and_never_counted() {
        let d = root();
        let (_, before) = cg(d.path(), &["show", "cluttered", "room"]);
        let pass_before = before["data"]["pass"].as_u64().unwrap();
        slice_shape(d.path());
        let (c, v) = cg(d.path(), &["show", "cluttered", "room"]);
        assert_eq!(c, 0, "{v}");
        let e1 = metric(&v, "E1");
        assert_eq!(e1["status"], "unmeasured", "{e1}");
        assert_eq!(e1["pass"], Value::Null, "an unmeasured metric has no pass/fail: {e1}");
        assert_eq!(e1["unmeasuredReason"], "no <main>: not measurable", "{e1}");
        assert_eq!(v["data"]["unmeasured"].as_u64().unwrap(), before["data"]["unmeasured"].as_u64().unwrap() + 1, "E1 joins the fixture's own null E7: {v}");
        assert_eq!(v["data"]["pass"].as_u64().unwrap(), pass_before - u64::from(metric(&before, "E1")["pass"] == true), "pass counts only measured passes");
        // a measured metric keeps a status too
        assert!(["pass", "fail"].contains(&metric(&v, "E2")["status"].as_str().unwrap()), "{}", metric(&v, "E2"));
    }
    #[test]
    fn e1_null_on_a_page_with_navigation_columns_is_a_structural_finding() {
        let d = root();
        slice_shape(d.path());
        let (_, v) = cg(d.path(), &["show", "cluttered", "room"]);
        assert!(v["data"]["layout"]["navColumns"].as_u64().unwrap() >= 1, "the fixture room has navigation columns: {v}");
        let f = v["data"]["structuralFindings"].as_array().unwrap();
        assert_eq!(f.iter().filter(|x| x["id"] == "no_main_landmark").count(), 1, "{f:?}");
    }
    #[test]
    fn e7_null_is_unmeasured_but_is_not_a_landmark_finding() {
        let d = root();
        edit(&d.path().join(ROOM), |e| e["metrics"]["E7"] = json!({ "value": null, "limit": 0.4, "pass": false, "detail": ["no content-start marker"] }));
        let (_, v) = cg(d.path(), &["show", "cluttered", "room"]);
        assert_eq!(metric(&v, "E7")["status"], "unmeasured", "{}", metric(&v, "E7"));
        assert!(v["data"]["structuralFindings"].as_array().unwrap().is_empty(), "{v}");
    }
    #[test]
    fn diff_shows_unmeasured_apart_from_pass_and_fail() {
        let d = root();
        slice_shape(d.path());
        let (c, v) = cg(d.path(), &["diff", "room", "--ours", "cluttered", "--ref", "clean"]);
        assert_eq!(c, 0, "{v}");
        assert_eq!(metric(&v, "E1")["ours"]["status"], "unmeasured", "{}", metric(&v, "E1"));
        assert_eq!(v["data"]["unmeasured"]["ours"], 2, "E1 and the fixture's own null E7: {v}");
    }
}

mod archetype {
    use super::*;
    fn without_registry(d: &Path) {
        std::fs::remove_file(d.join("ui/registry.json")).unwrap();
        std::fs::remove_file(d.join("ui/archetypes.json")).unwrap();
        slice_shape(d);
    }
    #[test]
    fn the_captures_own_archetype_applies_when_the_run_root_has_no_registry() {
        let d = root();
        without_registry(d.path());
        let (c, v) = cg(d.path(), &["show", "cluttered", "room"]);
        assert_eq!(c, 0, "{v}");
        assert_eq!(v["data"]["archetype"], "conversation", "{v}");
        let e2 = metric(&v, "E2");
        assert_eq!(e2["limitSource"], "probe-default:conversation", "{e2}");
    }
    #[test]
    fn screens_and_diff_name_the_archetype_too() {
        let d = root();
        without_registry(d.path());
        let (_, v) = cg(d.path(), &["screens", "cluttered"]);
        let room = v["data"]["items"].as_array().unwrap().iter().find(|i| i["screen"] == "room").unwrap();
        assert_eq!(room["archetype"], "conversation", "{v}");
        let (_, v) = cg(d.path(), &["diff", "room", "--ours", "cluttered", "--ref", "clean"]);
        assert_eq!(v["data"]["archetype"], "conversation", "{v}");
    }
    #[test]
    fn a_registry_limit_names_its_archetype_in_the_source() {
        let d = root();
        let (_, v) = cg(d.path(), &["show", "cluttered", "room"]);
        assert_eq!(metric(&v, "E1")["limitSource"], "registry:conversation", "{}", metric(&v, "E1"));
    }
}

mod nav {
    use super::*;
    const NAV: &str = "ui/cluttered/nav/1440x900-member.json";
    /// the probe's real shape: every target `unknown`, one group header, tabs
    fn probe_shape(d: &Path) {
        edit(&d.join(NAV), |n| {
            let mut nodes = vec![json!({ "badge": false, "hiddenBehind": null, "icon": false, "kind": "group", "label": "Members (#)", "level": 1, "order": 0, "target": "unknown" })];
            for i in 0..3 {
                nodes.push(json!({ "badge": false, "hiddenBehind": null, "icon": true, "kind": "place", "label": format!("P{i}"), "level": 1, "order": i + 1, "target": "unknown" }));
            }
            nodes.push(json!({ "badge": false, "hiddenBehind": null, "icon": false, "kind": "tab", "label": "T", "level": 1, "order": 4, "target": "unknown" }));
            n["nodes"] = json!(nodes);
        });
    }
    #[test]
    fn a_group_header_is_not_a_dead_item_and_a_tab_is_not_a_place() {
        let d = root();
        probe_shape(d.path());
        let (c, v) = cg(d.path(), &["nav", "cluttered"]);
        assert_eq!(c, 0, "{v}");
        let s = &v["data"]["stats"];
        assert_eq!((s["nodes"].as_u64(), s["places"].as_u64(), s["deadItems"].as_u64()), (Some(5), Some(3), Some(4)), "{s}");
    }
    #[test]
    fn when_no_target_was_resolved_the_answer_says_so_instead_of_calling_every_item_dead() {
        let d = root();
        probe_shape(d.path());
        let (_, v) = cg(d.path(), &["nav", "cluttered"]);
        let lims = v["evidence"]["limitations"].to_string();
        assert!(lims.contains("nav_targets_unresolved"), "{lims}");
    }
    fn rename_viewport(d: &Path) {
        for p in ["cluttered", "clean"] {
            let dir = d.join("ui").join(p).join("nav");
            let f = std::fs::read_dir(&dir).unwrap().flatten().next().unwrap().path();
            let seat = if p == "clean" { "ref" } else { "member" };
            let to = dir.join(format!("desktop-1440-{seat}.json"));
            std::fs::rename(&f, &to).unwrap();
            edit(&to, |n| n["viewport"] = json!("desktop-1440"));
        }
    }
    #[test]
    fn nav_diff_defaults_to_the_one_viewport_and_seat_both_sides_have() {
        let d = root();
        rename_viewport(d.path());
        let (c, v) = cg(d.path(), &["nav-diff", "--ours", "cluttered", "--ref", "clean"]);
        assert_eq!(c, 0, "{v}");
        assert_eq!(v["data"]["viewport"], "desktop-1440", "{v}");
    }
    fn add_iphone(d: &Path) {
        for p in ["cluttered", "clean"] {
            let dir = d.join("ui").join(p).join("nav");
            let seat = if p == "clean" { "ref" } else { "member" };
            let to = dir.join(format!("iphone-{seat}.json"));
            std::fs::copy(dir.join(format!("desktop-1440-{seat}.json")), &to).unwrap();
            edit(&to, |n| n["viewport"] = json!("iphone"));
        }
    }
    #[test]
    fn nav_diff_prefers_the_default_desktop_viewport_when_both_sides_have_several() {
        let d = root();
        rename_viewport(d.path());
        add_iphone(d.path());
        let (c, v) = cg(d.path(), &["nav-diff", "--ours", "cluttered", "--ref", "clean"]);
        assert_eq!(c, 0, "{v}");
        assert_eq!(v["data"]["viewport"], "desktop-1440", "{v}");
    }
    #[test]
    fn nav_diff_with_several_non_default_viewports_refuses_and_lists_the_choices() {
        let d = root();
        rename_viewport(d.path());
        add_iphone(d.path());
        for p in ["cluttered", "clean"] {
            let seat = if p == "clean" { "ref" } else { "member" };
            let dir = d.path().join("ui").join(p).join("nav");
            let to = dir.join(format!("ipad-portrait-{seat}.json"));
            std::fs::rename(dir.join(format!("desktop-1440-{seat}.json")), &to).unwrap();
            edit(&to, |n| n["viewport"] = json!("ipad-portrait"));
        }
        let (c, v) = cg(d.path(), &["nav-diff", "--ours", "cluttered", "--ref", "clean"]);
        assert_ne!(c, 0, "{v}");
        let m = v.to_string();
        assert!(m.contains("ipad-portrait") && m.contains("iphone") && m.contains("--viewport"), "the refusal lists the choices: {m}");
    }
}

mod coverage {
    use super::*;
    fn cov(d: &Path) -> PathBuf {
        std::fs::read_dir(d.join("ui/coverage")).unwrap().flatten().map(|e| e.path()).find(|p| read(p)["cells"].as_array().is_some_and(|c| c.iter().any(|x| x["product"] == "cluttered"))).unwrap()
    }
    fn blank_cells(d: &Path, plan: Option<Value>) -> String {
        let p = cov(d);
        let run = p.file_stem().unwrap().to_str().unwrap().to_string();
        edit(&p, |c| {
            let cells = c["cells"].as_array_mut().unwrap();
            for st in ["empty", "long", "error"] {
                cells.push(json!({ "product": "cluttered", "screen": "inbox", "seat": "member", "state": st, "status": "blank", "viewport": "1440x900", "why": "not in this run's plan and no n/a reason given" }));
            }
            cells.push(json!({ "product": "cluttered", "screen": "inbox", "seat": "member", "state": "populated", "status": "blank", "viewport": "iphone", "why": "" }));
            if let Some(pl) = &plan {
                c["plan"] = pl.clone();
            }
        });
        run
    }
    #[test]
    fn a_plan_na_rule_turns_the_unplanned_states_into_explained_na_and_leaves_other_blanks_blank() {
        let d = root();
        let run = blank_cells(d.path(), Some(json!({ "na": [{ "states": ["empty", "long", "error", "loading-skeleton"], "why": "no seeded workspace (SEED lane)" }] })));
        let (c, v) = cg(d.path(), &["coverage", &run]);
        assert!(c == 0 || c == 1, "{v}");
        let blanks = v["data"]["blankCells"].as_array().unwrap();
        assert_eq!(blanks.len(), 1, "only the populated/iphone cell stays blank: {blanks:?}");
        assert!(blanks[0].as_str().unwrap().contains("populated"), "{blanks:?}");
        assert_eq!(v["data"]["plannedNa"], 3, "{v}");
        assert_eq!(v["data"]["plannedNaReasons"][0]["why"], "no seeded workspace (SEED lane)", "{v}");
    }
    #[test]
    fn without_a_plan_the_same_cells_stay_blank() {
        let d = root();
        let run = blank_cells(d.path(), None);
        let (_, v) = cg(d.path(), &["coverage", &run]);
        assert_eq!(v["data"]["blankCells"].as_array().unwrap().len(), 4, "{v}");
    }
    #[test]
    fn a_plan_na_rule_without_a_reason_is_refused() {
        let d = root();
        let run = blank_cells(d.path(), Some(json!({ "na": [{ "states": ["empty"], "why": " " }] })));
        let (c, v) = cg(d.path(), &["coverage", &run]);
        assert_ne!(c, 0, "{v}");
        assert!(v.to_string().contains("plan.na"), "{v}");
    }
}

mod probe_loop1_contract {
    use super::*;
    #[test]
    fn e13_and_e14_are_read_judged_and_shown_with_fourteen_keys() {
        let d = root();
        edit(&d.path().join(ROOM), |e| {
            e["metrics"]["E13"] = json!({ "value": 1, "limit": 0, "pass": false, "detail": ["acknowledge in aside[Reminders] x4"] });
            e["metrics"]["E14"] = json!({ "value": 1, "limit": 0, "pass": false, "detail": ["card: 3 equal-weight buttons, no primary"] });
            e["metrics"]["E6"]["detail"] = json!(["offscreen-right: Save +40px", "tab-cut: Billing", "panel-cut: Drawer"]);
            e["metrics"]["E10"]["detail"] = json!(["nav-dup /rooms x2"]);
        });
        // a registry limit for the new key is accepted too
        edit(&d.path().join("ui/archetypes.json"), |a| a["archetypes"]["conversation"]["limits"]["E13"] = json!({ "desktop": "<=1", "phone": "<=1" }));
        let (c, v) = cg(d.path(), &["show", "cluttered", "room"]);
        assert_eq!(c, 0, "{v}");
        assert_eq!(v["data"]["metrics"].as_array().unwrap().len(), 14, "{v}");
        assert_eq!(v["data"]["of"], 14, "{v}");
        let e13 = metric(&v, "E13");
        assert_eq!((e13["status"].as_str(), e13["limit"].as_str(), e13["limitSource"].as_str()), (Some("pass"), Some("<=1"), Some("registry:conversation")), "{e13}");
        assert_eq!(metric(&v, "E14")["status"], "fail");
        let (_, m) = { map_room(d.path()); cg(d.path(), &["matrix", "devfam"]) };
        assert!(m.to_string().contains("/14"), "matrix cells count the 14 keys the capture carries: {m}");
    }
    #[test]
    fn a_redirect_capture_is_shown_as_redirect_and_not_compared_as_the_requested_screen() {
        let d = root();
        edit(&d.path().join(ROOM), |e| {
            e["support"] = json!({ "class": "redirect", "reasons": ["same-host route redirect"], "target": "https://example.test/rooms" });
            e["meta"]["finalUrl"] = json!("https://example.test/rooms");
            e["meta"]["requestedUrl"] = json!("https://example.test/room");
        });
        let (c, v) = cg(d.path(), &["diff", "room", "--ours", "cluttered", "--ref", "clean"]);
        assert_ne!(c, 0, "{v}");
        assert!(v.to_string().contains("redirect") && v.to_string().contains("https://example.test/rooms"), "{v}");
        let (_, m) = { map_room(d.path()); cg(d.path(), &["matrix", "devfam"]) };
        assert!(m.to_string().contains("redirect -> https://example.test/rooms"), "{m}");
    }
    #[test]
    fn a_graph_truncated_by_actions_is_reported_truncated_never_complete() {
        let d = root();
        edit(&d.path().join("ui/cluttered/graph/1440x900-member.json"), |g| {
            g["truncated"] = json!({ "depth": false, "nodes": false, "time": false, "actions": true, "screenActions": ["room"], "unexpanded": [] });
            g["caps"] = json!({ "depth": 3, "nodes": 300, "actions": 40, "screenActions": 12, "clickTimeoutMs": 800 });
        });
        let (c, v) = cg(d.path(), &["graph", "cluttered"]);
        assert_eq!(c, 0, "{v}");
        assert_eq!(v["data"]["complete"], false, "{v}");
        assert_eq!(v["data"]["truncated"]["screenActions"], json!(["room"]), "{v}");
        assert_eq!(v["data"]["caps"]["clickTimeoutMs"], 800, "{v}");
        assert!(v["evidence"]["limitations"].to_string().contains("graph_truncated") && v["evidence"]["limitations"].to_string().contains("room"), "{v}");
    }
    #[test]
    fn an_untruncated_graph_says_complete() {
        let d = root();
        edit(&d.path().join("ui/cluttered/graph/1440x900-member.json"), |g| g["truncated"] = json!({ "depth": false, "nodes": false, "time": false, "unexpanded": [] }));
        let (_, v) = cg(d.path(), &["graph", "cluttered"]);
        assert_eq!(v["data"]["complete"], true, "{v}");
    }
}

fn map_room(d: &Path) {
    let p = d.join("ui/map.csv");
    let mut csv = std::fs::read_to_string(&p).unwrap();
    csv.push_str("devfam,r-room,cluttered,room,/room,,\n");
    std::fs::write(p, csv).unwrap();
}
