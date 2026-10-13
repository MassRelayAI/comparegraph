//! ABS: the graph-backed absent verdict. Fixtures/typescript is built; features come from a small pack.
use comparegraph::application::{add::add_repo, build::build_repo};
use comparegraph::providers::BuildOptions;
use std::path::Path;
use std::process::Command;

fn copy_dir(from: &Path, to: &Path) {
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&t).unwrap();
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), t).unwrap();
        }
    }
}
fn git_commit(d: &Path) {
    for args in [
        vec!["init", "-q"],
        vec!["add", "."],
        vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "x"],
    ] {
        assert!(Command::new("git").arg("-C").arg(d).args(&args).status().unwrap().success());
    }
}
fn cc(root: &Path) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(root);
    c
}
fn json(a: assert_cmd::assert::Assert) -> serde_json::Value {
    serde_json::from_slice(&a.get_output().stdout).unwrap()
}
const PACK: &str = "---\nfamily: fam\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n\
| quantum.teleport | quantum | Teleport qubits | x | a teleporter | basics | 1 |\n\
| math.counter | math | Counter | x | a counter | basics | 1 |\n\
| naming.ountin | naming | Ountin | x | a mid-token path match | basics | 1 |\n\
| quality.tests | quality | Unit tests | x | tests | basics | 1 |\n";

fn setup() -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), PACK).unwrap();
    cc(ws.path()).args(["family", "add"]).arg(pack.path()).assert().success();
    let clone = tempfile::tempdir().unwrap();
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/typescript"), clone.path());
    // a file whose NAME carries the term mid-token ("counting" holds "ountin"): FTS prefix matching cannot see it
    std::fs::write(clone.path().join("src/counting.ts"), "export const x = 1;\n").unwrap();
    std::fs::write(
        clone.path().join("src/handlers.ts"),
        "export function loginUser() { return 1; }\nexport function logoutUser() { return 2; }\nexport function resetUser() { return 3; }\n",
    )
    .unwrap();
    std::fs::write(clone.path().join("LICENSE"), "MIT License\n\nCopyright (c) 2026 S\n").unwrap();
    git_commit(clone.path());
    add_repo(ws.path(), clone.path(), Some("ts"), false).unwrap();
    build_repo(ws.path(), "ts", "reference", &BuildOptions::default()).unwrap();
    (ws, clone, pack)
}
fn check(ws: &Path, feature: &str) -> serde_json::Value {
    json(cc(ws).args(["absent-check", "ts", "fam", feature]).assert().success())
}
fn claim_none(ws: &Path, feature: &str, extra: &[&str]) -> assert_cmd::assert::Assert {
    let mut c = cc(ws);
    c.args(["claim", "ts", feature, "none", "--author", "me"]).args(extra);
    c.assert()
}

#[test]
fn a_feature_nothing_matches_is_absent_and_the_claim_is_derived() {
    let (ws, _c, _p) = setup();
    let v = check(ws.path(), "quantum.teleport");
    assert_eq!(v["data"]["absent"], true, "{v}");
    assert_eq!(v["evidence"]["confidence"], "inferred", "{v}");
    for ch in ["symbols", "paths", "communities", "kinds"] {
        assert_eq!(v["data"]["channels"][ch]["count"], 0, "{ch}: {v}");
    }
    let c = json(claim_none(ws.path(), "quantum.teleport", &[]).success());
    assert_eq!(c["data"]["verdict"], "none", "{c}");
    assert_eq!(c["evidence"]["confidence"], "inferred", "{c}");
    let note = c["data"]["note"].as_str().unwrap();
    assert!(
        note.starts_with("graph-absent (names, paths, community terms, kinds; not strings, flags or config keys): terms="),
        "{note}"
    );
    assert!(note.contains("; stems=") && note.contains("; run=") && note.contains(" provider=reference"), "{note}");
    assert!(note.contains("channels=symbols:0,paths:0,communities:0,kinds:0"), "{note}");
}

