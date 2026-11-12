//! CMP-UI-1h / 1h2: the hardening of the UI evidence verbs, one module per FAILURE CLASS (REA lessons, CMP-UI-1-lanes section 16).
//! Every test here was seen red on the code before its fix. Roots are copies of the probe's own contract fixture, edited in temp dirs.
//! Classes: truncation, one_sided, drift, failed_nav, blank, integrity, owned_pass, unread, cursors, limits, hints, docs.
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
fn sha(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}
fn edit(p: &Path, f: impl Fn(&mut Value)) {
    let mut v = read(p);
    f(&mut v);
    write(p, &v);
}
/// A copy of the contract fixture holding ONLY the `product` run labelled `a`, with every input digest filled the same way, so two copies are comparable.
fn one_run_root(product: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    copy_dir(&fx("ui-contract"), d.path());
    for e in std::fs::read_dir(d.path().join("ui/runs")).unwrap().flatten() {
        let v = read(&e.path());
        if !(v["inputs"]["product"] == product && v["inputs"]["runLabel"] == "a") {
            std::fs::remove_file(e.path()).unwrap();
            let _ = std::fs::remove_file(d.path().join("ui/coverage").join(e.file_name()));
            continue;
        }
        let root = d.path().join("ui");
        edit(&e.path(), |v| {
            for k in ["registryDigest", "planDigest"] {
                if v["inputs"][k].is_null() {
                    v["inputs"][k] = json!("0".repeat(64));
                }
            }
            if v["inputs"].get("archetypesDigest").is_none() {
                v["inputs"]["archetypesDigest"] = Value::Null;
            }
            if v.get("evidenceSha256").is_none() {
                let m: serde_json::Map<String, Value> = v["captures"].as_object().unwrap().values().map(|f| (f.as_str().unwrap().to_string(), json!(sha(&std::fs::read(root.join(f.as_str().unwrap())).unwrap())))).collect();
                v["evidenceSha256"] = Value::Object(m);
            }
        });
    }
    d
}
fn run_file(root: &Path) -> PathBuf {
    std::fs::read_dir(root.join("ui/runs")).unwrap().flatten().next().unwrap().path()
}
fn verify(a: &Path, b: &Path, extra: &[&str]) -> (i32, Value) {
    let mut args = vec!["verify", "--root-a", a.to_str().unwrap(), "--root-b", b.to_str().unwrap()];
    args.extend_from_slice(extra);
    cg(a, &args)
}
fn limits_of(v: &Value) -> String {
    // an ok answer carries its limitations in evidence.limitations; a failed one keeps them in data.limitation_items
    format!("{} {}", v["evidence"]["limitations"], v["data"]["limitation_items"])
}

mod baseline {
    use super::*;
    #[test]
    fn two_equal_roots_verify_identical() {
        let (a, b) = (one_run_root("clean"), one_run_root("clean"));
        let (c, v) = verify(a.path(), b.path(), &[]);
        assert_eq!(c, 0, "{v}");
        assert_eq!(v["data"]["identical"], true);
        assert_eq!(v["data"]["compared"], 2);
    }
}

mod truncation {
    use super::*;
    #[test]
    fn verify_fails_when_it_compared_fewer_keys_than_both_runs_hold() {
        let (a, b) = (one_run_root("clean"), one_run_root("clean"));
        let (c, v) = verify(a.path(), b.path(), &["--max-nodes", "1"]);
        assert_eq!((c, v["error"]["code"].as_str()), (1, Some("check_failed")), "a partial comparison must not pass: {v}");
        assert_eq!(v["data"]["identical"], false);
        assert_eq!((v["data"]["compared"].as_u64(), v["data"]["total"].as_u64()), (Some(1), Some(2)), "{v}");
        assert!(limits_of(&v).contains("incomplete_comparison"), "{v}");
    }
}

mod one_sided {
    use super::*;
    #[test]
    fn a_key_on_one_side_only_fails_the_gate() {
        let (a, b) = (one_run_root("clean"), one_run_root("clean"));
        edit(&run_file(b.path()), |v| {
            let k = v["captures"].as_object().unwrap().keys().next().unwrap().clone();
            v["captures"].as_object_mut().unwrap().remove(&k);
        });
        let (c, v) = verify(a.path(), b.path(), &[]);
        assert_eq!((c, v["error"]["code"].as_str()), (1, Some("check_failed")), "{v}");
        assert_eq!(v["data"]["onlyInA"].as_array().unwrap().len(), 1, "{v}");
        assert_eq!(v["data"]["identical"], false);
    }
}

