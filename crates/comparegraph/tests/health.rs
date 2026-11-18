//! `health`: one dirty fixture and one clean twin per finding kind, the ratchet, determinism and one mutation per kind.
//! A mutation here is the library run with that section's detector disabled (`Options::disable`): the dirty fixture's
//! assertion ("the section reports a finding") must then fail, which shows the test would go red on a detector that finds nothing.
use comparegraph::application::health::{health, Options, SECTIONS};
use serde_json::Value;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    assert!(Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(args)
        .status()
        .unwrap()
        .success());
}
fn write(dir: &Path, files: &[(&str, &str)]) {
    for (p, c) in files {
        let f = dir.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, c).unwrap();
    }
}
fn clone_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), files);
    write(d.path(), &[("LICENSE", "MIT License\n\nCopyright (c) 2026 S\n")]);
    git(d.path(), &["init", "-q"]);
    git(d.path(), &["add", "."]);
    git(d.path(), &["commit", "-q", "-m", "x"]);
    d
}
fn cc(root: &Path) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(root);
    c
}
fn json(a: assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&a.get_output().stdout).unwrap()
}
/// A workspace with repo `r` built from `files`, and an optional `comparegraph.toml`.
fn ws_with(files: &[(&str, &str)], toml: &str) -> (tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let c = clone_with(files);
    if !toml.is_empty() {
        write(ws.path(), &[("comparegraph.toml", toml)]);
    }
    cc(ws.path()).args(["add", c.path().to_str().unwrap(), "--name", "r"]).assert().success();
    cc(ws.path()).args(["build", "r"]).assert().success();
    (ws, c)
}
fn count(ws: &Path, section: &str) -> u64 {
    let v = json(cc(ws).args(["health", "r"]).assert().success());
    assert_eq!(v["verb"], "health");
    v["data"]["summary"][section].as_u64().unwrap_or_else(|| panic!("{v}"))
}
fn mutated(ws: &Path, section: &str) -> u64 {
    let o = Options { limit: 50, disable: vec![section.to_string()], ..Default::default() };
    health(ws, "r", &o).unwrap().data["summary"][section].as_u64().unwrap()
}

/// dirty reports, clean twin reports zero, and the dirty one reports nothing when the detector is disabled.
fn kind(section: &str, dirty: &[(&str, &str)], clean: &[(&str, &str)], toml: &str) {
    assert!(SECTIONS.contains(&section));
    let (d, _dc) = ws_with(dirty, toml);
    let (c, _cc) = ws_with(clean, toml);
    assert!(count(d.path(), section) >= 1, "{section}: the dirty fixture reports nothing");
    assert_eq!(count(c.path(), section), 0, "{section}: the clean twin reports findings");
    assert_eq!(mutated(d.path(), section), 0, "{section}: the disabled detector still reports");
    // a finding carries evidence
    let v = json(cc(d.path()).args(["health", "r"]).assert().success());
    for it in v["data"]["sections"][section]["items"].as_array().unwrap() {
        assert!(!it["evidence"].as_array().unwrap().is_empty(), "{it}");
        assert!(it["evidence"][0]["line"].as_u64().unwrap() >= 1 && !it["evidence"][0]["path"].as_str().unwrap().is_empty());
    }
}

#[test]
fn cycles_in_imports() {
    kind(
        "cycles",
        &[
            ("src/a.ts", "import { b } from './b';\nexport function a() { return b(); }\n"),
            ("src/b.ts", "import { a } from './a';\nexport function b() { return a(); }\n"),
        ],
        &[
            ("src/a.ts", "import { b } from './b';\nexport function a() { return b(); }\n"),
            ("src/b.ts", "export function b() { return 1; }\n"),
        ],
        "",
    );
}

#[test]
fn cycles_in_extends() {
    kind(
        "cycles",
        &[("src/k.ts", "export class A extends B {}\nexport class B extends A {}\n")],
        &[("src/k.ts", "export class A extends B {}\nexport class B {}\n")],
        "",
    );
}

