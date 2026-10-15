use comparegraph::application::{add::add_repo, build::build_repo};
use comparegraph::providers::BuildOptions;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn clone_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let f = d.path().join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, c).unwrap();
    }
    std::fs::write(
        d.path().join("LICENSE"),
        "MIT License\n\nCopyright (c) 2026 S\n",
    )
    .unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "."],
        vec![
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "x",
        ],
    ] {
        assert!(Command::new("git")
            .arg("-C")
            .arg(d.path())
            .args(&args)
            .status()
            .unwrap()
            .success());
    }
    d
}
fn cc(root: &std::path::Path) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(root);
    c
}
fn json(a: assert_cmd::assert::Assert) -> serde_json::Value {
    serde_json::from_slice(&a.get_output().stdout).unwrap()
}
const PACK: &str = "---\nfamily: fam\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n\
| routing.retry | routing | Retry with backoff | Failed calls are retried | a retry loop | basics | 2 |\n\
| memory.recall | memory | Recall | Past turns | a store read | extra | 1 |\n";

fn setup() -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), PACK).unwrap();
    cc(ws.path())
        .args(["family", "add"])
        .arg(pack.path())
        .assert()
        .success();
    let a = clone_with(&[(
        "src/net/a.py",
        "def retry_with_backoff():\n    return 1\n\ndef recall_turns():\n    return 2\n",
    )]);
    add_repo(ws.path(), a.path(), Some("a"), false).unwrap();
    build_repo(ws.path(), "a", "reference", &BuildOptions::default()).unwrap();
    (ws, a, pack)
}