mod drift {
    use super::*;
    #[test]
    fn runs_with_different_inputs_are_refused_and_the_flag_is_stated_in_limitations() {
        let (a, b) = (one_run_root("clean"), one_run_root("clean"));
        edit(&run_file(b.path()), |v| v["inputs"]["probeVersion"] = json!("99"));
        let (c, v) = verify(a.path(), b.path(), &[]);
        assert_eq!((c, v["error"]["code"].as_str()), (1, Some("check_failed")), "{v}");
        assert!(v["data"]["inputDrift"].to_string().contains("probeVersion"), "{v}");
        assert!(v["error"]["hint"].as_str().unwrap().contains("--allow-input-drift"), "{v}");
        let (c, v) = verify(a.path(), b.path(), &["--allow-input-drift"]);
        assert_eq!(c, 0, "{v}");
        assert!(limits_of(&v).contains("input_drift_allowed") && limits_of(&v).contains("probeVersion"), "{v}");
    }
    #[test]
    fn runs_that_never_recorded_their_inputs_are_refused_too() {
        let (a, b) = (one_run_root("clean"), one_run_root("clean"));
        edit(&run_file(b.path()), |v| { v["inputs"].as_object_mut().unwrap().remove("fixtureHash"); });
        let (c, v) = verify(a.path(), b.path(), &[]);
        assert_eq!(c, 1, "{v}");
        assert!(v["data"]["inputDrift"].to_string().contains("fixtureHash"), "{v}");
    }
    #[test]
    fn diff_across_products_refuses_different_probe_versions() {
        let d = one_run_root("clean");
        let ev = d.path().join("ui/clean/room/populated/1440x900-ref.json");
        edit(&ev, |v| v["probe"]["version"] = json!("99"));
        let (c, v) = cg(d.path(), &["diff", "room", "--ours", "cluttered", "--ref", "clean", "--ui-root", "."]);
        assert_eq!((c, v["error"]["code"].as_str()), (1, Some("check_failed")), "{v}");
        let (c, v) = cg(d.path(), &["diff", "room", "--ours", "cluttered", "--ref", "clean", "--ui-root", ".", "--allow-input-drift"]);
        assert_eq!(c, 0, "{v}");
        assert!(limits_of(&v).contains("input_drift_allowed"), "{v}");
    }
    #[test]
    fn select_honours_locale_and_theme_instead_of_file_order() {
        let d = one_run_root("clean");
        let dir = d.path().join("ui/clean/room/populated");
        let mut de = read(&dir.join("1440x900-ref.json"));
        de["locale"] = json!("de-DE");
        de["key"] = json!("clean@contract-1/room/populated/1440x900/ref/de-DE/light");
        de["domHash"] = json!("d".repeat(64));
        write(&dir.join("1440x900-ref-de-DE-light.json"), &de);
        let (c, v) = cg(d.path(), &["show", "clean", "room", "--ui-root", "."]);
        assert_eq!(c, 0, "{v}");
        assert_eq!(v["data"]["locale"], "en-US", "the default is en-US/light, not whichever file sorts first: {v}");
        let (_, v) = cg(d.path(), &["show", "clean", "room", "--ui-root", ".", "--locale", "de-DE"]);
        assert_eq!(v["data"]["locale"], "de-DE", "{v}");
    }
}

