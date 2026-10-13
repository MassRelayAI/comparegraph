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

#[test]
fn python_init_reexport_resolves_in_both_call_shapes() {
    let (ws, _c) = ws_with(&[
        ("pkg/__init__.py", "from .impl import run\n"),
        ("pkg/impl.py", "def run():\n    pass\n"),
        ("other/run.py", "def run():\n    pass\n"),
        ("a.py", "from pkg import run\n\ndef go():\n    run()\n"),
        ("b.py", "import pkg\n\ndef go():\n    pkg.run()\n"),
    ]);
    for f in ["a.py::go", "b.py::go"] {
        let c = callees(ws.path(), f);
        assert_eq!(c, vec![("pkg/impl.py::run".to_string(), 1.0)], "{f}: {c:?}");
    }
}

#[test]
fn python_two_level_reexport_chain_and_a_cycle_terminates() {
    let (ws, _c) = ws_with(&[
        ("top/__init__.py", "from .mid import run\n"),
        ("top/mid/__init__.py", "from .impl import run\n"),
        ("top/mid/impl.py", "def run():\n    pass\n"),
        ("a.py", "from top import run\n\ndef go():\n    run()\n"),
        ("ca/__init__.py", "from cb import thing\n"),
        ("cb/__init__.py", "from ca import thing\n"),
        ("c.py", "from ca import thing\n\ndef go():\n    thing()\n"),
    ]);
    let c = callees(ws.path(), "a.py::go");
    assert_eq!(c, vec![("top/mid/impl.py::run".to_string(), 1.0)], "{c:?}");
    assert!(callees(ws.path(), "c.py::go").iter().all(|(_, conf)| *conf < 1.0));
}

#[test]
fn python_name_used_as_object_binds_a_class_or_instance_never_a_function() {
    let (ws, _c) = ws_with(&[
        ("m.py", "class Ring:\n    def since(self):\n        pass\n\ndef ring():\n    pass\n\ndef since():\n    pass\n"),
        ("n.py", "class Ring:\n    def since(self):\n        pass\n\nring = Ring()\n"),
        ("a.py", "from m import ring\n\ndef go():\n    ring.since()\n"),
        ("b.py", "from n import ring\n\ndef go():\n    ring.since()\n"),
    ]);
    let a = callees(ws.path(), "a.py::go");
    assert!(a.is_empty(), "a function is no object: {a:?}");
    let b = callees(ws.path(), "b.py::go");
    assert_eq!(b, vec![("n.py::since".to_string(), 1.0)], "{b:?}");
}

#[test]
fn calls_from_describe_bodies_and_test_file_scope_do_not_rescue_dead_code_candidates() {
    // methods, because an exported TypeScript function with no caller is an entry point, never a dead-code candidate
    let (ws, _c) = ws_with(&[
        ("src/lib.ts", "export class Svc {\n  inDescribe() {}\n  atFileScope() {}\n  inACase() {}\n  nobody() {}\n}\n"),
        (
            "src/lib.test.ts",
            "import { Svc } from './lib';\nconst s = new Svc();\ndescribe('x', () => {\n  s.inDescribe();\n  it('y', () => { s.inACase(); });\n});\ns.atFileScope();\n",
        ),
    ]);
    // the edges exist ...
    let c = callees(ws.path(), "src/lib.test.ts::describe: x");
    assert!(c.contains(&("src/lib.ts::inDescribe".to_string(), 1.0)), "{c:?}");
    // ... but only an `it:` case makes code live: a describe body and test-file scope run at load time
    let d = dead(ws.path());
    assert!(d.contains(&"src/lib.ts::nobody".to_string()), "control: {d:?}");
    assert!(d.contains(&"src/lib.ts::inDescribe".to_string()), "{d:?}");
    assert!(d.contains(&"src/lib.ts::atFileScope".to_string()), "{d:?}");
    assert!(!d.contains(&"src/lib.ts::inACase".to_string()), "{d:?}");
}

fn go_pkgs() -> Vec<(&'static str, &'static str)> {
    vec![
        ("go.mod", "module demo\n\ngo 1.21\n"),
        (
            "a.go",
            "package demo\n\nimport (\n\t\"demo/store\"\n\talias \"demo/other\"\n\t\"demo/tools/v3\"\n)\n\ntype Shape interface{ Area() int }\n\ntype Box struct{ w int }\n\nfunc (b *Box) Area() int { return b.w }\n\nfunc (b *Box) Grow() { b.Area() }\n\nfunc NewBox() *Box { return &Box{} }\n\nfunc Measure(s Shape) int { return s.Area() }\n\nfunc Convert(c Box) int {\n\tn := Box(c)\n\treturn n.w\n}\n\nfunc Build() {\n\tb := NewBox()\n\tb.Grow()\n\tvar c Box\n\tc.Area()\n\td := &Box{}\n\td.Area()\n\thelper()\n\tdefer helper()\n\tgo helper()\n\tstore.Save(1)\n\tr := store.NewRepo()\n\tr.Put()\n\tvar p *store.Repo\n\tp.Put()\n\tunknown().Put()\n\tx.Put()\n}\n\nfunc Viaalias() { alias.Save(2) }\n\nfunc Viaclause() { tools.Run() }\n",
        ),
        ("b.go", "package demo\n\nfunc helper() {}\n"),
        ("c_test.go", "package demo_test\n\nfunc helper() {}\n"),
        (
            "store/store.go",
            "package store\n\ntype Repo struct{}\n\nfunc NewRepo() *Repo { return &Repo{} }\n\nfunc (r *Repo) Put() { r.save() }\n\nfunc (r *Repo) save() {}\n\nfunc Save(n int) int { return n }\n",
        ),
        ("other/o.go", "package other\n\nfunc Save(n int) int { return n }\n"),
        ("tools/v3/t.go", "package tools\n\nfunc Run() {}\n"),
    ]
}

