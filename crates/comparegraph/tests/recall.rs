use serde_json::Value;
use std::process::Command;

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

fn ws_with(files: &[(&str, &str)]) -> (tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let c = clone_with(files);
    cc(ws.path())
        .args(["add", c.path().to_str().unwrap(), "--name", "r"])
        .assert()
        .success();
    cc(ws.path()).args(["build", "r"]).assert().success();
    (ws, c)
}
fn qns(v: &Value) -> Vec<String> {
    v["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["qualified_name"].as_str().unwrap().to_string())
        .collect()
}

fn callees(ws: &std::path::Path, from: &str) -> Vec<(String, f64)> {
    let v = json(cc(ws).args(["traverse", "r", from, "--direction", "callees", "--depth", "1"]).assert().success());
    v["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["qualified_name"].as_str().unwrap().to_string(), i["path_confidence"].as_f64().unwrap()))
        .collect()
}
fn dead(ws: &std::path::Path) -> Vec<String> {
    qns(&json(cc(ws).args(["dead-code", "r"]).assert().success()))
}

#[test]
fn python_main_guard_call_has_the_file_as_caller_at_1_0() {
    let (ws, _c) = ws_with(&[("app.py", "def main():\n    pass\n\nif __name__ == \"__main__\":\n    main()\n    print(1)\n")]);
    let c = callees(ws.path(), "app.py");
    assert!(c.contains(&("app.py::main".to_string(), 1.0)), "{c:?}");
    assert_eq!(c.len(), 1, "print is a builtin: {c:?}");
}

#[test]
fn python_decorator_call_and_module_table_reference_bind_only_known_functions() {
    let (ws, _c) = ws_with(&[
        ("reg.py", "def register(name):\n    return lambda f: f\n\ndef handle_x():\n    pass\n\ndef handle_y():\n    pass\n\n@register(\"x\")\ndef thing():\n    pass\n\nHANDLERS = {\"x\": handle_x}\nOTHER = [handle_y, undefined_name]\n"),
        ("use.py", "from reg import handle_x\n\nTABLE = {\"a\": handle_x, \"b\": not_imported}\n"),
        ("far.py", "def not_imported():\n    pass\n"),
    ]);
    let c = callees(ws.path(), "reg.py");
    for want in ["reg.py::register", "reg.py::handle_x", "reg.py::handle_y"] {
        assert!(c.contains(&(want.to_string(), 1.0)), "{want} in {c:?}");
    }
    assert_eq!(c.len(), 3, "{c:?}");
    // a reference is bound at 1.0 through an import, and is never a name-only guess
    let u = callees(ws.path(), "use.py");
    assert_eq!(u, vec![("reg.py::handle_x".to_string(), 1.0)], "{u:?}");
}

#[test]
fn a_function_referenced_only_from_a_table_in_a_non_entry_file_stays_a_dead_code_candidate() {
    let (ws, _c) = ws_with(&[
        ("handlers.py", "def _handle_x():\n    pass\n\nHANDLERS = {\"x\": _handle_x}\n"),
        ("main.py", "def _serve():\n    pass\n\nSERVERS = [_serve]\n_serve()\n"),
    ]);
    let d = dead(ws.path());
    assert!(d.contains(&"handlers.py::_handle_x".to_string()), "{d:?}");
    assert!(!d.contains(&"main.py::_serve".to_string()), "an entry file rescues what it calls: {d:?}");
}

#[test]
fn a_main_guard_makes_any_file_an_entry_file() {
    let (ws, _c) = ws_with(&[
        ("tool.py", "def _run():\n    pass\n\ndef _unused():\n    pass\n\nif __name__ == \"__main__\":\n    _run()\n"),
        ("lib.py", "def _helper():\n    pass\n\n_helper()\n"),
    ]);
    let d = dead(ws.path());
    assert!(!d.contains(&"tool.py::_run".to_string()), "{d:?}");
    assert!(d.contains(&"tool.py::_unused".to_string()), "{d:?}");
    assert!(d.contains(&"lib.py::_helper".to_string()), "called only at module level of a non-entry file: {d:?}");
}

#[test]
fn typescript_top_level_calls_and_tables() {
    let (ws, _c) = ws_with(&[
        ("src/router.ts", "export function router() { return 1; }\nexport function onData() {}\n"),
        ("src/server.ts", "import { router, onData } from './router';\nconst app = makeApp();\napp.use(router());\nconst handlers = { data: onData };\nfunction makeApp() { return {}; }\nfunction inner() { local(); }\nfunction local() {}\n"),
    ]);
    let c = callees(ws.path(), "src/server.ts");
    for want in ["src/router.ts::router", "src/router.ts::onData", "src/server.ts::makeApp"] {
        assert!(c.contains(&(want.to_string(), 1.0)), "{want} in {c:?}");
    }
    assert!(!c.iter().any(|(q, _)| q.ends_with("::local")), "a call inside a function belongs to the function: {c:?}");
}

#[test]
fn unknown_receivers_and_name_only_matches_stay_unbound_at_module_level() {
    let (ws, _c) = ws_with(&[
        ("a.py", "import os\nx = other.thing()\nos.getcwd()\nelsewhere()\n"),
        ("b.py", "def thing():\n    pass\n\ndef getcwd():\n    pass\n"),
        ("c.py", "def elsewhere():\n    pass\n"),
    ]);
    let c = callees(ws.path(), "a.py");
    assert!(!c.iter().any(|(q, _)| q.starts_with("b.py")), "{c:?}");
    // the name-only match for a bare call is allowed but never 1.0
    assert!(c.iter().all(|(_, conf)| *conf < 1.0), "{c:?}");
}

#[test]
fn go_rust_and_java_module_level_calls() {
    let (ws, _c) = ws_with(&[
        ("g/main.go", "package main\n\nvar cfg = load()\n\nfunc load() int { return 1 }\n\nfunc main() {}\n"),
        ("r/src/lib.rs", "static CFG: i32 = build();\nfn build() -> i32 { 1 }\nfn other() { build(); }\n"),
        ("j/App.java", "class App {\n    static int V = compute();\n    static { boot(); }\n    static int compute() { return 1; }\n    static void boot() {}\n}\n"),
    ]);
    assert!(callees(ws.path(), "g/main.go").contains(&("g/main.go::load".to_string(), 1.0)));
    assert!(callees(ws.path(), "r/src/lib.rs").contains(&("r/src/lib.rs::build".to_string(), 1.0)));
    let j = callees(ws.path(), "j/App.java");
    for want in ["j/App.java::compute", "j/App.java::boot"] {
        assert!(j.contains(&(want.to_string(), 1.0)), "{want} in {j:?}");
    }
}

#[test]
fn a_table_reference_never_binds_a_method() {
    let (ws, _c) = ws_with(&[
        ("m.py", "class K:\n    def items(self):\n        pass\n\nx = items\ny = [items]\n"),
        ("m.ts", "class K { items() {} }\nconst a = [items];\nconst b = { items };\n"),
    ]);
    assert_eq!(callees(ws.path(), "m.py"), vec![], "python");
    assert_eq!(callees(ws.path(), "m.ts"), vec![], "typescript");
}

#[test]
fn a_table_reference_through_a_module_alias_binds_at_1_0() {
    let (ws, _c) = ws_with(&[
        ("m.py", "def f():\n    pass\n"),
        ("t.py", "import m\nTABLE = {\"x\": m.f, \"y\": other.f}\n"),
    ]);
    assert_eq!(callees(ws.path(), "t.py"), vec![("m.py::f".to_string(), 1.0)]);
}

#[test]
fn single_line_main_guard_and_double_extension_stems() {
    let (ws, _c) = ws_with(&[
        ("tool.py", "def _run():\n    pass\n\nif __name__ == \"__main__\": _run()\n"),
        ("cli.config.py", "def _h():\n    pass\n\n_h()\n"),
    ]);
    let d = dead(ws.path());
    assert!(!d.contains(&"tool.py::_run".to_string()), "{d:?}");
    assert!(d.contains(&"cli.config.py::_h".to_string()), "cli.config is not the entry stem cli: {d:?}");
    let b = json(cc(ws.path()).args(["build", "r"]).assert().success());
    let s = b.to_string();
    assert!(s.contains("entry_file_heuristic"), "{s}");
    assert_eq!(s.matches("entry_file_heuristic").count(), 1, "once per run");
}