mod failed_nav {
    use super::*;
    #[test]
    fn a_failed_cell_with_its_reason_is_counted_and_keeps_the_run_complete_only_when_explained() {
        let d = one_run_root("clean");
        let cov = std::fs::read_dir(d.path().join("ui/coverage")).unwrap().flatten().next().unwrap().path();
        edit(&cov, |v| {
            v["cells"][0]["status"] = json!("failed");
            v["cells"][0]["why"] = json!("navigation failed: HTTP 404 (status 404, final http://x/gone)");
        });
        let run = read(&cov)["runId"].as_str().unwrap().to_string();
        let (c, v) = cg(d.path(), &["coverage", &run, "--ui-root", "."]);
        assert_eq!(c, 0, "{v}");
        assert!(v["data"]["table"][0]["failed"].as_u64().unwrap() >= 1, "{v}");
        edit(&cov, |v| v["cells"][0]["why"] = json!(""));
        let (_, v) = cg(d.path(), &["coverage", &run, "--ui-root", "."]);
        assert_eq!(v["data"]["complete"], false, "a failed cell without a reason is blank: {v}");
    }
    #[test]
    fn an_unstable_capture_is_excluded_from_pass_counts_and_counted() {
        let d = one_run_root("clean");
        edit(&d.path().join("ui/clean/room/populated/1440x900-ref.json"), |v| v["unstable"] = json!(true));
        let (c, v) = cg(d.path(), &["screens", "clean", "--ui-root", "."]);
        assert_eq!(c, 0, "{v}");
        let (_, phone) = cg(d.path(), &["show", "clean", "room", "--ui-root", ".", "--viewport", "390x844"]);
        assert_eq!(v["data"]["items"][0]["bestPass"], phone["data"]["pass"], "only the stable 390x844 capture counts: {v}");
        assert!(limits_of(&cg(d.path(), &["screens", "clean", "--ui-root", "."]).1).contains("unstable_excluded"), "the exclusion is counted");
    }
    #[test]
    fn verify_does_not_pass_on_an_unstable_capture() {
        let (a, b) = (one_run_root("clean"), one_run_root("clean"));
        edit(&b.path().join("ui/clean/room/populated/1440x900-ref.json"), |v| v["unstable"] = json!(true));
        // the evidence file changed: refresh its recorded digest so only the unstable rule is under test
        let rf = run_file(b.path());
        edit(&rf, |v| {
            let f = "clean/room/populated/1440x900-ref.json";
            v["evidenceSha256"][f] = json!(sha(&std::fs::read(b.path().join("ui").join(f)).unwrap()));
        });
        let (c, v) = verify(a.path(), b.path(), &[]);
        assert_eq!((c, v["error"]["code"].as_str()), (1, Some("check_failed")), "{v}");
        assert_eq!(v["data"]["unstable"].as_array().unwrap().len(), 1, "{v}");
    }
}

mod blank {
    use super::*;
    #[test]
    fn a_blank_cell_makes_the_run_incomplete_and_an_explained_one_does_not() {
        let d = one_run_root("clean");
        let cov = std::fs::read_dir(d.path().join("ui/coverage")).unwrap().flatten().next().unwrap().path();
        let run = read(&cov)["runId"].as_str().unwrap().to_string();
        edit(&cov, |v| {
            v["cells"][0]["status"] = json!("blank");
            v["cells"][0]["why"] = json!("not in this run's plan and no n/a reason given");
        });
        let (_, v) = cg(d.path(), &["coverage", &run, "--ui-root", "."]);
        assert_eq!(v["data"]["complete"], false, "{v}");
        assert!(v["data"]["table"][0]["blank"].as_u64().unwrap() >= 1, "{v}");
        edit(&cov, |v| {
            v["cells"][0]["status"] = json!("n/a");
            v["cells"][0]["why"] = json!("the product has no such screen (registry)");
        });
        let (_, v) = cg(d.path(), &["coverage", &run, "--ui-root", "."]);
        assert_eq!(v["data"]["complete"], true, "the guard flips with the cell: {v}");
    }
}

