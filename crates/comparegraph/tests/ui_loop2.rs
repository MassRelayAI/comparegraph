//! CMP-UI-LOOP iteration 2: engine fixes found by comparing the second real sweep (run roots that carry registry.json / archetypes.json beside ui/).
//! Every test was seen red on the code before its fix.
use assert_cmd::Command;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
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
fn sha(p: &Path) -> String {
    Sha256::digest(std::fs::read(p).unwrap()).iter().map(|b| format!("{b:02x}")).collect()
}
fn lim_codes(v: &Value) -> Vec<String> {
    v["evidence"]["limitations"].as_array().map(|a| a.iter().filter_map(|x| x["code"].as_str().map(String::from)).collect()).unwrap_or_default()
}
const ROOM: &str = "ui/cluttered/room/populated/1440x900-member.json";
fn sources(v: &Value) -> Vec<String> {
    v["data"]["metrics"].as_array().unwrap().iter().map(|m| m["limitSource"].as_str().unwrap().to_string()).collect()
}

/// The slice shape: registry.json / archetypes.json sit in the run root BESIDE ui/, the runs record their digests.
fn sweep_shape(d: &Path, digests_match: bool) {
    for f in ["registry.json", "archetypes.json"] {
        std::fs::rename(d.join("ui").join(f), d.join(f)).unwrap();
    }
    let (r, a) = if digests_match { (sha(&d.join("registry.json")), sha(&d.join("archetypes.json"))) } else { ("0".repeat(64), "1".repeat(64)) };
    for e in std::fs::read_dir(d.join("ui/runs")).unwrap().flatten() {
        edit(&e.path(), |v| {
            v["inputs"]["registryDigest"] = json!(r);
            v["inputs"]["archetypesDigest"] = json!(a);
        });
    }
}

mod limits_from_the_run_root {
    use super::*;
    #[test]
    fn registry_beside_ui_is_read_when_the_digests_match() {
        let d = root();
        let (_, before) = cg(d.path(), &["show", "cluttered", "room"]);
        assert!(sources(&before).iter().any(|s| s.starts_with("registry:")), "control: the fixture reads ui/registry.json");
        sweep_shape(d.path(), true);
        let (c, v) = cg(d.path(), &["show", "cluttered", "room"]);
        assert_eq!(c, 0, "{v}");
        assert!(sources(&v).iter().any(|s| s.starts_with("registry:")), "{:?}", sources(&v));
        assert!(!lim_codes(&v).contains(&"registry_missing".to_string()), "{:?}", lim_codes(&v));
    }
    #[test]
    fn a_digest_mismatch_uses_no_registry_and_says_so() {
        let d = root();
        sweep_shape(d.path(), false);
        let (c, v) = cg(d.path(), &["show", "cluttered", "room"]);
        assert_eq!(c, 0, "{v}");
        assert!(sources(&v).iter().all(|s| !s.starts_with("registry:")), "{:?}", sources(&v));
        assert!(lim_codes(&v).contains(&"registry_digest_mismatch".to_string()), "{:?}", lim_codes(&v));
        assert!(lim_codes(&v).contains(&"archetypes_digest_mismatch".to_string()), "{:?}", lim_codes(&v));
    }
}

mod blocking_dialog {
    use super::*;
    fn block(d: &Path) {
        edit(&d.join(ROOM), |e| e["blocking"] = json!({ "label": "cookie consent", "role": "dialog" }));
    }
    #[test]
    fn show_says_blocked_and_scores_nothing() {
        let d = root();
        block(d.path());
        let (c, v) = cg(d.path(), &["show", "cluttered", "room", "--viewport", "1440x900"]);
        assert_eq!(c, 0, "{v}");
        assert_eq!(v["data"]["blockedLine"], "blocked by dialog cookie consent", "{v}");
        assert_eq!(v["data"]["pass"], Value::Null, "{v}");
        assert!(v["data"]["metrics"].as_array().unwrap().is_empty(), "{v}");
    }
    #[test]
    fn diff_refuses_a_blocked_side() {
        let d = root();
        block(d.path());
        let (c, v) = cg(d.path(), &["diff", "room", "--ours", "cluttered", "--ref", "clean", "--viewport", "1440x900"]);
        assert_ne!(c, 0, "{v}");
        assert!(v.to_string().contains("blocked by dialog cookie consent"), "{v}");
    }
    #[test]
    fn matrix_cell_says_blocked_instead_of_a_score() {
        let d = root();
        std::fs::write(d.path().join("ui/map.csv"), "family,row_id,product,screen,route,task,note\nfam,r1,cluttered,room,/room,,\n").unwrap();
        let (_, base) = cg(d.path(), &["matrix", "fam"]);
        assert!(base.to_string().contains("/14") || base.to_string().contains("/12"), "control: a score cell: {base}");
        block(d.path());
        let (c, v) = cg(d.path(), &["matrix", "fam"]);
        assert_eq!(c, 0, "{v}");
        let s = v.to_string();
        assert!(s.contains("blocked by dialog cookie consent"), "{s}");
    }
}

mod matrix_map {
    use super::*;
    #[test]
    fn map_csv_in_the_run_root_is_found_and_an_unknown_family_lists_the_names() {
        let d = root();
        std::fs::rename(d.path().join("ui/map.csv"), d.path().join("map.csv")).unwrap();
        let (c, v) = cg(d.path(), &["matrix", "nosuchfamily"]);
        assert_ne!(c, 0, "{v}");
        let s = v.to_string();
        assert!(s.contains("budgets") && s.contains("surfaces"), "the error lists the family names from the run root map.csv: {s}");
    }
    #[test]
    fn map_flag_names_the_file() {
        let d = root();
        std::fs::rename(d.path().join("ui/map.csv"), d.path().join("elsewhere.csv")).unwrap();
        let (c, v) = cg(d.path(), &["matrix", "nosuchfamily", "--map", "elsewhere.csv"]);
        assert_ne!(c, 0, "{v}");
        assert!(v.to_string().contains("budgets"), "{v}");
        let (_, v2) = cg(d.path(), &["matrix", "nosuchfamily"]);
        assert!(!v2.to_string().contains("budgets"), "without --map or a map.csv there are no names to list: {v2}");
    }
}

mod nav_inferred {
    use super::*;
    fn navfile(d: &Path) -> PathBuf {
        std::fs::read_dir(d.join("ui/cluttered/nav")).unwrap().flatten().next().unwrap().path()
    }
    #[test]
    fn a_heuristic_nav_is_a_limitation_in_nav_and_nav_diff() {
        let d = root();
        edit(&navfile(d.path()), |n| n["navSource"] = json!("heuristic"));
        let (c, v) = cg(d.path(), &["nav", "cluttered"]);
        assert_eq!(c, 0, "{v}");
        assert!(lim_codes(&v).contains(&"nav_inferred".to_string()), "{:?}", lim_codes(&v));
        let (_, w) = cg(d.path(), &["nav-diff", "--ours", "cluttered", "--ref", "clean"]);
        assert!(lim_codes(&w).contains(&"nav_inferred".to_string()), "{:?}", lim_codes(&w));
        let (_, k) = cg(d.path(), &["nav", "clean"]);
        assert!(!lim_codes(&k).contains(&"nav_inferred".to_string()));
    }
}
