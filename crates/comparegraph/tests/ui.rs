//! `comparegraph ui ...`: golden outputs over fixtures/ui (two products: `cluttered` and `clean`), plus hand-written assertions
//! on the numbers so a golden can never bless a wrong answer. Regenerate goldens with UPDATE_GOLDEN=1 and READ THE DIFF.
use assert_cmd::Command;
use serde_json::{json, Value};
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/ui")
}
/// Run from the fixture dir with `--ui-root .` so printed image paths are the same on every machine.
fn run_in(dir: &std::path::Path, args: &[&str]) -> (i32, String) {
    let out = Command::cargo_bin("comparegraph").unwrap().current_dir(dir).arg("--pretty").arg("ui").args(args).arg("--ui-root").arg(".").output().unwrap();
    (out.status.code().unwrap(), String::from_utf8(out.stdout).unwrap())
}
fn ui(args: &[&str]) -> (i32, Value) {
    let (c, s) = run_in(&fixtures(), args);
    (c, serde_json::from_str(&s).unwrap_or_else(|e| panic!("not json ({e}): {s}")))
}
fn golden(name: &str, args: &[&str], want_code: i32) -> Value {
    let (code, text) = run_in(&fixtures(), args);
    assert_eq!(code, want_code, "exit code of {args:?}: {text}");
    let again = run_in(&fixtures(), args).1;
    assert_eq!(text, again, "determinism: {args:?} must print byte-identical output twice");
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/ui").join(format!("{name}.json"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, &text).unwrap();
    }
    let want = std::fs::read_to_string(&p).unwrap_or_else(|_| panic!("missing golden {}", p.display()));
    assert_eq!(text, want, "golden {name}");
    serde_json::from_str(&text).unwrap()
}
fn words(v: &Value) -> Vec<&str> {
    v.as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect()
}

#[test]
fn products_lists_both_fixtures_and_what_each_has() {
    let v = golden("products", &["products"], 0);
    let items = v["data"]["items"].as_array().unwrap();
    let names: Vec<&str> = items.iter().map(|i| i["product"].as_str().unwrap()).collect();
    assert_eq!(names, ["clean", "cluttered"]);
    assert_eq!(items[1]["evidenceFiles"], 4);
    assert_eq!(items[1]["states"], json!(["empty", "populated"]));
    assert_eq!(items[1]["system"], true);
    assert_eq!(items[0]["system"], false);
    assert_eq!(v["verb"], "ui-products");
}

#[test]
fn screens_shows_archetype_and_best_pass() {
    let v = golden("screens-cluttered", &["screens", "cluttered"], 0);
    let items = v["data"]["items"].as_array().unwrap();
    assert_eq!(items[0]["screen"], "inbox");
    assert_eq!(items[0]["archetype"], "inbox");
    assert_eq!(items[1]["screen"], "room");
    assert_eq!(items[1]["archetype"], "conversation");
    assert_eq!(items[1]["bestPass"], 7); // the 390x844 capture passes 7
}