#[test]
fn dead_imports() {
    kind(
        "dead_imports",
        &[
            ("src/lib.ts", "export function used() { return 1; }\nexport function unused() { return 2; }\n"),
            ("src/m.ts", "import { used, unused } from './lib';\nexport const v = used();\n"),
        ],
        &[
            ("src/lib.ts", "export function used() { return 1; }\nexport function unused() { return 2; }\n"),
            ("src/m.ts", "import { used, unused } from './lib';\nexport const v = used() + unused();\n"),
        ],
        "",
    );
}

fn body(name: &str, ops: [&str; 4]) -> String {
    format!(
        "export function {name}(x: number) {{\n  const a = x {} 1;\n  const b = a {} 2;\n  const c = b {} 3;\n  const d = c {} 4;\n  const e = d + a;\n  const g = e * b;\n  const h = g - c;\n  return h + d;\n}}\n",
        ops[0], ops[1], ops[2], ops[3]
    )
}
#[test]
fn duplicate_bodies() {
    let same = ["+", "*", "-", "/"];
    kind(
        "duplicate_bodies",
        &[("src/one.ts", &body("one", same)), ("src/two.ts", &body("two", same))],
        &[("src/one.ts", &body("one", same)), ("src/two.ts", &body("two", ["-", "+", "*", "+"]))],
        "",
    );
}

#[test]
fn size_outliers() {
    let toml = "[health]\nmax_file_lines = 6\nmax_function_lines = 4\n";
    kind(
        "size_outliers",
        &[("src/big.ts", &body("big", ["+", "*", "-", "/"]))],
        &[("src/small.ts", "export function s() {\n  return 1;\n}\n")],
        toml,
    );
}

#[test]
fn coupling_outliers() {
    let toml = "[health]\nmax_file_fan_out = 1\n";
    let libs = [("src/a.ts", "export const a = 1;\n"), ("src/b.ts", "export const b = 2;\n")];
    let mut dirty = libs.to_vec();
    dirty.push(("src/hub.ts", "import { a } from './a';\nimport { b } from './b';\nexport const h = a + b;\n"));
    let mut clean = libs.to_vec();
    clean.push(("src/hub.ts", "import { a } from './a';\nexport const h = a;\n"));
    kind("coupling_outliers", &dirty, &clean, toml);
}

#[test]
fn boundary_violations() {
    let toml = "[[health.boundary]]\nfrom = \"src/ui/**\"\nnever = \"src/db/**\"\n";
    kind(
        "boundary_violations",
        &[
            ("src/db/x.ts", "export const x = 1;\n"),
            ("src/ui/a.ts", "import { x } from '../db/x';\nexport const a = x;\n"),
        ],
        &[
            ("src/lib/y.ts", "export const y = 1;\n"),
            ("src/db/x.ts", "export const x = 1;\n"),
            ("src/ui/a.ts", "import { y } from '../lib/y';\nexport const a = y;\n"),
        ],
        toml,
    );
}

#[test]
fn dead_code_is_folded_in() {
    kind(
        "dead_code",
        &[("m.py", "def _unused():\n    return 1\n\n\ndef main():\n    return 2\n")],
        &[("m.py", "def _used():\n    return 1\n\n\ndef main():\n    return _used()\n")],
        "",
    );
}

fn cyc_files(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    let mut v = vec![
        ("src/a.ts", "import { b } from './b';\nexport function a() { return b(); }\n"),
        ("src/b.ts", "import { a } from './a';\nexport function b() { return a(); }\n"),
    ];
    v.extend_from_slice(extra);
    v
}

fn rebuild(ws: &Path, clone: &Path, files: &[(&str, &str)]) {
    write(clone, files);
    git(clone, &["add", "."]);
    git(clone, &["commit", "-q", "-m", "y"]);
    cc(ws).args(["build", "r"]).assert().success();
}