#[test]
fn go_receiver_methods_package_calls_and_the_things_that_bind_nothing() {
    let (ws, _c) = ws_with(&go_pkgs());
    let grow = callees(ws.path(), "a.go::Grow");
    assert_eq!(grow, vec![("a.go::Area".to_string(), 1.0)], "typed receiver: {grow:?}");
    let build = callees(ws.path(), "a.go::Build");
    for want in [
        ("a.go::NewBox", 1.0),
        ("a.go::Grow", 1.0),
        ("a.go::Area", 1.0),
        ("b.go::helper", 1.0),
        ("store/store.go::Save", 1.0),
        ("store/store.go::NewRepo", 1.0),
        ("store/store.go::Put", 1.0),
    ] {
        assert!(build.contains(&(want.0.to_string(), want.1)), "{want:?} missing in {build:?}");
    }
    assert_eq!(build.len(), 7, "nothing else: unknown receivers bind nothing: {build:?}");
    // an interface-typed parameter binds nothing (interface vs implementation ambiguity)
    let m = callees(ws.path(), "a.go::Measure");
    assert!(m.is_empty(), "interface receiver: {m:?}");
    // a type conversion is not a call
    let c = callees(ws.path(), "a.go::Convert");
    assert!(c.is_empty(), "conversion: {c:?}");
    // an explicit import alias, and a package named differently from its last path segment (bound by the package clause)
    let v = callees(ws.path(), "a.go::Viaalias");
    assert_eq!(v, vec![("other/o.go::Save".to_string(), 1.0)], "{v:?}");
    let t = callees(ws.path(), "a.go::Viaclause");
    assert_eq!(t, vec![("tools/v3/t.go::Run".to_string(), 1.0)], "{t:?}");
    // a method of the imported package calls its sibling method
    let put = callees(ws.path(), "store/store.go::Put");
    assert_eq!(put, vec![("store/store.go::save".to_string(), 1.0)], "{put:?}");
}

#[test]
fn go_type_parameters_shadow_package_types() {
    let (ws, _c) = ws_with(&[
        ("go.mod", "module demo\n\ngo 1.21\n"),
        (
            "a.go",
            "package demo\n\ntype T struct{}\n\nfunc (T) M() {}\n\nfunc Ok(x T) { x.M() }\n\nfunc Gen[T any](x T) { x.M() }\n\ntype Stack[T any] struct{ top T }\n\nfunc (s *Stack[T]) Push(x T) { x.M() }\n\nfunc (s *Stack[T]) Peek() { s.top.M() }\n",
        ),
    ]);
    let ok = callees(ws.path(), "a.go::Ok");
    assert_eq!(ok, vec![("a.go::M".to_string(), 1.0)], "control: a declared type binds: {ok:?}");
    for f in ["Gen", "Push", "Peek"] {
        let c = callees(ws.path(), &format!("a.go::{f}"));
        assert!(c.is_empty(), "{f}: a type parameter named like a package type binds nothing: {c:?}");
    }
}

#[test]
fn go_unresolvable_embed_makes_a_promoted_method_ambiguous() {
    let (ws, _c) = ws_with(&[
        ("go.mod", "module demo\n\ngo 1.21\n"),
        (
            "a.go",
            "package demo\n\nimport \"sync\"\n\ntype Inner struct{}\n\nfunc (Inner) Lock() {}\n\ntype Outer struct {\n\tsync.Mutex\n\tInner\n}\n\nfunc Use(o Outer) { o.Lock() }\n\nfunc (Inner) Own() {}\n\nfunc UseOwn(o Outer) { o.Own() }\n",
        ),
    ]);
    // sync.Mutex (not ours) may supply Lock too: bind nothing; Own can only come from Inner ... but the unreadable embed could also have it
    let u = callees(ws.path(), "a.go::Use");
    assert!(u.is_empty(), "{u:?}");
    let o = callees(ws.path(), "a.go::UseOwn");
    assert!(o.is_empty(), "{o:?}");
}

#[test]
fn go_explicit_alias_equal_to_the_last_segment_is_honoured() {
    let (ws, _c) = ws_with(&[
        ("go.mod", "module demo\n\ngo 1.21\n"),
        ("a.go", "package demo\n\nimport util \"demo/util\"\n\nfunc Use() { util.Do() }\n"),
        ("util/u.go", "package utils\n\nfunc Do() {}\n"),
    ]);
    let c = callees(ws.path(), "a.go::Use");
    assert_eq!(c, vec![("util/u.go::Do".to_string(), 1.0)], "{c:?}");
}