mod integrity {
    use super::*;
    #[test]
    fn verify_integrity_rehashes_blobs_and_evidence() {
        let (a, b) = (one_run_root("clean"), one_run_root("clean"));
        let (c, v) = verify(a.path(), b.path(), &["--integrity"]);
        assert_eq!(c, 0, "{v}");
        assert_eq!(v["data"]["integrity"]["mismatches"].as_array().unwrap().len(), 0);
        assert!(v["data"]["integrity"]["checked"].as_u64().unwrap() >= 4, "two evidence files and two blobs per side: {v}");
    }
    #[test]
    fn a_tampered_blob_or_evidence_file_is_an_integrity_mismatch() {
        let (a, b) = (one_run_root("clean"), one_run_root("clean"));
        let ev = read(&b.path().join("ui/clean/room/populated/1440x900-ref.json"));
        let blob = b.path().join("ui").join(ev["image"]["file"].as_str().unwrap());
        let mut bytes = std::fs::read(&blob).unwrap();
        bytes.push(0);
        std::fs::write(&blob, bytes).unwrap();
        let (c, v) = verify(a.path(), b.path(), &["--integrity"]);
        assert_eq!((c, v["error"]["code"].as_str()), (1, Some("check_failed")), "{v}");
        assert!(limits_of(&v).contains("integrity_mismatch") || v["data"]["limitation_items"].to_string().contains("integrity_mismatch"), "{v}");
        assert!(v["data"]["integrity"]["mismatches"][0]["file"].as_str().unwrap().contains("blobs/"), "{v}");
        // without the flag the same roots are not re-hashed
        let (c, _) = verify(a.path(), b.path(), &[]);
        assert_eq!(c, 0);
        // evidence tampering is caught by the digest the run recorded
        let (a, b) = (one_run_root("clean"), one_run_root("clean"));
        edit(&b.path().join("ui/clean/room/populated/390x844-ref.json"), |v| v["meta"]["capturedAt"] = json!("2030-01-01T00:00:00.000Z"));
        let (c, v) = verify(a.path(), b.path(), &["--integrity"]);
        assert_eq!(c, 1, "{v}");
        assert!(v["data"]["integrity"]["mismatches"].to_string().contains("390x844-ref.json"), "{v}");
    }
}

mod owned_pass {
    use super::*;
    fn metric<'a>(v: &'a Value, id: &str) -> &'a Value {
        v["data"]["metrics"].as_array().unwrap().iter().find(|m| m["id"] == id).unwrap()
    }
    #[test]
    fn pass_is_recomputed_from_the_registry_limits_and_the_probes_pass_is_advisory() {
        let d = one_run_root("cluttered");
        let (c, v) = cg(d.path(), &["show", "cluttered", "room", "--ui-root", "."]);
        assert_eq!(c, 0, "{v}");
        let (e1, before) = (metric(&v, "E1").clone(), v["data"]["pass"].as_u64().unwrap());
        assert_eq!((e1["pass"].clone(), e1["limitSource"].clone(), e1["limit"].clone()), (json!(false), json!("registry"), json!("<=2")), "{e1}");
        // loosen the registry: E1 (navigation columns before content) may now be 5 on desktop; the page has 3, the probe's own limit said 2
        edit(&d.path().join("ui/archetypes.json"), |a| a["archetypes"]["conversation"]["limits"]["E1"]["desktop"] = json!("<=5"));
        let (_, v) = cg(d.path(), &["show", "cluttered", "room", "--ui-root", "."]);
        let e1 = metric(&v, "E1");
        assert_eq!(e1["pass"], true, "the engine owns pass: {e1}");
        assert_eq!(e1["probePass"], false, "the probe's verdict is shown, advisory: {e1}");
        assert_eq!(v["data"]["pass"].as_u64().unwrap(), before + 1, "{v}");
    }
    #[test]
    fn the_phone_column_applies_to_a_phone_viewport() {
        let d = one_run_root("cluttered");
        edit(&d.path().join("ui/archetypes.json"), |a| a["archetypes"]["conversation"]["limits"]["E1"]["phone"] = json!("<=9"));
        let (_, v) = cg(d.path(), &["show", "cluttered", "room", "--ui-root", ".", "--viewport", "390x844"]);
        let e1 = metric(&v, "E1");
        assert_eq!((e1["limit"].clone(), e1["limitSource"].clone()), (json!("<=9"), json!("registry")), "{e1}");
        let (_, v) = cg(d.path(), &["show", "cluttered", "room", "--ui-root", "."]);
        assert_eq!(metric(&v, "E1")["limit"], "<=2", "desktop keeps its own column");
    }
    #[test]
    fn a_reference_seat_is_never_gated_and_says_so() {
        let d = one_run_root("clean");
        edit(&d.path().join("ui/archetypes.json"), |a| a["archetypes"]["conversation"]["limits"]["E1"]["desktop"] = json!("=0"));
        let (_, v) = cg(d.path(), &["show", "clean", "room", "--ui-root", "."]);
        let e1 = metric(&v, "E1");
        assert_eq!((e1["pass"].clone(), e1["limitSource"].clone()), (json!(true), json!("reference-seat")), "{e1}");
    }
}