#[test]
fn a_symbol_hit_refuses_the_claim_and_lists_the_hits() {
    let (ws, _c, _p) = setup();
    let v = check(ws.path(), "math.counter");
    assert_eq!(v["data"]["absent"], false, "{v}");
    assert_eq!(v["evidence"]["confidence"], "observed", "{v}");
    assert!(v["data"]["channels"]["symbols"]["count"].as_i64().unwrap() > 0, "{v}");
    assert!(!v["data"]["channels"]["symbols"]["hits"].as_array().unwrap().is_empty());
    let r = json(claim_none(ws.path(), "math.counter", &[]).failure());
    assert_eq!(r["error"]["code"], "invalid_arguments", "{r}");
    let m = r["error"]["message"].as_str().unwrap();
    assert!(m.contains("the graph has") && m.contains("--searched"), "{m}");
    let none = json(cc(ws.path()).args(["claims"]).assert().success());
    assert_eq!(none["data"]["items"].as_array().unwrap().len(), 0, "nothing written");
}

#[test]
fn a_path_only_hit_refuses() {
    let (ws, _c, _p) = setup();
    let v = check(ws.path(), "naming.ountin");
    assert_eq!(v["data"]["channels"]["symbols"]["count"], 0, "{v}");
    assert!(v["data"]["channels"]["paths"]["count"].as_i64().unwrap() > 0, "{v}");
    assert_eq!(v["data"]["absent"], false, "{v}");
    claim_none(ws.path(), "naming.ountin", &[]).failure();
}

#[test]
fn a_kind_only_hit_refuses() {
    let (ws, _c, _p) = setup();
    let v = check(ws.path(), "quality.tests");
    assert_eq!(v["data"]["channels"]["symbols"]["count"], 0, "{v}");
    assert!(v["data"]["channels"]["kinds"]["count"].as_i64().unwrap() > 0, "{v}");
    assert_eq!(v["data"]["absent"], false, "{v}");
    claim_none(ws.path(), "quality.tests", &[]).failure();
}

#[test]
fn a_community_keyword_alone_refuses() {
    // `user` is a keyword of the community (loginUser, logoutUser, resetUser).
    let (ws, _c, _p) = setup();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(
        pack.path().join("rows.md"),
        "---\nfamily: fam2\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n| acct.user | acct | User | x | x | basics | 1 |\n",
    )
    .unwrap();
    cc(ws.path()).args(["family", "add"]).arg(pack.path()).assert().success();
    let v = json(cc(ws.path()).args(["absent-check", "ts", "fam2", "acct.user"]).assert().success());
    // since the stem match splits camelCase, the symbols channel also sees `user`; the communities channel must still report it
    assert!(v["data"]["channels"]["communities"]["count"].as_i64().unwrap() > 0, "{v}");
    assert_eq!(v["data"]["absent"], false, "{v}");
    let r = json(
        cc(ws.path()).args(["claim", "ts", "acct.user", "none", "--author", "me"]).assert().failure(),
    );
    assert_eq!(r["error"]["code"], "invalid_arguments", "{r}");
}

#[test]
fn searched_still_records_the_manual_absence() {
    let (ws, _c, _p) = setup();
    let c = json(claim_none(ws.path(), "quantum.teleport", &["--searched", "teleport qqqmissing"]).success());
    assert!(c["data"]["note"].as_str().unwrap().starts_with("searched (author-supplied): teleport qqqmissing → 0 hits"), "{c}");
    assert_eq!(c["evidence"]["confidence"], "derived", "{c}");
    // terms that do not name the feature are refused, whatever they find
    let r = json(claim_none(ws.path(), "quantum.teleport", &["--searched", "qqqq"]).failure());
    assert!(r["error"]["message"].as_str().unwrap().contains("searched terms do not name the feature"), "{r}");
}

#[test]
fn two_checks_give_the_same_json() {
    let (ws, _c, _p) = setup();
    for f in ["quantum.teleport", "math.counter", "quality.tests"] {
        let a = cc(ws.path()).args(["absent-check", "ts", "fam", f]).assert().success().get_output().stdout.clone();
        let b = cc(ws.path()).args(["absent-check", "ts", "fam", f]).assert().success().get_output().stdout.clone();
        let (mut a, mut b): (serde_json::Value, serde_json::Value) =
            (serde_json::from_slice(&a).unwrap(), serde_json::from_slice(&b).unwrap());
        a["cost"] = serde_json::Value::Null;
        b["cost"] = serde_json::Value::Null;
        assert_eq!(a, b);
    }
}

