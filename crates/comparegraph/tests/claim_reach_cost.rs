//! The cost of `reach` in a batch (fix/reach-cost): the per-run reach state (entry set, the walk's node and adjacency
//! caches, the walk from every entry point) is built once per run for a whole `claim --batch`, not once per row, and
//! the cached answers are identical to a fresh build per row.
use comparegraph::application::claim::{claim_batch_with, BatchRow, ClaimOut};
use comparegraph::application::reach::state_builds;
use std::path::Path;

const PACK: &str = "---\nfamily: fam\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n\
| routing.retry | routing | Retry with backoff | Failed calls are retried | a retry loop | basics | 2 |\n\
| memory.recall | memory | Recall | Past turns | a store read | extra | 1 |\n";

/// `main` drives a turn; the tools are present and never called; `handle_ping` reaches `pong_far` by name only.
/// (`other.py` and `far.py` open with a comment: a function spanning its whole file resolves to the file node)
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
const OTHER: &str = "# a handler\n\ndef handle_ping(n):
    return pong_far(n)
";
const FAR: &str = "# far away\n\ndef pong_far(n):
    return n
";

fn cc(root: &Path) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(root);
    c
}

fn write_tree(dir: &Path, files: &[(String, String)]) {
    for (p, c) in files {
        let f = dir.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, c).unwrap();
    }
    std::fs::write(dir.join("LICENSE"), "MIT License\n\nCopyright (c) 2026 S\n").unwrap();
}

/// `main` calls 60 functions, each calling 40 leaves: 2,461 functions, more than the walk's 2,000-node bound.
/// Returns the source and the lines of `a5`, `b_0_0` and `b_59_39`'s bodies.
fn big_source() -> (String, usize, usize, usize) {
    let mut s = String::from("def main():\n");
    for i in 0..60 {
        s.push_str(&format!("    a{i}()\n"));
    }
    let (mut a5, mut b00, mut blast) = (0, 0, 0);
    for i in 0..60 {
        s.push_str(&format!("\ndef a{i}():\n"));
        if i == 5 {
            a5 = s.lines().count() + 1;
        }
        for j in 0..40 {
            s.push_str(&format!("    b_{i}_{j}()\n"));
        }
    }
    for i in 0..60 {
        for j in 0..40 {
            s.push_str(&format!("\ndef b_{i}_{j}():\n"));
            if (i, j) == (0, 0) {
                b00 = s.lines().count() + 1;
            }
            if (i, j) == (59, 39) {
                blast = s.lines().count() + 1;
            }
            s.push_str(&format!("    return {j}\n"));
        }
    }
    (s, a5, b00, blast)
}

struct Ws {
    ws: tempfile::TempDir,
    a: tempfile::TempDir,
    _big: tempfile::TempDir,
    _pack: tempfile::TempDir,
    big_lines: (usize, usize, usize),
}

/// A workspace with two built repos: `a` (reached, unreached, uncalled, name-guessed, file scope) and `big` (truncated).
fn workspace() -> Ws {
    let ws = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("rows.md"), PACK).unwrap();
    cc(ws.path()).args(["family", "add"]).arg(pack.path()).assert().success();
    let a = tempfile::tempdir().unwrap();
    write_tree(
        a.path(),
        &[
            ("src/main.py".into(), MAIN.into()),
            ("src/tools.py".into(), TOOLS.into()),
            ("src/other.py".into(), OTHER.into()),
            ("src/far.py".into(), FAR.into()),
        ],
    );
    let big = tempfile::tempdir().unwrap();
    let (src, a5, b00, blast) = big_source();
    write_tree(big.path(), &[("src/main.py".into(), src)]);
    for (dir, name) in [(a.path(), "a"), (big.path(), "big")] {
        cc(ws.path()).arg("add").arg(dir).args(["--name", name]).assert().success();
        cc(ws.path()).args(["build", name]).assert().success();
    }
    Ws { ws, a, _big: big, _pack: pack, big_lines: (a5, b00, blast) }
}

fn cases(w: &Ws) -> Vec<(&'static str, &'static str, String)> {
    let (a5, b00, blast) = w.big_lines;
    vec![
        ("a", "routing.retry", "src/main.py:10".into()),
        ("a", "memory.recall", "src/tools.py:2".into()),
        ("a", "routing.retry", "src/main.py:1".into()),
        ("a", "memory.recall", "src/far.py:4".into()),
        ("a", "routing.retry", "src/other.py:4".into()),
        ("a", "memory.recall", "src/tools.py:5".into()),
        ("big", "routing.retry", format!("src/main.py:{a5}")),
        ("big", "memory.recall", format!("src/main.py:{b00}")),
        ("big", "routing.retry", format!("src/main.py:{blast}")),
        ("big", "memory.recall", "src/main.py:2".into()),
    ]
}