fn session(ws: &std::path::Path, lines: &[String]) -> Vec<Value> {
    let bin = assert_cmd::cargo::cargo_bin("comparegraph");
    let mut ch = Command::new(bin)
        .arg("--root")
        .arg(ws)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = ch.stdin.take().unwrap();
    for l in lines {
        writeln!(stdin, "{l}").unwrap();
    }
    drop(stdin);
    let out: Vec<Value> = BufReader::new(ch.stdout.take().unwrap())
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect();
    ch.wait().unwrap();
    out
}
fn call(id: i64, name: &str, args: Value) -> String {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}})
        .to_string()
}
fn env(o: &Value) -> Value {
    serde_json::from_str(o["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}
fn analysed() -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
    let (ws, a, p) = setup();
    cc(ws.path()).args(["relate"]).assert().success();
    cc(ws.path())
        .args([
            "claim",
            "a",
            "routing.retry",
            "has",
            "--author",
            "me",
            "--at",
            "src/net/a.py:1-2",
        ])
        .assert()
        .success();
    cc(ws.path())
        .args([
            "claim",
            "a",
            "memory.recall",
            "none",
            "--author",
            "me",
            "--searched",
            "memory zzzqqq",
            "--read-hits",
            "src/net/a.py::recall_turns",
        ])
        .assert()
        .success();
    cc(ws.path())
        .args([
            "claim",
            "a",
            "routing.retry",
            "partial",
            "--author",
            "me",
            "--at",
            "src/net/a.py:1-2",
        ])
        .assert()
        .success();
    cc(ws.path())
        .args(["decide", "a", "adopt", "--why", "good", "--by", "me"])
        .assert()
        .success();
    (ws, a, p)
}

#[test]
fn json_export_carries_the_analysis() {
    let (ws, _a, _p) = analysed();
    let v = json(cc(ws.path()).args(["export", "json"]).assert().success())["data"].clone();
    for k in ["families", "relatedness", "claims", "decisions"] {
        assert!(v[k].is_array(), "{k} in {v}");
    }
    assert_eq!(v["families"][0]["name"], "fam");
    assert_eq!(v["families"][0]["features"].as_array().unwrap().len(), 2);
    assert!(v["families"][0]["matrix"]["rows"].is_array());
    assert_eq!(v["relatedness"].as_array().unwrap().len(), 1);
    let c = v["claims"].as_array().unwrap();
    assert_eq!(c.len(), 2);
    let retry = c.iter().find(|x| x["feature"] == "routing.retry").unwrap();
    assert_eq!(retry["verdict"], "partial");
    assert_eq!(retry["history"], 2);
    assert_eq!(v["decisions"][0]["disposition"], "adopt");
    assert_eq!(v["decisions"][0]["history"], 1);
}

#[test]
fn vault_has_family_and_feature_notes_that_resolve_and_are_deterministic() {
    let (ws, _a, _p) = analysed();
    let out = tempfile::tempdir().unwrap();
    for n in ["x", "y"] {
        cc(ws.path())
            .args(["export", "vault", "--out"])
            .arg(out.path().join(n))
            .assert()
            .success();
    }
    let x = out.path().join("x");
    let fam = std::fs::read_to_string(x.join("families/fam.md")).unwrap();
    assert!(
        fam.contains("type: \"family\"") || fam.contains("type: family"),
        "{fam}"
    );
    assert!(
        fam.contains("[[features/fam/routing-retry]]") && fam.contains("[[repos/a]]"),
        "{fam}"
    );
    let feat = std::fs::read_to_string(x.join("features/fam/routing-retry.md")).unwrap();
    assert!(
        feat.contains("a retry loop") && feat.contains("partial"),
        "{feat}"
    );
    assert!(feat.contains("[[symbols/a/"), "{feat}");
    let none = std::fs::read_to_string(x.join("features/fam/memory-recall.md")).unwrap();
    assert!(none.contains("none"), "{none}");
    let repo = std::fs::read_to_string(x.join("repos/a.md")).unwrap();
    assert!(
        repo.contains("[[families/fam]]") && repo.contains("adopt"),
        "{repo}"
    );
    assert!(std::fs::read_to_string(x.join("index.md"))
        .unwrap()
        .contains("[[families/fam]]"));
    fn walk(base: &std::path::Path, d: &std::path::Path, v: &mut Vec<(String, Vec<u8>)>) {
        let mut es: Vec<_> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        es.sort();
        for p in es {
            if p.is_dir() {
                walk(base, &p, v)
            } else {
                v.push((
                    p.strip_prefix(base).unwrap().to_string_lossy().into(),
                    std::fs::read(&p).unwrap(),
                ))
            }
        }
    }
    let (mut a, mut b) = (vec![], vec![]);
    walk(&x, &x, &mut a);
    walk(&out.path().join("y"), &out.path().join("y"), &mut b);
    assert_eq!(a, b);
    for (k, body) in a.iter().filter(|(k, _)| k.ends_with(".md")) {
        for part in String::from_utf8_lossy(body).split("[[").skip(1) {
            let t = part.split("]]").next().unwrap();
            assert!(x.join(format!("{t}.md")).exists(), "{k} -> {t}");
        }
    }
}

#[test]
fn mcp_exposes_eighteen_tools_and_the_analysis_verbs() {
    let (ws, _a, _p) = analysed();
    let out = session(
        ws.path(),
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string(),
            call(2, "comparegraph_families", json!({})),
            call(3, "comparegraph_matrix", json!({"family":"fam"})),
            call(
                4,
                "comparegraph_claim",
                json!({"repo":"a","feature":"routing.retry","verdict":"has","at":["src/net/a.py:1-2"]}),
            ),
            call(
                5,
                "comparegraph_claim",
                json!({"repo":"a","feature":"routing.retry","verdict":"has","author":"bot","at":["src/net/a.py:1-2"]}),
            ),
            call(6, "comparegraph_relate", json!({})),
            call(7, "comparegraph_claims", json!({"repo":"a","history":true})),
            call(
                8,
                "comparegraph_map",
                json!({"repo":"a","family":"fam","limit":2}),
            ),
            call(9, "comparegraph_decide", json!({})),
            call(
                10,
                "comparegraph_claim",
                json!({"repo":"a","feature":"routing.retry","verdict":"unknown","author":"bot","family":"fam"}),
            ),
            call(
                11,
                "comparegraph_claim",
                json!({"repo":"a","feature":"routing.retry","verdict":"unknown","author":"bot","family":"nope"}),
            ),
        ],
    );
    let tools = out[0]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 18);
    for t in tools {
        assert_eq!(t["inputSchema"]["additionalProperties"], false);
    }
    let claim_tool = tools
        .iter()
        .find(|t| t["name"] == "comparegraph_claim")
        .unwrap();
    assert!(claim_tool["inputSchema"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r == "author"));
    assert!(tools.iter().all(|t| t["name"] != "comparegraph_decide"));
    assert_eq!(env(&out[1])["data"]["items"][0]["name"], "fam");
    let m = env(&out[2]);
    assert_eq!(m["verb"], "matrix");
    assert_eq!(m["data"]["rows"].as_array().unwrap().len(), 2);
    assert_eq!(out[3]["result"]["isError"], true);
    assert_eq!(env(&out[3])["error"]["code"], "invalid_arguments");
    assert_eq!(out[4]["result"]["isError"], false, "{}", out[4]);
    assert_eq!(env(&out[4])["data"]["author"], "bot");
    let r = env(&out[5]);
    assert_eq!(r["data"]["items"].as_array().unwrap().len(), 1);
    assert!(r["data"]["items"][0]["band"].is_string());
    assert!(env(&out[6])["data"]["items"].as_array().unwrap().len() >= 3);
    assert_eq!(env(&out[7])["verb"], "map");
    assert_eq!(out[8]["result"]["isError"], true);
    assert_eq!(out[9]["result"]["isError"], false, "{}", out[9]);
    assert_eq!(out[10]["result"]["isError"], true);
    assert_eq!(env(&out[10])["error"]["code"], "not_found");
    let claim_tool = tools
        .iter()
        .find(|t| t["name"] == "comparegraph_claim")
        .unwrap();
    assert!(claim_tool["inputSchema"]["properties"]["family"].is_object());
}

#[test]
fn mcp_relate_reads_and_never_computes() {
    let (ws, _a, _p) = setup();
    let out = session(ws.path(), &[call(1, "comparegraph_relate", json!({}))]);
    assert_eq!(env(&out[0])["data"]["items"].as_array().unwrap().len(), 0);
}