#[test]
fn show_gives_metrics_pass_fail_layout_style_and_image() {
    let v = golden("show-cluttered-room", &["show", "cluttered", "room"], 0);
    let d = &v["data"];
    assert_eq!((d["pass"].as_u64(), d["of"].as_u64()), (Some(5), Some(12)));
    assert_eq!(d["viewport"], "1440x900");
    assert_eq!(d["archetype"], "conversation");
    assert_eq!(d["metrics"][0]["id"], "E1");
    assert_eq!(d["metrics"][0]["pass"], false);
    assert_eq!(d["metrics"][0]["archetypeLimit"], 2);
    assert_eq!(d["metrics"][11]["id"], "E12");
    assert_eq!(d["layout"]["navColumns"], 3);
    assert!(d["layout"]["summary"].as_str().unwrap().contains("no primary"));
    assert!(d["style"]["summary"].as_str().unwrap().starts_with("type sizes 11, weights 2, colours 23"));
    assert!(d["image"]["path"].as_str().unwrap().starts_with("./ui/blobs/"));
    assert_eq!(d["image"]["present"], true);
    assert!(v["evidence"]["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "snapshot_recompute_pending"));
    golden("show-cluttered-room-mobile", &["show", "cluttered", "room", "--viewport", "390x844"], 0);
    golden("show-cluttered-room-empty", &["show", "cluttered", "room", "--state", "empty"], 0);
}

#[test]
fn diff_reads_in_words_and_compares_only_the_same_screen_state_viewport() {
    let v = golden("diff-room", &["diff", "room", "--ours", "cluttered", "--ref", "clean"], 0);
    let d = &v["data"];
    assert_eq!(words(&d["layoutDiff"]), ["nav columns 3 vs 1", "primaries 0 vs 1", "topbar regions 0 vs 1", "content start y 352 vs 120"]);
    let style = words(&d["styleDiff"]);
    for w in ["type sizes 11 vs 6", "colours 23 vs 12", "onScale4 0.71 vs 0.96", "shadows 4 vs 1"] {
        assert!(style.contains(&w), "{w} in {style:?}");
    }
    assert_eq!(d["pass"], json!({ "ours": 5, "ref": 11, "of": 12 }));
    assert!(d["images"]["ours"].as_str().unwrap().ends_with(".png") && d["images"]["ref"].as_str().unwrap().ends_with(".png"));
    assert_eq!(d["metrics"][9]["id"], "E10");
    assert_eq!(d["metrics"][9]["ref"]["pass"], false);
    // a map.csv row id resolves to the screen of each product
    let by_row = ui(&["diff", "surfaces.room", "--ours", "cluttered", "--ref", "clean"]).1;
    assert_eq!(by_row["data"]["screen"], "room");
    assert_eq!(by_row["data"]["layoutDiff"], d["layoutDiff"]);
    // refusals: another state, another viewport, a screen one side lacks
    let (c, e) = ui(&["diff", "room", "--ours", "cluttered", "--ref", "clean", "--state", "empty"]);
    assert_eq!((c, e["error"]["code"].as_str()), (1, Some("not_found")));
    assert!(e["error"]["message"].as_str().unwrap().contains("has no state `empty`"));
    let (c, e) = ui(&["diff", "room", "--ours", "cluttered", "--ref", "clean", "--viewport", "390x844"]);
    assert_eq!((c, e["error"]["code"].as_str()), (1, Some("not_found")));
    let (c, e) = ui(&["diff", "inbox", "--ours", "cluttered", "--ref", "clean"]);
    assert_eq!((c, e["error"]["code"].as_str()), (1, Some("not_found")));
}

#[test]
fn matrix_cells_are_pass_over_12_or_a_dash() {
    let v = golden("matrix-surfaces", &["matrix", "surfaces"], 0);
    let d = &v["data"];
    assert_eq!(d["products"], json!(["clean", "cluttered"]));
    let rows = d["items"].as_array().unwrap();
    let row = |id: &str| rows.iter().find(|r| r["row"] == id).unwrap()["cells"].clone();
    assert_eq!(row("surfaces.room"), json!({ "clean": "11/12", "cluttered": "5/12" }));
    assert_eq!(row("surfaces.inbox"), json!({ "clean": "-", "cluttered": "5/12" }));
    assert_eq!(row("surfaces.router-overview"), json!({ "clean": "-", "cluttered": "-" }));
    let e = ui(&["matrix", "surfaces", "--state", "empty"]).1;
    assert_eq!(e["data"]["items"].as_array().unwrap().iter().find(|r| r["row"] == "surfaces.room").unwrap()["cells"], json!({ "clean": "-", "cluttered": "4/12" }));
}

#[test]
fn system_reads_system_json_or_derives_the_union() {
    let v = golden("system-cluttered", &["system", "cluttered"], 0);
    assert_eq!(v["data"]["source"], "system.json");
    assert_eq!(v["data"]["targets"][0], json!({ "target": "type sizes", "value": "11", "limit": "<= 6", "pass": false }));
    let c = golden("system-clean", &["system", "clean"], 0);
    assert_eq!(c["data"]["source"], "derived");
    assert_eq!(c["data"]["targets"][0]["pass"], true);
    assert!(c["evidence"]["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "system_derived"));
}

#[test]
fn flow_lists_steps_clicks_and_dead_ends() {
    let v = golden("flow-cluttered", &["flow", "cluttered", "create-workflow"], 0);
    assert_eq!((v["data"]["stepCount"].as_u64(), v["data"]["clicks"].as_u64()), (Some(6), Some(5)));
    assert_eq!(v["data"]["deadEnds"].as_array().unwrap().len(), 2);
    let c = ui(&["flow", "clean", "create-workflow"]).1;
    assert_eq!(c["data"]["stepCount"], 3);
    assert_eq!(ui(&["flow", "clean", "nope"]).1["error"]["code"], "not_found");
}

#[test]
fn nav_derives_the_numbers_and_nav_diff_says_them_in_words() {
    let v = golden("nav-cluttered", &["nav", "cluttered"], 0);
    let s = &v["data"]["stats"];
    assert_eq!((s["places"].as_u64(), s["maxDepth"].as_u64(), s["nodes"].as_u64()), (Some(4), Some(2), Some(9)));
    assert_eq!(s["itemsPerLevel"], json!({ "1": 4, "2": 5 }));
    assert_eq!(s["itemsPerGroup"], json!({ "Work": 3 }));
    assert_eq!(
        (s["destinations"].as_u64(), s["duplicateDestinations"].as_u64(), s["deadItems"].as_u64(), s["overflowOnlyItems"].as_u64()),
        (Some(5), Some(1), Some(2), Some(1))
    );
    assert_eq!(v["data"]["tree"][0]["label"], "Home");
    let d = golden("nav-diff", &["nav-diff", "--ours", "cluttered", "--ref", "clean"], 0);
    assert_eq!(
        words(&d["data"]["diff"]),
        [
            "places 4 vs 3", "max depth 2 vs 1", "nodes 9 vs 3", "items at level 1 4 vs 3", "items at level 2 5 vs 0", "destinations 5 vs 3",
            "duplicate destinations 1 vs 0", "dead items 2 vs 0", "items only via overflow 1 vs 0", "screens only in ours: billing, flows"
        ]
    );
    assert_eq!(ui(&["nav", "cluttered", "--viewport", "390x844"]).1["error"]["code"], "not_found");
}

#[test]
fn journey_reports_completed_partial_and_na_with_the_reason() {
    let v = golden("journey-j1", &["journey", "j1"], 0);
    let rows = v["data"]["items"].as_array().unwrap();
    let line = |p: &str| rows.iter().find(|r| r["product"] == p).unwrap()["outcome"].as_str().unwrap().to_string();
    assert_eq!(line("clean"), "completed");
    assert_eq!(line("cluttered"), "partial at step 3: the first room cannot be created without an admin");
    assert_eq!(line("legacy"), "n/a: no rooms");
    let only = ui(&["journey", "j1", "--products", "clean,cluttered,ghost"]).1;
    assert_eq!(only["data"]["items"].as_array().unwrap().len(), 2);
    assert!(only["evidence"]["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "no_result"));
    assert_eq!(ui(&["journey", "j9"]).1["error"]["code"], "not_found");
}

#[test]
fn coverage_is_incomplete_when_a_cell_is_blank() {
    let ok = golden("coverage-run-a", &["coverage", "run-a"], 0);
    assert_eq!(ok["data"]["complete"], true);
    let bad = golden("coverage-run-c", &["coverage", "run-c"], 0);
    assert_eq!(bad["data"]["complete"], false);
    assert_eq!(bad["data"]["blankCells"], json!(["clean/inbox/populated/1440x900-ref", "clean/room/populated/1440x900-ref"]));
    assert!(bad["evidence"]["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "incomplete_run"));
}

#[test]
fn verify_is_the_determinism_gate() {
    let same = golden("verify-a-b", &["verify", "run-a", "run-b"], 0);
    assert_eq!((same["data"]["identical"].clone(), same["data"]["compared"].clone()), (json!(true), json!(3)));
    let v = golden("verify-a-c", &["verify", "run-a", "run-c"], 1);
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "check_failed");
    let d = &v["data"];
    assert_eq!(d["compared"], 2);
    let kinds: Vec<(&str, &str)> = d["differences"].as_array().unwrap().iter().map(|x| (x["kind"].as_str().unwrap(), x["metric"].as_str().unwrap_or(""))).collect();
    assert_eq!(kinds, [("domHash", ""), ("metric", "E5"), ("image", "")]);
    assert_eq!(d["differences"][2]["distance"], 64);
    assert_eq!(d["onlyInA"].as_array().unwrap().len(), 1);
    assert_eq!(d["onlyInB"].as_array().unwrap().len(), 1);
}

#[test]
fn bounded_and_honest_about_what_it_skipped() {
    let t = ui(&["products", "--max-nodes", "3"]).1;
    assert!(t["evidence"]["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "truncated"), "{t}");
    assert_eq!(ui(&["products", "--max-nodes", "5000"]).1["error"]["code"], "invalid_arguments");
    assert_eq!(ui(&["show", "../x", "room"]).1["error"]["code"], "invalid_arguments");
    assert_eq!(ui(&["show", "cluttered", "room", "--state", "ready"]).1["error"]["code"], "not_found");
    // an invalid evidence file is skipped and named, not fatal
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("ui/p/room/populated");
    std::fs::create_dir_all(&dir).unwrap();
    let mut ev: Value = serde_json::from_str(&std::fs::read_to_string(fixtures().join("ui/clean/room/populated/1440x900-ref.json")).unwrap()).unwrap();
    ev["product"] = json!("p");
    ev["viewport"] = json!("1440");
    ev["seat"] = json!("root");
    std::fs::write(dir.join("1440x900-ref.json"), ev.to_string()).unwrap();
    let (_, out) = run_in(d.path(), &["products"]);
    let v: Value = serde_json::from_str(&out).unwrap();
    let lims = v["evidence"]["limitations"].as_array().unwrap();
    assert!(lims.iter().any(|l| l["code"] == "invalid_evidence" && l["detail"].as_str().unwrap().contains("viewport `1440`") && l["detail"].as_str().unwrap().contains("seat `root`")), "{v}");
}

#[test]
fn the_ui_verbs_are_mcp_tools() {
    use std::io::Write;
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("comparegraph"))
        .current_dir(fixtures())
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let call = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "comparegraph_ui_diff", "arguments": { "target": "room", "ours": "cluttered", "ref": "clean", "ui_root": "." } } });
    let list = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
    writeln!(child.stdin.as_mut().unwrap(), "{list}\n{call}").unwrap();
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    let lines: Vec<Value> = String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let names: Vec<&str> = lines[0]["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for n in ["products", "screens", "show", "diff", "matrix", "system", "flow", "nav", "nav_diff", "journey", "coverage", "verify"] {
        assert!(names.contains(&format!("comparegraph_ui_{n}").as_str()), "tool ui_{n}");
    }
    let env: Value = serde_json::from_str(lines[1]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(env["ok"], true, "{env}");
    assert_eq!(env["data"]["layoutDiff"][0], "nav columns 3 vs 1");
}