/// `n` has-claim rows cycling through the cases, each with its own note so no row is skipped as equal.
fn rows(w: &Ws, n: usize, tag: &str) -> String {
    let c = cases(w);
    (0..n)
        .map(|k| {
            let (repo, feature, at) = &c[k % c.len()];
            serde_json::json!({"repo": repo, "feature": feature, "verdict": "has", "author": "me", "at": [at], "note": format!("{tag}-{k}")})
                .to_string()
                + "\n"
        })
        .collect()
}

fn batch(root: &Path, text: &str, cache: bool) -> Vec<ClaimOut> {
    let mut out = vec![];
    let sum = claim_batch_with(root, text.as_bytes(), cache, |r: BatchRow<ClaimOut>| {
        assert!(r.ok, "row {} failed: {:?}", r.line, r.error.as_ref().map(|e| &e.message));
        out.push(r.data.unwrap());
    })
    .unwrap();
    assert_eq!(sum.failed, 0);
    out
}

/// what the reach decided for a row: the reach itself and every reach-related limitation
fn reach_of(o: &ClaimOut) -> serde_json::Value {
    serde_json::json!({ "reach": o.reach, "confidence": o.confidence, "limitations": o.limitation_items })
}

#[test]
fn a_batch_builds_the_reach_state_once_per_run_not_once_per_row() {
    let w = workspace();
    let before = state_builds();
    batch(w.ws.path(), &rows(&w, 10, "ten"), true);
    let ten = state_builds() - before;
    let before = state_builds();
    batch(w.ws.path(), &rows(&w, 40, "forty"), true);
    let forty = state_builds() - before;
    // two repos, two runs: one build each, whatever the row count
    assert_eq!(ten, 2, "10 rows built the reach state {ten} times");
    assert_eq!(forty, 2, "40 rows built the reach state {forty} times");
}

#[test]
fn the_cached_reach_equals_a_fresh_build_for_every_row() {
    let w = workspace();
    let n = 2 * cases(&w).len();
    let fresh = batch(w.ws.path(), &rows(&w, n, "fresh"), false);
    let cached = batch(w.ws.path(), &rows(&w, n, "cached"), true);
    assert_eq!(fresh.len(), cached.len());
    for (k, (f, c)) in fresh.iter().zip(&cached).enumerate() {
        assert_eq!(reach_of(f), reach_of(c), "row {k}");
    }
    // the fixture covers every reach outcome and qualifier
    let all = serde_json::to_string(&fresh.iter().map(reach_of).collect::<Vec<_>>()).unwrap();
    for want in [
        "\"reached\"",
        "\"unreached\"",
        "\"unknown\"",
        "\"truncated\"",
        "\"reach_uncalled_target\"",
        "\"reach_static_only\"",
        "\"reach_unknown\"",
        "\"reach_name_guessed\"",
    ] {
        assert!(all.contains(want), "no {want} in {all}");
    }
}

#[test]
fn a_new_run_between_two_batches_gets_a_new_reach() {
    let w = workspace();
    let row = "{\"repo\":\"a\",\"feature\":\"memory.recall\",\"verdict\":\"has\",\"author\":\"me\",\"at\":[\"src/tools.py:2\"]}\n";
    let first = batch(w.ws.path(), row, true);
    assert_eq!(first[0].reach.as_ref().unwrap().status, "unreached");
    // a turn now calls the tool; the rebuild is a new run
    std::fs::write(
        w.a.path().join("src/main.py"),
        MAIN.replace("    return respond(msg)", "    return respond(search_tool(msg))")
            .replace("import os", "import os\nfrom tools import search_tool"),
    )
    .unwrap();
    cc(w.ws.path()).args(["build", "a"]).assert().success();
    let before = state_builds();
    let second = batch(w.ws.path(), row, true);
    assert_eq!(state_builds() - before, 1);
    assert_ne!(first[0].run_id, second[0].run_id);
    let r = second[0].reach.as_ref().unwrap();
    assert_eq!(r.status, "reached", "{r:?}");
    assert!(r.path.last().unwrap().ends_with("search_tool"), "{r:?}");
}
