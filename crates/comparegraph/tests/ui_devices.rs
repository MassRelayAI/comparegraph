//! CMP-UI-1i: device profiles and support classes in the engine (CMP-UI-1-lanes sections 17 and 17b). The data is the probe's own `devprod` product in
//! `fixtures/ui-contract/` (every support class once: full, degraded by a 30px tap target and by overflow, blocked, redirect, absent), edited in temp copies.
//! Classes: read (the new blocks are valid), tablet (limits of the tablet class), phone (the phone column reaches a device id), cross_device (diff refuses),
//! matrix (support-class columns, absent cells), invalid (a malformed block is invalid_evidence).
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
fn cg(dir: &Path, args: &[&str]) -> (i32, Value, String) {
    let out = Command::cargo_bin("comparegraph").unwrap().current_dir(dir).arg("--pretty").arg("ui").args(args).arg("--ui-root").arg(".").output().unwrap();
    let s = String::from_utf8(out.stdout).unwrap();
    (out.status.code().unwrap(), serde_json::from_str(&s).unwrap_or_else(|e| panic!("not json ({e}): {s}")), s)
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
fn ev(d: &Path, screen: &str, stem: &str) -> PathBuf {
    d.join("ui/devprod").join(screen).join("populated").join(format!("{stem}.json"))
}
fn clean(d: &Path, args: &[&str]) -> Value {
    let (c, v, s) = cg(d, args);
    assert_eq!(c, 0, "{args:?} exited {c}: {s}");
    assert!(!s.contains("invalid_evidence"), "{args:?}: {s}");
    v
}
fn metric<'a>(v: &'a Value, id: &str) -> &'a Value {
    v["data"]["metrics"].as_array().unwrap().iter().find(|m| m["id"] == id).unwrap()
}

mod read {
    use super::*;
    #[test]
    fn every_support_class_of_the_probes_devprod_is_read_clean() {
        let d = root();
        let p = clean(d.path(), &["screens", "devprod"]);
        let screens: Vec<&str> = p["data"]["items"].as_array().unwrap().iter().map(|i| i["screen"].as_str().unwrap()).collect();
        assert_eq!(screens, ["inbox", "room", "settings"]);

        let ph = clean(d.path(), &["show", "devprod", "room", "--viewport", "iphone"]);
        assert_eq!((ph["data"]["deviceClass"].as_str(), ph["data"]["support"]["class"].as_str()), (Some("phone"), Some("degraded")), "{ph}");
        assert_eq!(ph["data"]["touch"]["failingCount"], 1);
        assert!(ph["data"]["touchLine"].as_str().unwrap().contains("1 of"), "{ph}");
        assert_eq!(ph["data"]["device"]["size"], "390x844");

        let pad = clean(d.path(), &["show", "devprod", "room", "--viewport", "ipad-landscape"]);
        assert_eq!(pad["data"]["deviceClass"], "tablet");
        assert_eq!(pad["data"]["touch"]["hoverOnlyCount"], 1, "the hover-only delete button is a finding: {pad}");

        let blocked = clean(d.path(), &["show", "devprod", "inbox", "--viewport", "iphone"]);
        assert_eq!(blocked["data"]["support"]["class"], "blocked");
        assert!(blocked["data"]["support"]["message"].as_str().unwrap().to_lowercase().contains("not available on mobile"), "{blocked}");

        let redirect = clean(d.path(), &["show", "devprod", "settings", "--viewport", "android"]);
        assert_eq!(redirect["data"]["support"]["class"], "redirect");
        assert!(redirect["data"]["support"]["target"].as_str().unwrap().starts_with("http://localhost:"), "{redirect}");

        let desktop = clean(d.path(), &["show", "devprod", "room"]); // no --viewport: the default device (desktop-1440) is preferred
        assert_eq!((desktop["data"]["viewport"].as_str(), desktop["data"]["support"]["class"].as_str()), (Some("desktop-1440"), Some("full")));
        assert!(desktop["data"]["touch"].is_null(), "no touch block on a desktop device");
    }
}