mod unread {
    use super::*;
    #[test]
    fn matrix_marks_a_product_it_did_not_read_with_a_question_mark_not_a_dash() {
        let d = one_run_root("clean");
        let fam = d.path().join("ui/map.csv");
        std::fs::write(&fam, "family,row_id,product,screen,route,task,note\nfx,r1,clean,room,/r,,\nfx,r1,cluttered,room,/r,,\n").unwrap();
        // the cap is spent on the first product: `clean` has 2 evidence files, so `cluttered` is never read
        let (c, v) = cg(d.path(), &["matrix", "fx", "--ui-root", ".", "--max-nodes", "2"]);
        assert_eq!(c, 0, "{v}");
        assert_eq!(v["data"]["items"][0]["cells"]["cluttered"], "?", "unread is not absent: {v}");
        assert!(limits_of(&v).contains("products_unread") && limits_of(&v).contains("cluttered"), "{v}");
        let (_, v) = cg(d.path(), &["matrix", "fx", "--ui-root", "."]);
        assert_ne!(v["data"]["items"][0]["cells"]["cluttered"], "?");
    }
}

mod cursors {
    use super::*;
    #[test]
    fn list_verbs_page_with_a_real_cursor() {
        let d = one_run_root("clean");
        let (_, v) = cg(d.path(), &["products", "--ui-root", ".", "--page-size", "1"]);
        assert_eq!(v["data"]["items"].as_array().unwrap().len(), 1, "{v}");
        assert_eq!(v["page"]["next"], "1", "{v}");
        assert_eq!(v["page"]["total"], 3);
        let (_, v) = cg(d.path(), &["products", "--ui-root", ".", "--page-size", "1", "--cursor", "1"]);
        assert_eq!(v["data"]["items"][0]["product"], "cluttered", "{v}");
        assert_eq!(v["page"]["next"], "2", "{v}");
        let (_, v) = cg(d.path(), &["products", "--ui-root", ".", "--page-size", "1", "--cursor", "2"]);
        assert_eq!(v["data"]["items"][0]["product"], "devprod", "{v}");
        assert!(v["page"]["next"].is_null());
        let (c, v) = cg(d.path(), &["products", "--ui-root", ".", "--cursor", "banana"]);
        assert_eq!((c, v["error"]["code"].as_str()), (1, Some("invalid_arguments")), "{v}");
        let (_, v) = cg(d.path(), &["screens", "cluttered", "--ui-root", ".", "--page-size", "1"]);
        assert_eq!(v["page"]["next"], "1", "{v}");
    }
}

mod limits {
    use super::*;
    #[test]
    fn ui_limits_prints_the_one_catalogue() {
        let d = one_run_root("clean");
        let (c, v) = cg(d.path(), &["limits"]);
        assert_eq!(c, 0, "{v}");
        let names: Vec<&str> = v["data"]["items"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap()).collect();
        for n in ["MAX_NODES", "DEFAULT_MAX_NODES", "MAX_FILE", "MAX_INVALID_LISTED", "MAX_BLANK_LISTED", "MAX_GRAPH_NODES", "DEFAULT_MAX_LEN", "MAX_LEN", "DEFAULT_PAGE_SIZE", "MAX_PAGE_SIZE"] {
            assert!(names.contains(&n), "limits catalogue lacks {n}: {names:?}");
        }
        assert!(v["data"]["items"][0]["unit"].is_string() && v["data"]["items"][0]["whenHit"].is_string());
    }
    #[test]
    fn every_numeric_cap_const_in_the_ui_modules_is_in_the_catalogue() {
        let cat = include_str!("../src/application/ui_limits.rs");
        let mut missing = vec![];
        for (file, src) in [
            ("application/ui.rs", include_str!("../src/application/ui.rs")),
            ("application/ui_graph.rs", include_str!("../src/application/ui_graph.rs")),
            ("application/ui_limits.rs", cat),
            ("domain/ui.rs", include_str!("../src/domain/ui.rs")),
            ("domain/ui_nav.rs", include_str!("../src/domain/ui_nav.rs")),
            ("domain/ui_graph.rs", include_str!("../src/domain/ui_graph.rs")),
        ] {
            for l in src.lines() {
                let t = l.trim_start().trim_start_matches("pub ").trim_start_matches("pub(crate) ");
                if let Some(rest) = t.strip_prefix("const ") {
                    if let Some((name, ty)) = rest.split_once(':') {
                        let ty = ty.trim_start();
                        if ["usize", "u64", "u32", "u16", "i64"].iter().any(|n| ty.starts_with(n)) && !cat.contains(&format!("\"{}\"", name.trim())) {
                            missing.push(format!("{file}: {}", name.trim()));
                        }
                    }
                }
            }
        }
        assert!(missing.is_empty(), "caps not registered in application/ui_limits.rs: {missing:?}");
    }
}