#[test]
fn mcp_exposes_the_check_and_cannot_record_a_graph_absence() {
    let (ws, _c, _p) = setup();
    let req = |id: u32, m: &str, p: serde_json::Value| {
        serde_json::json!({"jsonrpc":"2.0","id":id,"method":m,"params":p}).to_string()
    };
    let input = [
        req(1, "tools/list", serde_json::json!({})),
        req(2, "tools/call", serde_json::json!({"name":"comparegraph_absent_check","arguments":{"repo":"ts","family":"fam","feature":"quantum.teleport"}})),
        req(3, "tools/call", serde_json::json!({"name":"comparegraph_claim","arguments":{"repo":"ts","feature":"quantum.teleport","verdict":"none","author":"me"}})),
    ]
    .join("\n");
    let out = cc(ws.path()).arg("mcp").write_stdin(input).assert().success().get_output().stdout.clone();
    let lines: Vec<serde_json::Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let names: Vec<&str> = lines[0]["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"comparegraph_absent_check"), "{names:?}");
    let env: serde_json::Value = serde_json::from_str(lines[1]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(env["data"]["absent"], true, "{env}");
    // the claim tool cannot record a graph-backed absence: `none` over MCP needs --searched terms
    assert_eq!(lines[2]["result"]["isError"], true, "{}", lines[2]);
}

// ---- the reviewer's four probes (a mini repo that implements each feature) ----
const PROBE_PACK: &str = "---\nfamily: pf\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n\
| symbols.extraction | symbols | Extraction | x | x | basics | 1 |\n\
| core.configuration | core | Configuration | x | x | basics | 1 |\n\
| cli.verbose | cli | Verbose | x | x | basics | 1 |\n\
| perf.caching | perf | Caching | x | x | basics | 1 |\n";

fn probe_ws() -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), PROBE_PACK).unwrap();
    cc(ws.path()).args(["family", "add"]).arg(pack.path()).assert().success();
    let clone = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(clone.path().join("pkg")).unwrap();
    std::fs::write(clone.path().join("pkg/defs.py"), "def extract_defs():\n    return 1\n\ndef parse_tree():\n    return 2\n").unwrap();
    std::fs::write(clone.path().join("pkg/cfg.py"), "def load_cfg():\n    return {}\n").unwrap();
    std::fs::write(clone.path().join("pkg/args.py"), "FLAGS = [\"--verbose\"]\n\ndef run():\n    return FLAGS\n").unwrap();
    std::fs::write(clone.path().join("pkg/helpers.py"), "def memoize(f):\n    return f\n").unwrap();
    std::fs::write(clone.path().join("LICENSE"), "MIT License\n\nCopyright (c) 2026 S\n").unwrap();
    git_commit(clone.path());
    add_repo(ws.path(), clone.path(), Some("mini"), false).unwrap();
    build_repo(ws.path(), "mini", "reference", &BuildOptions::default()).unwrap();
    (ws, clone, pack)
}

#[test]
fn probe_extraction_is_found_by_the_stem_match() {
    let (ws, _c, _p) = probe_ws();
    let v = json(cc(ws.path()).args(["absent-check", "mini", "pf", "symbols.extraction"]).assert().success());
    assert_eq!(v["data"]["absent"], false, "{v}");
    let hits = v["data"]["channels"]["symbols"]["hits"].as_array().unwrap();
    assert!(hits.iter().any(|h| h["name"].as_str().unwrap().contains("extract_defs")), "{v}");
    assert!(
        hits.iter().any(|h| h["matched"].as_array().unwrap().iter().any(|m| m == "extraction")),
        "the term that matched is named: {v}"
    );
    claim_probe(ws.path(), "symbols.extraction", &[]).failure();
}
fn claim_probe(ws: &Path, f: &str, extra: &[&str]) -> assert_cmd::assert::Assert {
    let mut c = cc(ws);
    c.args(["claim", "mini", f, "none", "--author", "me"]).args(extra);
    c.assert()
}

#[test]
fn probes_out_of_reach_are_recorded_inferred_with_the_coverage_limit_never_derived() {
    // abbreviations (cfg), strings ("--verbose") and synonyms (memoize) are not names the graph can match:
    // the claim is recorded, but as `inferred`, and its note says what the check could not see.
    let (ws, _c, _p) = probe_ws();
    for f in ["core.configuration", "cli.verbose", "perf.caching"] {
        let c = json(claim_probe(ws.path(), f, &[]).success());
        assert_eq!(c["evidence"]["confidence"], "inferred", "{f}: {c}");
        let note = c["data"]["note"].as_str().unwrap();
        assert!(note.contains("not strings, flags or config keys"), "{note}");
    }
}

