//! The delta "already on base" model (design read 2026-11-19, docs/decisions.md "Plan 14: the delta model"): whole-tree copy
//! counts, a computed unread set, one-to-one credit and a three-valued answer. Each test is a review probe (rounds 1-3) or a
//! slice's red test; fixture repos are real git repositories in temp dirs, no network.
#![allow(dead_code)]
mod common;
use common::{git, ok};
use serde_json::Value;
use std::path::{Path, PathBuf};

const MIT: &str = "MIT License\n\nCopyright (c) 2026 S\n";
const BAR: &str = "def bar(x):\n    y = x * 2\n    return y + 1\n";
const FOO: &str = "def foo():\n    return 1\n";
const KEEP: &str = "def keep():\n    return 1\n";

fn commit(dir: &Path, msg: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", msg]);
}
fn write(dir: &Path, rel: &str, body: &str) {
    let f = dir.join(rel);
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(f, body).unwrap();
}

struct Fx {
    ws: tempfile::TempDir,
    base: tempfile::TempDir,
}
impl Fx {
    /// A clone on branch `main` with the given files, registered as `r` in a fresh store.
    fn new(files: &[(&str, &str)]) -> Fx {
        let (ws, base) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let d = base.path().join("r");
        std::fs::create_dir_all(&d).unwrap();
        git(&d, &["init", "-q", "-b", "main"]);
        write(&d, "LICENSE", MIT);
        for (p, c) in files {
            write(&d, p, c);
        }
        commit(&d, "c1");
        ok(ws.path(), &["add", d.to_str().unwrap(), "--name", "r"]);
        Fx { ws, base }
    }
    fn d(&self) -> PathBuf {
        self.base.path().join("r")
    }
    fn root(&self) -> &Path {
        self.ws.path()
    }
    fn delta(&self) -> Value {
        ok(self.root(), &["delta", "r", "--base", "main", "--head", "feat", "--refresh"])
    }
    fn feat(&self) {
        git(&self.d(), &["checkout", "-q", "-b", "feat"]);
    }
    fn main(&self) {
        git(&self.d(), &["checkout", "-q", "main"]);
    }
}

/// `def process(items)`: 30 lines with their own names, optionally a 2-line guard and a tweak on the return.
fn long_process(guard: bool, tweak: &str) -> String {
    let mut s = String::from("def process(items):\n    total = 0\n");
    if guard {
        s.push_str("    if not items:\n        raise Denied()\n");
    }
    for i in 0..30 {
        s.push_str(&format!("    total = total + items[{i}] * {} - len(items) + k{i}\n", i + 2));
    }
    s.push_str(&format!("    return total{tweak}\n"));
    s
}
fn long_named(name: &str, guard: bool) -> String {
    long_process(guard, "").replacen("def process(", &format!("def {name}("), 1)
}
fn exact_share(v: &Value) -> Option<f64> {
    v["data"]["summary"]["share_on_base_exact"].as_f64()
}
fn branch(v: &Value) -> &Value {
    &v["data"]["branch"]
}
fn limits(v: &Value) -> String {
    v["evidence"]["limitations"].to_string()
}

// ---------------------------------------------------------------- slice 0: every side's limitation; the whole-tree exact share

/// P5's limitation half: base renames the file to a Windows reserved name, which base's run skips. The skipped path must reach
/// the delta's own limitations (every limitation of every side is carried, not a fixed list of codes).
#[test]
fn s0_every_side_limitation_is_carried_into_the_delta() {
    let fx = Fx::new(&[("src/a.py", &format!("{}\n{KEEP}", long_process(false, ""))), ("src/k.py", KEEP)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", KEEP);
    commit(&d, "lane deletes process");
    fx.main();
    git(&d, &["mv", "src/a.py", "src/aux.py"]);
    commit(&d, "base renames to aux.py");
    let v = fx.delta();
    let l = limits(&v);
    assert!(l.contains("windows_reserved_skipped") && l.contains("src/aux.py"), "the skipped path is silent in delta: {l}");
    let lim = v["evidence"]["limitations"].as_array().unwrap().iter().find(|x| x["code"] == "windows_reserved_skipped").unwrap().clone();
    assert!(lim["detail"].as_str().unwrap().starts_with("base"), "the side is named: {lim}");
}

/// P8: the whole-tree summary carries the exact share under the branch summary's name.
#[test]
fn p8_whole_tree_has_share_on_base_exact() {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", &format!("{FOO}\n{BAR}"));
    commit(&d, "feat adds bar");
    fx.main();
    let v = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--refresh", "--whole-tree"]);
    let ws = &v["data"]["whole_tree"]["summary"];
    assert_eq!(ws["share_on_base_exact"].as_f64(), Some(0.0), "whole_tree.summary lacks share_on_base_exact: {ws}");
    assert_eq!(ws["share_on_base_exact"], ws["share_already_on_base"], "{ws}");
}