mod hints {
    use super::*;
    #[test]
    fn not_found_carries_candidates_and_a_recovery_hint() {
        let d = one_run_root("clean");
        let (c, v) = cg(d.path(), &["show", "clean", "nowhere", "--ui-root", "."]);
        assert_eq!((c, v["error"]["code"].as_str()), (1, Some("not_found")), "{v}");
        assert_eq!(v["error"]["data"]["selector"], "nowhere");
        assert_eq!(v["error"]["data"]["candidates"], json!(["room"]), "{v}");
        assert!(v["error"]["hint"].as_str().unwrap().contains("ui screens"), "{v}");
        let (_, v) = cg(d.path(), &["screens", "nope", "--ui-root", "."]);
        assert!(v["error"]["data"]["candidates"].as_array().unwrap().iter().any(|p| p == "clean"), "{v}");
    }
    #[test]
    fn a_truncation_limitation_names_the_control_and_says_when_raising_is_impossible() {
        let d = one_run_root("clean");
        let (_, v) = cg(d.path(), &["products", "--ui-root", ".", "--max-nodes", "1"]);
        let l = v["evidence"]["limitations"].as_array().unwrap().iter().find(|l| l["code"] == "truncated").unwrap().clone();
        assert!(l["detail"].as_str().unwrap().contains("--max-nodes 1"), "{l}");
        assert_eq!(l["data"]["control"], "max_nodes", "{l}");
        assert_eq!(l["data"]["max"], 2000);
        let (_, v) = cg(d.path(), &["products", "--ui-root", ".", "--max-nodes", "2000"]);
        assert!(limits_of(&v).contains("maximum 2000") || !limits_of(&v).contains("truncated"), "{v}");
        assert!(!limits_of(&v).contains("raise --max-nodes"), "never advise a raise that cannot be made: {v}");
    }
}

mod docs {
    #[test]
    fn mcp_tools_md_states_the_real_tool_count_and_lists_every_tool() {
        let md = include_str!("../../../docs/mcp-tools.md");
        let cat: serde_json::Value = serde_json::from_str(include_str!("../../../docs/catalog.json")).unwrap();
        let tools: Vec<&str> = cat["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        let stated: Vec<usize> = md.lines().filter_map(|l| l.split(" tools").next().and_then(|h| h.rsplit(' ').next()).and_then(|w| w.parse().ok())).collect();
        assert!(stated.contains(&tools.len()), "docs/mcp-tools.md must state `{} tools` (found {stated:?})", tools.len());
        let missing: Vec<&&str> = tools.iter().filter(|t| !md.contains(&format!("`{t}`"))).collect();
        assert!(missing.is_empty(), "tools missing from docs/mcp-tools.md: {missing:?}");
    }
    #[test]
    fn error_codes_in_the_catalog_match_the_codes_the_code_can_return() {
        let src = include_str!("../src/application/mod.rs");
        let body = src.split("pub fn code(&self)").nth(1).unwrap().split("pub fn hint").next().unwrap();
        let mut used: Vec<String> = body.split('"').skip(1).step_by(2).map(String::from).collect();
        used.push("catalog_stale".into());
        // and every code an `AppError::structured("<code>", ...)` in the source can carry
        let re = regex::Regex::new(r#"structured\(\s*"([a-z_]+)""#).unwrap();
        fn files(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    files(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push(p);
                }
            }
        }
        let mut srcs = vec![];
        files(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut srcs);
        for f in srcs {
            let t = std::fs::read_to_string(f).unwrap();
            used.extend(re.captures_iter(&t).map(|c| c[1].to_string()));
        }
        used.sort();
        used.dedup();
        let cat: serde_json::Value = serde_json::from_str(include_str!("../../../docs/catalog.json")).unwrap();
        let mut listed: Vec<String> = cat["error_codes"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect();
        listed.sort();
        assert_eq!(used, listed, "AppError::code() and ERROR_CODES / catalog.json disagree");
    }
}
