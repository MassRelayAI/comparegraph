//! Field-run ask 5 (docs/reports/2026-11-07-twelve-harness-field-run.md): a has/partial claim carries `reach`, computed
//! at write time by a bounded callee walk from the repo's entry points. The fixture reproduces the field run's real
//! defect: the tools exist, but nothing on a turn's path calls them.

/// `main` drives a turn; the tools are present and never called.
/// (a `\`-continued literal would strip the indentation, so these are written out)
const MAIN: &str = "import os

def main():
    return run_turn(\"hi\")

def run_turn(msg):
    return respond(msg)

def respond(msg):
    return msg.upper()
";

const TOOLS: &str = "def search_tool(q):
    return q.strip()

def fetch_tool(url):
    return url.lower()
";

/// every function has a caller: the entry-points heuristic finds none
const CYCLE: &str = "def ping(n):
    return pong(n - 1)

def pong(n):
    return ping(n)
";

const PACK: &str = "---\nfamily: fam\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n\
| routing.retry | routing | Retry with backoff | Failed calls are retried | a retry loop | basics | 2 |\n\
| memory.recall | memory | Recall | Past turns | a store read | extra | 1 |\n";

fn cc(root: &std::path::Path) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(root);
    c
}
fn json(a: assert_cmd::assert::Assert) -> serde_json::Value {
    serde_json::from_slice(&a.get_output().stdout).unwrap()
}

