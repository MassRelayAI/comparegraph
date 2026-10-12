//! Call recall by KNOWN types and paths (Plan 4 fix wave B): same-crate and workspace Rust types, Python declared receivers and
//! aliases, inline Rust tests, `it()` bodies, minified files. Each case also pins what must stay unbound.
use super::ReferenceProvider;
use crate::domain::EdgeKind;
use crate::providers::{BuildOptions, BuildOutput, GraphProvider};

fn build(files: &[(&str, &str)]) -> BuildOutput {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let full = d.path().join(p);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, c).unwrap();
    }
    ReferenceProvider.build(d.path(), &BuildOptions::default()).unwrap()
}
fn has(o: &BuildOutput, k: EdgeKind, a: &str, b: &str) -> bool {
    o.edges_by_qn.iter().any(|e| e.0 == k && e.1 == a && e.2 == b)
}
fn conf(o: &BuildOutput, k: EdgeKind, a: &str, b: &str) -> Option<f64> {
    o.edges_by_qn.iter().find(|e| e.0 == k && e.1 == a && e.2 == b).map(|e| e.3)
}

#[test]
fn rust_assoc_call_on_a_type_of_the_same_crate_without_use() {
    let o = build(&[
        ("Cargo.toml", "[package]\nname = \"demo\"\n"),
        ("src/lib.rs", "pub mod store;\npub mod app;\n"),
        ("src/store.rs", "pub struct Store;\nimpl Store {\n    pub fn new() -> Store { Store }\n}\n"),
        ("src/app.rs", "pub fn run() {\n    let s = Store::new();\n    let v = Vec::new();\n}\n"),
    ]);
    assert_eq!(conf(&o, EdgeKind::Calls, "src/app.rs::run", "src/store.rs::new"), Some(1.0), "{:?}", o.edges_by_qn);
    // `Vec::new()` is no type of ours: it never binds to `Store::new`
    assert_eq!(o.edges_by_qn.iter().filter(|e| e.0 == EdgeKind::Calls && e.1 == "src/app.rs::run").count(), 1);
}

#[test]
fn rust_crate_name_paths_resolve_in_tests_and_across_workspace_crates() {
    let o = build(&[
        ("Cargo.toml", "[package]\nname = \"demo-app\"\n"),
        ("src/lib.rs", "pub mod util;\n"),
        ("src/util.rs", "pub fn double(x: f64) -> f64 { x }\n"),
        ("tests/util.rs", "use demo_app::util::double;\nfn check() { double(1.0); }\n"),
        ("crates/db/Cargo.toml", "[package]\nname = \"gm-db\"\n"),
        ("crates/db/src/lib.rs", "pub mod queries;\n"),
        ("crates/db/src/queries.rs", "pub struct Q;\nimpl Q {\n    pub fn new() -> Q { Q }\n}\n"),
        ("crates/cli/Cargo.toml", "[package]\nname = \"gm-cli\"\n"),
        ("crates/cli/src/main.rs", "use gm_db::queries::Q;\nfn main() { let q = Q::new(); }\n"),
        ("crates/cli/tests/e2e.rs", "mod common;\nuse common::Env;\n#[test]\nfn t() { let e = Env::new(); }\n"),
        ("crates/cli/tests/common/mod.rs", "pub struct Env;\nimpl Env {\n    pub fn new() -> Env { Env }\n}\n"),
    ]);
    assert!(has(&o, EdgeKind::Imports, "tests/util.rs", "src/util.rs"), "{:?}", o.edges_by_qn);
    assert!(has(&o, EdgeKind::Calls, "tests/util.rs::check", "src/util.rs::double"));
    assert!(has(&o, EdgeKind::Calls, "crates/cli/src/main.rs::main", "crates/db/src/queries.rs::new"));
    assert!(has(&o, EdgeKind::Calls, "crates/cli/tests/e2e.rs::t", "crates/cli/tests/common/mod.rs::new"));
}

#[test]
fn rust_test_functions_and_cfg_test_mods_link_to_what_they_call() {
    let o = build(&[(
        "src/lib.rs",
        "pub fn parse(x: u8) -> u8 { x }\npub fn other() {}\n#[cfg(test)]\nmod tests {\n    use super::*;\n    fn helper() { parse(1); }\n}\n#[test]\nfn direct() {\n    other();\n}\n",
    )]);
    assert!(has(&o, EdgeKind::Tests, "src/lib.rs::helper", "src/lib.rs::parse"), "{:?}", o.edges_by_qn);
    assert!(has(&o, EdgeKind::Tests, "src/lib.rs::direct", "src/lib.rs::other"));
    // production code is not test code
    assert!(!o.edges_by_qn.iter().any(|e| e.0 == EdgeKind::Tests && e.1 == "src/lib.rs::parse"));
}