mod tablet {
    use super::*;
    fn set_value(d: &Path, stem: &str, id: &str, v: f64) {
        edit(&ev(d, "room", stem), |e| e["metrics"][id]["value"] = json!(v));
    }
    #[test]
    fn the_tablet_class_is_judged_by_its_own_limits_and_names_them() {
        let d = root();
        // E1 <= 1 side column, E4 <= 12 actions, E7 content start <= 0.25
        for (id, ok, bad) in [("E1", 1.0, 2.0), ("E4", 12.0, 13.0), ("E7", 0.25, 0.26)] {
            set_value(d.path(), "ipad-portrait-member", id, ok);
            let v = clean(d.path(), &["show", "devprod", "room", "--viewport", "ipad-portrait"]);
            let m = metric(&v, id);
            assert_eq!((m["pass"].clone(), m["limitSource"].as_str()), (json!(true), Some("tablet-default")), "{id} at {ok}: {m}");
            set_value(d.path(), "ipad-portrait-member", id, bad);
            let v = clean(d.path(), &["show", "devprod", "room", "--viewport", "ipad-portrait"]);
            let m = metric(&v, id);
            assert_eq!(m["pass"], false, "{id} at {bad} must fail on a tablet: {m}");
            assert!(m["limit"].as_str().unwrap().starts_with("<="), "{m}");
        }
    }
    #[test]
    fn the_same_value_is_judged_by_the_desktop_limit_on_a_desktop() {
        let d = root();
        set_value(d.path(), "desktop-1440-member", "E1", 2.0);
        let m = metric(&clean(d.path(), &["show", "devprod", "room", "--viewport", "desktop-1440"]), "E1").clone();
        assert_eq!((m["pass"].clone(), m["limitSource"].as_str(), m["limit"].as_str()), (json!(true), Some("registry"), Some("<=2")), "{m}");
    }
    #[test]
    fn an_archetype_tablet_column_wins_over_the_default_and_other_metrics_use_the_desktop_column() {
        let d = root();
        set_value(d.path(), "ipad-portrait-member", "E1", 4.0);
        edit(&d.path().join("ui/archetypes.json"), |a| a["archetypes"]["conversation"]["limits"]["E1"]["tablet"] = json!("<=5"));
        let v = clean(d.path(), &["show", "devprod", "room", "--viewport", "ipad-portrait"]);
        let m = metric(&v, "E1");
        assert_eq!((m["pass"].clone(), m["limitSource"].as_str(), m["limit"].as_str()), (json!(true), Some("registry"), Some("<=5")), "{m}");
        let e2 = metric(&v, "E2");
        assert_eq!((e2["limitSource"].as_str(), e2["limit"].as_str()), (Some("registry"), Some("=0")), "E2 has no tablet default: the desktop column: {e2}");
    }
}

mod phone {
    use super::*;
    #[test]
    fn the_phone_column_reaches_a_device_id_not_only_a_390_wide_viewport() {
        let d = root();
        let v = clean(d.path(), &["show", "devprod", "room", "--viewport", "iphone"]);
        assert_eq!(metric(&v, "E1")["limit"], "=0", "phone column of E1: {}", metric(&v, "E1"));
        let a = clean(d.path(), &["show", "devprod", "room", "--viewport", "android"]);
        assert_eq!(metric(&a, "E4")["limit"], "<=6");
        edit(&d.path().join("ui/archetypes.json"), |a| a["archetypes"]["conversation"]["limits"]["E1"]["phone"] = json!("<=9"));
        assert_eq!(metric(&clean(d.path(), &["show", "devprod", "room", "--viewport", "iphone"]), "E1")["limit"], "<=9");
        let desk = clean(d.path(), &["show", "devprod", "room", "--viewport", "desktop-1440"]);
        assert_eq!(metric(&desk, "E1")["limit"], "<=2");
    }
}