fn setup(files: &[(&str, &str)]) -> (tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), PACK).unwrap();
    cc(ws.path())
        .args(["family", "add"])
        .arg(pack.path())
        .assert()
        .success();
    let a = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let f = a.path().join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, c).unwrap();
    }
    std::fs::write(
        a.path().join("LICENSE"),
        "MIT License\n\nCopyright (c) 2026 S\n",
    )
    .unwrap();
    // the CLI's build (its file-size and file-count limits); `BuildOptions::default()` parses no symbol bodies
    cc(ws.path())
        .arg("add")
        .arg(a.path())
        .args(["--name", "a"])
        .assert()
        .success();
    cc(ws.path()).args(["build", "a"]).assert().success();
    (ws, a)
}
fn agent() -> (tempfile::TempDir, tempfile::TempDir) {
    setup(&[("src/main.py", MAIN), ("src/tools.py", TOOLS)])
}
fn claim(ws: &std::path::Path, feature: &str, args: &[&str]) -> serde_json::Value {
    let mut c = cc(ws);
    c.args(["claim", "a", feature]).args(args);
    json(c.assert().success())
}
fn codes(v: &serde_json::Value) -> Vec<String> {
    v["evidence"]["limitations"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|l| l["code"].as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default()
}
fn path_names(r: &serde_json::Value) -> Vec<String> {
    r["path"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_function_on_a_turns_path_is_reached_with_its_path() {
    let (ws, _a) = agent();
    let v = claim(
        ws.path(),
        "routing.retry",
        &["has", "--author", "me", "--at", "src/main.py:10"],
    );
    let r = &v["data"]["reach"];
    assert_eq!(r["status"], "reached", "{v}");
    let p = path_names(r);
    assert_eq!(p.len(), 3, "{p:?}");
    assert!(
        p[0].ends_with("main") && p[1].ends_with("run_turn") && p[2].ends_with("respond"),
        "{p:?}"
    );
}

#[test]
fn a_tool_that_exists_but_no_turn_calls_is_unreached() {
    let (ws, _a) = agent();
    let v = claim(
        ws.path(),
        "memory.recall",
        &["has", "--author", "me", "--at", "src/tools.py:2"],
    );
    let r = &v["data"]["reach"];
    assert_eq!(r["status"], "unreached", "{v}");
    assert!(r["path"].as_array().unwrap().is_empty(), "{v}");
    // the target is an entry point only because nothing calls it: that is said, not counted as reached
    assert!(
        codes(&v).contains(&"reach_uncalled_target".to_string()),
        "{v}"
    );
    // `claims` shows the stored reach
    let c = json(
        cc(ws.path())
            .args(["claims", "--feature", "memory.recall"])
            .assert()
            .success(),
    );
    assert_eq!(c["data"]["items"][0]["reach"]["status"], "unreached", "{c}");
}

#[test]
fn a_line_at_file_scope_is_unknown_and_says_why() {
    let (ws, _a) = agent();
    let v = claim(
        ws.path(),
        "routing.retry",
        &["partial", "--author", "me", "--at", "src/main.py:1"],
    );
    assert_eq!(v["data"]["reach"]["status"], "unknown", "{v}");
    assert!(codes(&v).contains(&"reach_unknown".to_string()), "{v}");
}

#[test]
fn a_repo_with_no_entry_points_is_unknown() {
    let (ws, _a) = setup(&[("src/cycle.py", CYCLE)]);
    let v = claim(
        ws.path(),
        "routing.retry",
        &["has", "--author", "me", "--at", "src/cycle.py:2"],
    );
    assert_eq!(v["data"]["reach"]["status"], "unknown", "{v}");
    let lim = v["evidence"]["limitations"].to_string();
    assert!(lim.contains("no entry points"), "{lim}");
}

#[test]
fn a_symbol_past_the_depth_bound_is_unknown_not_unreached() {
    // main -> f1 -> ... -> f12: f12 sits 12 hops out, past depth 10
    let mut src = String::from("def main():\n    return f1()\n");
    for k in 1..12 {
        src.push_str(&format!("\ndef f{k}():\n    return f{}()\n", k + 1));
    }
    src.push_str("\ndef f12():\n    return 12\n");
    let line = src.lines().count();
    let (ws, _a) = setup(&[("src/chain.py", &src)]);
    let at = format!("src/chain.py:{line}");
    let v = claim(
        ws.path(),
        "routing.retry",
        &["has", "--author", "me", "--at", &at],
    );
    assert_eq!(v["data"]["reach"]["status"], "unknown", "{v}");
    assert!(codes(&v).contains(&"truncated".to_string()), "{v}");
    // f10 is 10 hops out: inside the bound
    let at = format!("src/chain.py:{}", line - 6);
    let v = claim(
        ws.path(),
        "memory.recall",
        &["has", "--author", "me", "--at", &at],
    );
    assert_eq!(v["data"]["reach"]["status"], "reached", "{v}");
    assert_eq!(
        v["data"]["reach"]["path"].as_array().unwrap().len(),
        11,
        "{v}"
    );
}

#[test]
fn none_and_unknown_claims_carry_no_reach() {
    let (ws, _a) = agent();
    let v = claim(ws.path(), "routing.retry", &["unknown", "--author", "me"]);
    assert!(v["data"].get("reach").is_none(), "{v}");
}

#[test]
fn matrix_filters_on_reach_and_shows_it_per_cell() {
    let (ws, _a) = agent();
    claim(
        ws.path(),
        "routing.retry",
        &["has", "--author", "me", "--at", "src/main.py:10"],
    );
    claim(
        ws.path(),
        "memory.recall",
        &["has", "--author", "me", "--at", "src/tools.py:2"],
    );
    let all = json(cc(ws.path()).args(["matrix", "fam"]).assert().success());
    assert_eq!(all["data"]["rows"].as_array().unwrap().len(), 2, "{all}");
    for (want, id, other) in [
        ("unreached", "memory.recall", "routing.retry"),
        ("reached", "routing.retry", "memory.recall"),
    ] {
        let m = json(
            cc(ws.path())
                .args(["matrix", "fam", "--reach", want])
                .assert()
                .success(),
        );
        let rows = m["data"]["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{want}: {m}");
        assert_eq!(rows[0]["id"], id, "{m}");
        assert_eq!(rows[0]["reach"][0], want, "{m}");
        assert!(!m.to_string().contains(other), "{m}");
    }
    let m = json(
        cc(ws.path())
            .args(["matrix", "fam", "--reach", "unknown"])
            .assert()
            .success(),
    );
    assert!(m["data"]["rows"].as_array().unwrap().is_empty(), "{m}");
    cc(ws.path())
        .args(["matrix", "fam", "--reach", "maybe"])
        .assert()
        .failure();
}

#[test]
fn a_batch_row_gets_reach_too() {
    let (ws, _a) = agent();
    let mut c = cc(ws.path());
    c.args(["claim", "--batch", "-"]).write_stdin(
        "{\"repo\":\"a\",\"feature\":\"memory.recall\",\"verdict\":\"has\",\"author\":\"me\",\"at\":[\"src/tools.py:5\"]}\n",
    );
    let out = String::from_utf8(c.assert().success().get_output().stdout.clone()).unwrap();
    assert!(out.contains("\"unreached\""), "{out}");
}