#[test]
fn the_check_pins_the_reference_run() {
    let (ws, _c, _p) = probe_ws();
    let v = json(cc(ws.path()).args(["absent-check", "mini", "pf", "perf.caching"]).assert().success());
    assert_eq!(v["data"]["provider"], "reference", "{v}");
    assert!(v["data"]["run_id"].as_i64().unwrap() > 0, "{v}");
    assert!(v["data"]["stems"].is_array(), "{v}");
}

// ---- Plan 8 lane C fix round: family words, and no kinds scoping on the symbol channel ----
fn row(id: &str, title: &str) -> String {
    let grp = id.split('.').next().unwrap();
    format!("| {id} | {grp} | {title} | x | y | basics | 1 |\n")
}
fn pack_of(rows: &[(&str, &str)]) -> String {
    let mut p = String::from("---\nfamily: fam\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n");
    for (id, t) in rows {
        p.push_str(&row(id, t));
    }
    p
}
/// A workspace with repo `r` built from `files` (path, text) and the family `fam` from `pack`.
fn ws_of(pack: &str, files: &[(&str, String)]) -> tempfile::TempDir {
    let ws = tempfile::tempdir().unwrap();
    let pk = tempfile::tempdir().unwrap();
    std::fs::write(pk.path().join("rows.md"), pack).unwrap();
    cc(ws.path()).args(["family", "add"]).arg(pk.path()).assert().success();
    let clone = tempfile::tempdir().unwrap();
    for (p, t) in files {
        let f = clone.path().join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, t).unwrap();
    }
    std::fs::write(clone.path().join("LICENSE"), "MIT License\n\nCopyright (c) 2026 S\n").unwrap();
    git_commit(clone.path());
    add_repo(ws.path(), clone.path(), Some("r"), false).unwrap();
    build_repo(ws.path(), "r", "reference", &BuildOptions::default()).unwrap();
    ws
}
fn py_fns(prefix: &str, n: usize) -> String {
    (0..n).map(|i| format!("def {prefix}_{i}():\n    return {i}\n\n")).collect()
}
fn claims(ws: &Path) -> i64 {
    let db = rusqlite::Connection::open(ws.join(".comparegraph/comparegraph.db")).unwrap();
    db.query_row("SELECT count(*) FROM claim", [], |r| r.get(0)).unwrap()
}
fn check_r(ws: &Path, feature: &str, ok: bool) -> serde_json::Value {
    let a = cc(ws).args(["absent-check", "r", "fam", feature]).assert();
    json(if ok { a.success() } else { a.failure() })
}

// Defect 1 (review clone c1): a feature that names a kind must not lose the symbol hits of a function named for it.
#[test]
fn a_function_named_for_a_feature_that_names_a_kind_is_a_symbol_hit_and_none_is_refused() {
    let pack = pack_of(&[("serving.http-endpoints", "HTTP endpoints"), ("symbols.test-detection", "Test detection")]);
    let ws = ws_of(
        &pack,
        &[
            ("pkg/m1.py", format!("def handle_http_api_request():\n    return 1\n\n{}", py_fns("alpha_bravo_charlie_delta_echo", 25))),
            ("pkg/m2.py", format!("def detect_test_functions():\n    return 2\n\n{}", py_fns("foxtrot_golf_hotel_india_juliet", 25))),
        ],
    );
    for f in ["serving.http-endpoints", "symbols.test-detection"] {
        let v = check_r(ws.path(), f, true);
        assert_eq!(v["data"]["absent"], false, "{f}: {v}");
        assert!(v["data"]["channels"]["symbols"]["count"].as_i64().unwrap() >= 1, "{f}: {v}");
        let c = json(cc(ws.path()).args(["claim", "r", f, "none", "--author", "me"]).assert().failure());
        assert_eq!(c["error"]["code"], "invalid_arguments", "{c}");
    }
    assert_eq!(claims(ws.path()), 0);
}