mod cross_device {
    use super::*;
    #[test]
    fn diff_refuses_a_phone_against_a_desktop_and_accepts_the_same_device() {
        let d = root();
        let (c, v, s) = cg(d.path(), &["diff", "room", "--ours", "devprod", "--ref", "devprod", "--viewport", "iphone", "--ref-viewport", "desktop-1440"]);
        assert_ne!(c, 0, "{s}");
        assert!(v["error"]["message"].as_str().unwrap_or(&s).contains("cross-device"), "{s}");
        let ok = clean(d.path(), &["diff", "room", "--ours", "devprod", "--ref", "devprod", "--viewport", "iphone"]);
        assert_eq!(ok["data"]["viewport"], "iphone");
        assert_eq!(ok["data"]["ours"]["support"]["class"], "degraded");
        // the default device and the legacy 1440x900 are the same device
        clean(d.path(), &["diff", "room", "--ours", "devprod", "--ref", "clean"]);
    }
    #[test]
    fn diff_does_not_silently_pick_a_phone_for_one_side_and_a_desktop_for_the_other() {
        let d = root();
        // the reference has only its 390x844 capture left: without the guard the default pick would set a phone against a desktop
        std::fs::remove_file(d.path().join("ui/clean/room/populated/1440x900-ref.json")).unwrap();
        let (c, v, s) = cg(d.path(), &["diff", "room", "--ours", "devprod", "--ref", "clean"]);
        assert_ne!(c, 0, "{s}");
        assert!(v["error"]["message"].as_str().unwrap_or("").contains("only within the same device"), "{s}");
    }
    #[test]
    fn diff_refuses_when_the_two_sides_were_captured_on_different_devices() {
        let d = root();
        // clean/room has 1440x900 and 390x844; devprod/room has no 390x844: the sides cannot be matched on the phone, and desktop-vs-phone is never picked silently
        let (c, _, s) = cg(d.path(), &["diff", "room", "--ours", "devprod", "--ref", "clean", "--viewport", "iphone"]);
        assert_ne!(c, 0, "{s}");
        // a file that says it is a tablet under a phone key is invalid evidence (device.class must match the capture, id must be the key)
        edit(&ev(d.path(), "room", "iphone-member"), |e| e["device"]["id"] = json!("android"));
        let (_, _, s) = cg(d.path(), &["screens", "devprod"]);
        assert!(s.contains("invalid_evidence") && s.contains("device.id"), "{s}");
    }
}