const IDX: &str = "class Indexer:\n    def index(self):\n        return 1\n\nclass Database:\n    def close(self):\n        pass\n    def open(self):\n        pass\n";

#[test]
fn python_declared_constructed_and_chained_receivers_bind() {
    let o = build(&[
        ("pkg/idx.py", IDX),
        (
            "pkg/use.py",
            "from pkg.idx import Indexer, Database\n\n_db: Database | None = None\n\ndef a(d: Database):\n    d.close()\n\ndef b():\n    with Database() as w:\n        w.open()\n\ndef c():\n    x = Indexer()\n    x.index()\n\ndef d():\n    Indexer().index()\n\ndef e():\n    _db.close()\n\ndef f(other):\n    other.close()\n    y = make()\n    y.index()\n",
        ),
    ]);
    for (src, tgt) in [("a", "close"), ("b", "open"), ("c", "index"), ("d", "index"), ("e", "close")] {
        assert_eq!(conf(&o, EdgeKind::Calls, &format!("pkg/use.py::{src}"), &format!("pkg/idx.py::{tgt}")), Some(1.0), "{src}: {:?}", o.edges_by_qn);
    }
    // an untyped parameter and an untyped local bind to nothing
    assert!(!o.edges_by_qn.iter().any(|e| e.0 == EdgeKind::Calls && e.1 == "pkg/use.py::f" && e.2.starts_with("pkg/idx.py")), "{:?}", o.edges_by_qn);
}

#[test]
fn python_untyped_parameter_shadows_a_typed_module_variable() {
    let o = build(&[
        ("pkg/idx.py", IDX),
        ("pkg/use.py", "from pkg.idx import Database\n_db: Database = Database()\n\ndef f(_db):\n    _db.close()\n\ndef g():\n    _db.close()\n"),
    ]);
    assert!(!has(&o, EdgeKind::Calls, "pkg/use.py::f", "pkg/idx.py::close"), "{:?}", o.edges_by_qn);
    assert!(has(&o, EdgeKind::Calls, "pkg/use.py::g", "pkg/idx.py::close"));
}

#[test]
fn python_module_and_function_aliases_bind() {
    let o = build(&[
        ("pkg/build.py", "def get_variants(r):\n    return r\n\ndef other():\n    pass\n"),
        ("pkg/use.py", "import pkg.build as build_mod\nfrom pkg.build import other as o2\n\ndef f():\n    build_mod.get_variants(1)\n    o2()\n\ndef g(z):\n    z.get_variants(1)\n"),
    ]);
    assert!(has(&o, EdgeKind::Calls, "pkg/use.py::f", "pkg/build.py::get_variants"), "{:?}", o.edges_by_qn);
    assert!(has(&o, EdgeKind::Calls, "pkg/use.py::f", "pkg/build.py::other"));
    assert!(!has(&o, EdgeKind::Calls, "pkg/use.py::g", "pkg/build.py::get_variants"));
}

#[test]
fn it_bodies_become_tests_edges_from_the_test_file() {
    let o = build(&[
        ("src/user.ts", "export class UserService {\n  getUser(id: number) { return id; }\n}\nexport function parse(x: number) { return x; }\n"),
        (
            "src/user.test.ts",
            "import { UserService, parse } from './user';\ndescribe('u', () => {\n  it('gets', () => {\n    const service = new UserService();\n    const u = service.getUser(1);\n    expect(parse(u)).toBe(1);\n  });\n});\n",
        ),
    ]);
    for t in ["src/user.ts::getUser", "src/user.ts::parse"] {
        assert!(has(&o, EdgeKind::Tests, "src/user.test.ts", t), "{t}: {:?}", o.edges_by_qn);
    }
    // the runner's own `expect(...)`/`toBe` bind to nothing and no calls edge hangs off the file node
    assert!(!o.edges_by_qn.iter().any(|e| e.0 == EdgeKind::Calls && e.1 == "src/user.test.ts"));
}