// Defect 2 (review clones c2f, c2c): a word carried by 300 names is evidence of presence, not a common term.
#[test]
fn a_word_carried_by_hundreds_of_names_is_not_dropped_and_none_is_refused() {
    let pack = pack_of(&[("scheduling.cron-tasks", "Cron tasks"), ("queue.retry", "Retry")]);
    for prefix in ["cron_job", "cron_task"] {
        let ws = ws_of(&pack, &[("pkg/jobs.py", py_fns(prefix, 300))]);
        let v = check_r(ws.path(), "scheduling.cron-tasks", true);
        assert_eq!(v["data"]["absent"], false, "{prefix}: {v}");
        assert_eq!(v["data"]["common"], serde_json::json!([]), "{prefix}: {v}");
        assert!(v["data"]["channels"]["symbols"]["count"].as_i64().unwrap() >= 300, "{prefix}: {v}");
        let c = json(cc(ws.path()).args(["claim", "r", "scheduling.cron-tasks", "none", "--author", "me"]).assert().failure());
        assert!(c["error"]["message"].as_str().unwrap().contains("the graph has"), "{c}");
        assert_eq!(claims(ws.path()), 0);
    }
}

const AGENTS: [(&str, &str); 7] = [
    ("agents.alpha", "Alpha"),
    ("agents.bravo", "Bravo"),
    ("agents.charlie", "Charlie"),
    ("agents.delta", "Delta"),
    ("agents.echo", "Echo"),
    ("agents.foxtrot", "Foxtrot"),
    ("agents.x", "Zebra"),
];

// A family word is dropped and the rest decides; a mutation that never drops it goes red here.
#[test]
fn a_family_word_is_set_aside_and_zero_hits_for_the_rest_refuses_naming_it() {
    let ws = ws_of(&pack_of(&AGENTS), &[("pkg/a.py", py_fns("agent_run", 30)), ("pkg/b.py", py_fns("agents_pool", 30))]);
    let v = check_r(ws.path(), "agents.x", true);
    // `agents` is set aside (it would hit every agent_* name); with zero hits for the rest the graph proves less, so the
    // check refuses and names the uncounted word instead of recording an absence
    assert_eq!(v["data"]["absent"], false, "{v}");
    assert_eq!(v["data"]["common"], serde_json::json!(["agents"]), "{v}");
    assert_eq!(v["data"]["channels"]["symbols"]["count"], 0, "{v}");
    let c = json(cc(ws.path()).args(["claim", "r", "agents.x", "none", "--author", "me"]).assert().failure());
    let msg = c["error"]["message"].as_str().unwrap();
    assert!(msg.contains("family word(s) agents were not counted") && msg.contains("--searched"), "{c}");
    let all = json(cc(ws.path()).args(["claims", "--history"]).assert().success());
    assert_eq!(all["data"]["items"].as_array().unwrap().len(), 0, "{all}");
}

#[test]
fn a_feature_whose_every_term_is_a_family_word_is_refused_and_records_nothing() {
    let mut rows = AGENTS.to_vec();
    rows.push(("agents.agents", "Agents"));
    let ws = ws_of(&pack_of(&rows), &[("pkg/a.py", py_fns("other", 5))]);
    let msg = "every term of agents.agents is a family word in fam (agents); pass --searched with specific terms";
    let v = check_r(ws.path(), "agents.agents", false);
    assert_eq!(v["error"]["code"], "invalid_arguments", "{v}");
    assert_eq!(v["error"]["message"], msg, "{v}");
    let c = json(cc(ws.path()).args(["claim", "r", "agents.agents", "none", "--author", "me"]).assert().failure());
    assert_eq!(c["error"]["message"], msg, "{c}");
    assert_eq!(claims(ws.path()), 0, "an all-family-word feature must not record an absence");
}

// A word shared by fewer than 4 features is not a family word.
#[test]
fn a_word_in_three_features_is_not_a_family_word() {
    let rows = [("agents.alpha", "Alpha"), ("agents.bravo", "Bravo"), ("agents.x", "Zebra")];
    let ws = ws_of(&pack_of(&rows), &[("pkg/a.py", py_fns("agent_run", 5))]);
    let v = check_r(ws.path(), "agents.x", true);
    assert_eq!(v["data"]["common"], serde_json::json!([]), "{v}");
    assert_eq!(v["data"]["absent"], false, "{v}");
}