mod matrix {
    use super::*;
    fn with_family() -> tempfile::TempDir {
        let d = root();
        let mut csv = std::fs::read_to_string(d.path().join("ui/map.csv")).unwrap();
        for (row, screen) in [("r-room", "room"), ("r-inbox", "inbox"), ("r-settings", "settings"), ("r-flows", "flows-list")] {
            csv.push_str(&format!("devfam,{row},devprod,{screen},/{screen},,\n"));
        }
        std::fs::write(d.path().join("ui/map.csv"), csv).unwrap();
        d
    }
    #[test]
    fn devices_columns_carry_the_support_class_and_the_absent_cell_with_its_source() {
        let d = with_family();
        let v = clean(d.path(), &["matrix", "devfam", "--devices", "desktop-1440,ipad-portrait,iphone,android"]);
        let rows = v["data"]["items"].as_array().unwrap();
        let cell = |row: &str, dev: &str| rows.iter().find(|r| r["row"] == row).unwrap()["cells"]["devprod"][dev].as_str().unwrap().to_string();
        let sup = |row: &str, dev: &str| rows.iter().find(|r| r["row"] == row).unwrap()["support"]["devprod"][dev].as_str().unwrap().to_string();
        assert!(cell("r-room", "iphone").ends_with("/12 degraded"), "{}", cell("r-room", "iphone"));
        assert!(cell("r-room", "desktop-1440").ends_with("/12 full"));
        assert!(cell("r-inbox", "iphone").ends_with("/12 blocked"));
        assert!(cell("r-settings", "android").ends_with("/12 redirect"));
        assert_eq!(cell("r-inbox", "ipad-portrait"), "-", "nothing captured and not absent");
        assert_eq!(cell("r-flows", "iphone"), "n/a (absent)");
        assert_eq!(sup("r-flows", "desktop-1440"), "absent");
        let cols = &v["data"]["supportColumns"]["devprod"];
        assert_eq!(cols["iphone"]["degraded"], 1);
        assert_eq!(cols["iphone"]["blocked"], 1);
        assert_eq!(cols["iphone"]["absent"], 1);
        assert_eq!(cols["android"]["redirect"], 1);
        assert_eq!(v["data"]["devices"], json!(["desktop-1440", "ipad-portrait", "android", "iphone"]), "columns are ordered desktop, tablet, phone, then by id");
    }
    #[test]
    fn the_absent_cell_names_where_the_fact_comes_from_in_the_coverage_it_reads() {
        let d = with_family();
        let dir = d.path().join("ui/coverage");
        let mut found = false;
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let c = read(&e.path());
            for cell in c["cells"].as_array().unwrap() {
                if cell["support"] == "absent" {
                    found = true;
                    assert!(cell["why"].as_str().unwrap().starts_with("absent: ") && !cell["source"].as_str().unwrap().is_empty(), "{cell}");
                }
            }
        }
        assert!(found, "the contract fixture holds an absent cell");
        // cut the fact out of the coverage and the cell is `-`, not a silent absent
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            edit(&e.path(), |c| {
                for cell in c["cells"].as_array_mut().unwrap() {
                    if cell["support"] == "absent" {
                        cell["support"] = json!(null);
                    }
                }
            });
        }
        let v = clean(d.path(), &["matrix", "devfam", "--devices", "iphone"]);
        let r = v["data"]["items"].as_array().unwrap().iter().find(|r| r["row"] == "r-flows").unwrap();
        assert_eq!(r["cells"]["devprod"]["iphone"], "-");
    }
    #[test]
    fn devices_and_viewport_are_not_both_accepted() {
        let d = with_family();
        let (c, _, s) = cg(d.path(), &["matrix", "devfam", "--devices", "iphone", "--viewport", "iphone"]);
        assert_ne!(c, 0, "{s}");
    }
}

mod invalid {
    use super::*;
    #[test]
    fn a_malformed_device_or_support_block_is_invalid_evidence_never_silently_read() {
        for (what, f) in [
            ("support.class", (|e: &mut Value| e["support"]["class"] = json!("wobbly")) as fn(&mut Value)),
            ("support.target", |e: &mut Value| { e["support"] = json!({"class": "redirect", "reasons": []}); }),
            ("support.message", |e: &mut Value| { e["support"] = json!({"class": "blocked", "reasons": []}); }),
            ("device.class", |e: &mut Value| e["device"]["class"] = json!("watch")),
            ("device.size", |e: &mut Value| e["device"]["size"] = json!("wide")),
        ] {
            let d = root();
            edit(&ev(d.path(), "room", "iphone-member"), f);
            let (_, _, s) = cg(d.path(), &["screens", "devprod"]);
            assert!(s.contains("invalid_evidence") && s.contains(what), "{what}: {s}");
        }
        let d = root();
        edit(&ev(d.path(), "room", "desktop-1440-member"), |e| e["touch"] = json!({"minTargetPx": 44, "checked": 1, "failingCount": 0, "failing": [], "hoverOnlyCount": 0, "hoverOnly": [], "smallText": 0}));
        let (_, _, s) = cg(d.path(), &["screens", "devprod"]);
        assert!(s.contains("invalid_evidence") && s.contains("touch"), "a desktop device with touch data is invalid: {s}");
    }
    #[test]
    fn an_old_evidence_without_the_new_blocks_still_reads() {
        let d = root();
        edit(&ev(d.path(), "room", "iphone-member"), |e| {
            let o = e.as_object_mut().unwrap();
            o.remove("device");
            o.remove("support");
            o.remove("touch");
        });
        let v = clean(d.path(), &["show", "devprod", "room", "--viewport", "iphone"]);
        assert_eq!(metric(&v, "E1")["limit"], "=0", "the phone column still applies to a known device id when the file has no device block");
        assert!(v["data"]["support"].is_null() && v["data"]["deviceClass"].is_null(), "no block, nothing invented");
    }
}