#[test]
fn minified_files_are_skipped_by_name_and_by_line_length() {
    let long = format!("function a(){{{}}}\n", "x=1;".repeat(2000));
    let o = build(&[
        ("src/ok.js", "function f() {}\n"),
        ("src/d3.min.js", "function m() {}\n"),
        ("src/app.bundle.js", "function b() {}\n"),
        ("src/gen.js", &long),
    ]);
    let names: Vec<_> = o.nodes.iter().map(|n| n.qualified_name.as_str()).filter(|q| !q.contains("::")).collect();
    assert_eq!(names, vec!["src/ok.js"], "{names:?}");
    let l = o.limitations.iter().find(|l| l.code == "skipped_minified").expect("limitation");
    assert!(l.detail.starts_with("3 "), "{}", l.detail);
}

#[test]
fn python_private_class_return_annotation_and_package_absolute_import() {
    let o = build(&[
        ("src/pk/db.py", "class Database:\n    def stats(self):\n        return 1\n\nclass _Line:\n    def total(self):\n        return 2\n\ndef get_db() -> Database:\n    return Database()\n"),
        (
            "src/pk/server.py",
            "from pk.db import get_db, _Line\n\ndef status():\n    db = get_db()\n    return db.stats()\n\ndef timed():\n    with _Line() as line:\n        return line.total()\n\ndef untyped(make):\n    db = make()\n    return db.stats()\n",
        ),
    ]);
    assert!(has(&o, EdgeKind::Imports, "src/pk/server.py", "src/pk/db.py"), "{:?}", o.edges_by_qn);
    assert!(has(&o, EdgeKind::Calls, "src/pk/server.py::status", "src/pk/db.py::stats"));
    assert!(has(&o, EdgeKind::Calls, "src/pk/server.py::timed", "src/pk/db.py::total"));
    // `make` has no return annotation: `db` has no type and `db.stats()` binds to nothing
    assert!(!has(&o, EdgeKind::Calls, "src/pk/server.py::untyped", "src/pk/db.py::stats"));
}

#[test]
fn python_global_declared_module_variable_and_cls_calls() {
    let o = build(&[
        ("pkg/db.py", "class Database:\n    def close(self):\n        pass\n"),
        (
            "pkg/srv.py",
            "from pkg.db import Database\n\n_db: Database | None = None\n\ndef configure():\n    global _db\n    if _db is not None:\n        _db.close()\n    _db = None\n\nclass W:\n    @classmethod\n    def path(cls):\n        return 1\n\n    @classmethod\n    def read(cls):\n        return cls.path()\n",
        ),
    ]);
    assert!(has(&o, EdgeKind::Calls, "pkg/srv.py::configure", "pkg/db.py::close"), "{:?}", o.edges_by_qn);
    assert!(has(&o, EdgeKind::Calls, "pkg/srv.py::read", "pkg/srv.py::path"));
}

#[test]
fn a_local_variable_hides_a_module_alias_and_a_function_name_is_not_a_type() {
    let o = build(&[
        ("pkg/daemon.py", "def reconcile(c):\n    return c\n"),
        ("pkg/use.py", "from pkg import daemon\n\ndef ok():\n    daemon.reconcile(1)\n\ndef shadowed(env):\n    daemon = env['d']\n    daemon.reconcile(1)\n"),
        ("pkg/util.py", "def normalise(s):\n    return s\n"),
        ("pkg/call.py", "def uses(normalise=None):\n    return normalise(1)\n\ndef plain():\n    return normalise(1)\n\ndef fix():\n    return 1\n\ndef t(fix):\n    return fix()\n"),
        ("src/a.rs", "fn hooks_dir() {}\nfn f(p: u8) {\n    let hooks_dir = g(p);\n    hooks_dir.join(1);\n}\n"),
    ]);
    assert!(has(&o, EdgeKind::Calls, "pkg/use.py::ok", "pkg/daemon.py::reconcile"), "{:?}", o.edges_by_qn);
    assert!(!has(&o, EdgeKind::Calls, "pkg/use.py::shadowed", "pkg/daemon.py::reconcile"));
    // a parameter called by name is not the repo function of that name in another file; an unshadowed call still binds
    assert!(!has(&o, EdgeKind::Calls, "pkg/call.py::uses", "pkg/util.py::normalise"), "{:?}", o.edges_by_qn);
    assert!(has(&o, EdgeKind::Calls, "pkg/call.py::plain", "pkg/util.py::normalise"));
    assert!(has(&o, EdgeKind::Calls, "pkg/call.py::t", "pkg/call.py::fix"));
    assert!(!o.edges_by_qn.iter().any(|e| e.0 == EdgeKind::Calls && e.1 == "src/a.rs::f" && e.2 == "src/a.rs::hooks_dir"), "{:?}", o.edges_by_qn);
}