#[test]
fn the_ratchet_fails_on_a_new_cycle_and_passes_on_a_fixed_one() {
    let (ws, clone) = ws_with(&cyc_files(&[]), "");
    let base = ws.path().join("base.json");
    let b = base.to_str().unwrap();
    let w = json(cc(ws.path()).args(["health", "r", "--baseline", b]).assert().success());
    assert_eq!(w["data"]["baseline"]["keys"], 1);
    let text = std::fs::read_to_string(&base).unwrap();
    let keys: Vec<String> = serde_json::from_str::<Value>(&text).unwrap()["keys"].as_array().unwrap().iter().map(|k| k.as_str().unwrap().to_string()).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "the baseline is sorted");
    assert!(keys[0].starts_with("cycle:imports:"));
    // unchanged: exit 0
    let v = json(cc(ws.path()).args(["health", "r", "--check", b]).assert().success());
    assert_eq!(v["data"]["check"]["new_total"], 0);
    // a new cycle: c -> a -> b -> c via b importing c and c importing b
    rebuild(
        ws.path(),
        clone.path(),
        &[
            ("src/b.ts", "import { a } from './a';\nimport { c } from './c';\nexport function b() { return a() + c(); }\n"),
            ("src/c.ts", "import { b } from './b';\nexport function c() { return b(); }\n"),
        ],
    );
    let v = json(cc(ws.path()).args(["health", "r", "--check", b]).assert().code(1));
    assert_eq!(v["ok"], false);
    assert!(v["data"]["check"]["new_total"].as_u64().unwrap() >= 1, "{v}");
    assert!(v["data"]["check"]["new"][0]["key"].as_str().unwrap().starts_with("cycle:imports:"));
    assert_eq!(v["error"]["code"], "check_failed");
    // fixed: no cycle left, exit 0, and the message says to shrink the baseline
    rebuild(
        ws.path(),
        clone.path(),
        &[
            ("src/a.ts", "export function a() { return 1; }\n"),
            ("src/b.ts", "export function b() { return 1; }\n"),
            ("src/c.ts", "export function c() { return 1; }\n"),
        ],
    );
    let v = json(cc(ws.path()).args(["health", "r", "--check", b]).assert().success());
    assert_eq!(v["data"]["check"]["new_total"], 0);
    assert_eq!(v["data"]["check"]["fixed_total"], 1);
    assert!(v["data"]["check"]["message"].as_str().unwrap().contains("now clean, shrink the baseline"));
}

#[test]
fn two_runs_give_the_same_bytes() {
    let (ws, _c) = ws_with(
        &[
            ("src/a.ts", "import { b } from './b';\nexport function a() { return b(); }\n"),
            ("src/b.ts", "import { a } from './a';\nimport { z } from './z';\nexport function b() { return a(); }\n"),
            ("src/z.ts", "export const z = 1;\nexport const y = 2;\n"),
            ("m.py", "def _unused():\n    return 1\n"),
        ],
        "[health]\nmax_file_lines = 1\n",
    );
    let run = || cc(ws.path()).args(["health", "r"]).assert().success().get_output().stdout.clone();
    let (a, b) = (run(), run());
    assert!(!a.is_empty());
    assert_eq!(a, b);
    let v: Value = serde_json::from_slice(&a).unwrap();
    assert!(v["data"]["summary"]["total"].as_u64().unwrap() >= 3);
}

#[test]
fn bad_config_and_arguments_are_refused() {
    let (ws, _c) = ws_with(&[("a.ts", "export const a = 1;\n")], "[health]\nbogus = 3\n");
    cc(ws.path()).args(["health", "r"]).assert().code(1);
    let (ws, _c) = ws_with(&[("a.ts", "export const a = 1;\n")], "");
    cc(ws.path()).args(["health", "r", "--limit", "0"]).assert().code(1);
    cc(ws.path()).args(["health", "nope"]).assert().code(1);
}

#[test]
fn the_dead_code_section_joins_the_ratchet_only_when_asked() {
    let (ws, _c) = ws_with(&[("m.py", "def _unused():\n    return 1\n")], "");
    let b = ws.path().join("b.json");
    json(cc(ws.path()).args(["health", "r", "--baseline", b.to_str().unwrap()]).assert().success());
    assert_eq!(serde_json::from_str::<Value>(&std::fs::read_to_string(&b).unwrap()).unwrap()["keys"].as_array().unwrap().len(), 0);
    json(cc(ws.path()).args(["health", "r", "--ratchet-dead-code", "--baseline", b.to_str().unwrap()]).assert().success());
    assert_eq!(serde_json::from_str::<Value>(&std::fs::read_to_string(&b).unwrap()).unwrap()["keys"].as_array().unwrap().len(), 1);
}
