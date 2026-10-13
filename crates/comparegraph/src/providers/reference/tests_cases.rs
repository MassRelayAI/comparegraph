//! Per-case test nodes (`it: <title>`) and same-family test links (Plan 5 A3).
use super::ReferenceProvider;
use crate::domain::{EdgeKind, NodeKind};
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

const SRC: &str = "export function parse(x: number) { return x; }\nexport function other() { return 1; }\n";

#[test]
fn an_it_case_is_a_node_with_a_tests_edge_to_what_it_calls() {
    let o = build(&[
        ("src/p.ts", SRC),
        ("src/p.test.ts", "import { parse } from './p';\nit('parses', () => {\n  parse(1);\n});\n"),
    ]);
    let q = "src/p.test.ts::it: parses";
    let n = o.nodes.iter().find(|n| n.qualified_name == q).expect("case node");
    assert_eq!((n.kind, n.name.as_str(), n.line_start, n.line_end), (NodeKind::Function, "it: parses", 2, 4));
    assert!(has(&o, EdgeKind::Contains, "src/p.test.ts", q));
    assert!(has(&o, EdgeKind::Tests, q, "src/p.ts::parse"), "{:?}", o.edges_by_qn);
    assert!(!has(&o, EdgeKind::Tests, "src/p.test.ts", "src/p.ts::parse"));
}

#[test]
fn nested_describe_prefixes_the_title_and_long_titles_are_cut() {
    let long = "x".repeat(120);
    let src = format!("import {{ parse }} from './p';\ndescribe('outer', () => {{\n  describe('inner', () => {{\n    it('works', () => {{ parse(1); }});\n  }});\n}});\ntest('{long}', () => {{ parse(2); }});\n");
    let o = build(&[("src/p.ts", SRC), ("src/p.test.ts", &src)]);
    assert!(o.nodes.iter().any(|n| n.qualified_name == "src/p.test.ts::it: outer > inner > works"), "{:?}", o.nodes.iter().map(|n| &n.qualified_name).collect::<Vec<_>>());
    let cut = o.nodes.iter().find(|n| n.name.starts_with("it: xxx")).expect("long case");
    assert_eq!(cut.name.chars().count(), 4 + 80);
    // Plan 6 A1 reverses the Plan 5 A3 decision: a describe block IS a node (`describe: <title>`)
    assert!(o.nodes.iter().any(|n| n.qualified_name == "src/p.test.ts::describe: outer > inner" && n.kind == NodeKind::Function));
}

#[test]
fn two_cases_with_one_title_get_a_numbered_suffix() {
    let o = build(&[
        ("src/p.ts", SRC),
        ("src/p.test.ts", "import { parse, other } from './p';\nit('same', () => { parse(1); });\nit('same', () => { other(); });\n"),
    ]);
    assert!(has(&o, EdgeKind::Tests, "src/p.test.ts::it: same", "src/p.ts::parse"));
    assert!(has(&o, EdgeKind::Tests, "src/p.test.ts::it: same#2", "src/p.ts::other"), "{:?}", o.edges_by_qn);
}

#[test]
fn a_top_level_call_in_a_test_file_stays_file_sourced() {
    let o = build(&[
        ("src/p.ts", SRC),
        ("src/p.test.ts", "import { parse } from './p';\nparse(0);\nit('x', () => {});\n"),
    ]);
    assert!(has(&o, EdgeKind::Tests, "src/p.test.ts", "src/p.ts::parse"), "{:?}", o.edges_by_qn);
    assert!(!o.edges_by_qn.iter().any(|e| e.1 == "src/p.test.ts::it: x"));
}

#[test]
fn naming_links_stay_within_a_language_family() {
    let o = build(&[
        ("src/x.ts", "export function a() {}\n"),
        ("tests/test_x.py", "def test_a():\n    pass\n"),
    ]);
    assert!(!o.edges_by_qn.iter().any(|e| e.0 == EdgeKind::Tests), "{:?}", o.edges_by_qn);
    let o = build(&[
        ("src/x.tsx", "export function a() {}\n"),
        ("src/x.test.ts", "export function t() {}\n"),
    ]);
    assert!(has(&o, EdgeKind::Tests, "src/x.test.ts", "src/x.tsx"), "ts and tsx are one family");
}

#[test]
fn a_rust_cfg_test_fn_calling_a_function_yields_a_tests_edge() {
    let o = build(&[
        ("Cargo.toml", "[package]\nname = \"demo\"\n"),
        (
            "src/lib.rs",
            "pub fn parse(x: u32) -> u32 { x }\n#[cfg(test)]\nmod tests {\n    use super::*;\n    #[test]\n    fn parses() {\n        parse(1);\n    }\n}\n",
        ),
    ]);
    assert!(has(&o, EdgeKind::Tests, "src/lib.rs::parses", "src/lib.rs::parse"), "{:?}", o.edges_by_qn);
}

fn parsed(path: &str, lang: &str, src: &str, with_cases: bool) -> super::ParsedFile {
    let r = super::vendor::parse(path, src, lang).unwrap();
    let mut symbols = r.symbols;
    if with_cases {
        symbols.extend(super::js_ctx::test_cases(src, lang).0.into_iter().map(|c| super::vendor::Symbol {
            name: c.name,
            kind: super::vendor::SymbolKind::Function,
            line_start: c.line_start,
            line_end: c.line_end,
            signature: None,
            doc: None,
        }));
    }
    super::ParsedFile { rel_path: path.into(), language: lang.into(), lines: src.lines().map(String::from).collect(), symbols, call_sites: r.call_sites, imports: r.imports }
}

#[test]
fn case_calls_bind_by_name_and_line_not_by_position() {
    let t = "import { parse, other } from './p';\nit('a', () => { parse(1); });\nit('b', () => { other(); });\n";
    let mut tf = parsed("src/p.test.ts", "typescript", t, true);
    // reverse the case symbols and put an unrelated symbol after them
    tf.symbols.reverse();
    tf.symbols.push(super::vendor::Symbol { name: "tail".into(), kind: super::vendor::SymbolKind::Function, line_start: 9, line_end: 9, signature: None, doc: None });
    let files = vec![parsed("src/p.ts", "typescript", SRC, false), tf];
    let e = super::resolve::resolve_edges(&files);
    let tests = |a: &str, b: &str| e.iter().any(|x| x.0 == EdgeKind::Calls && x.1 == a && x.2 == b);
    assert!(tests("src/p.test.ts::it: a", "src/p.ts::parse"), "{e:?}");
    assert!(tests("src/p.test.ts::it: b", "src/p.ts::other"), "{e:?}");
    assert!(!tests("src/p.test.ts::it: a", "src/p.ts::other"));
}

#[test]
fn case_calls_without_case_symbols_make_no_edges() {
    let t = "import { parse } from './p';\nit('a', () => { parse(1); });\n";
    let files = vec![parsed("src/p.ts", "typescript", SRC, false), parsed("src/p.test.ts", "typescript", t, false)];
    let e = super::resolve::resolve_edges(&files);
    assert!(!e.iter().any(|x| x.0 == EdgeKind::Calls && x.2 == "src/p.ts::parse"), "{e:?}");
}

#[test]
fn playwright_test_members_are_not_cases() {
    let t = "import { parse } from './p';\ntest.describe('grp', () => {\n  test.beforeEach(() => { parse(0); });\n  test('real', async () => {\n    await test.step('s', async () => { parse(1); });\n  });\n  test.skip('skipped', () => { parse(2); });\n});\n";
    let o = build(&[("src/p.ts", SRC), ("src/p.test.ts", t)]);
    let names: Vec<_> = o.nodes.iter().filter(|n| n.name.starts_with("it:")).map(|n| n.name.as_str()).collect();
    assert_eq!(names, vec!["it: grp > real", "it: grp > skipped"], "{names:?}");
    assert!(has(&o, EdgeKind::Tests, "src/p.test.ts::it: grp > real", "src/p.ts::parse"));
    assert!(has(&o, EdgeKind::Tests, "src/p.test.ts::describe: grp", "src/p.ts::parse"), "a hook in a describe body belongs to that describe (Plan 6 A1)");
}

#[test]
fn multibyte_titles_are_cut_on_a_char_boundary_and_repeated_describes_are_numbered() {
    let title = "é".repeat(100);
    let t = format!("import {{ parse }} from './p';\nit('{title}', () => {{ parse(1); }});\ndescribe('d', () => {{ it('x', () => {{ parse(1); }}); }});\ndescribe('d', () => {{ it('x', () => {{ parse(2); }}); }});\n");
    let o = build(&[("src/p.ts", SRC), ("src/p.test.ts", &t)]);
    let cut = o.nodes.iter().find(|n| n.name.starts_with("it: é")).expect("multibyte case");
    assert_eq!(cut.name.chars().count(), 4 + 80);
    assert!(o.nodes.iter().any(|n| n.qualified_name == "src/p.test.ts::it: d > x"));
    assert!(o.nodes.iter().any(|n| n.qualified_name == "src/p.test.ts::it: d > x#2"));
}

const DESC: &str = "import { parse, other } from './p';\ndescribe('outer', () => {\n  parse(1);\n  describe('inner', () => {\n    other();\n    it('works', () => { parse(2); });\n  });\n});\ndescribe('outer', () => {});\nparse(3);\n";

#[test]
fn a_describe_block_is_a_node_and_owns_the_calls_in_its_body() {
    let o = build(&[("src/p.ts", SRC), ("src/p.test.ts", DESC)]);
    let outer = "src/p.test.ts::describe: outer";
    let inner = "src/p.test.ts::describe: outer > inner";
    let n = o.nodes.iter().find(|n| n.qualified_name == outer).expect("describe node");
    assert_eq!((n.kind, n.name.as_str(), n.line_start, n.line_end), (NodeKind::Function, "describe: outer", 2, 8));
    assert!(has(&o, EdgeKind::Calls, outer, "src/p.ts::parse"), "{:?}", o.edges_by_qn);
    assert!(!has(&o, EdgeKind::Calls, outer, "src/p.ts::other"), "inner's call is inner's");
    assert!(has(&o, EdgeKind::Calls, inner, "src/p.ts::other"));
    assert!(has(&o, EdgeKind::Calls, "src/p.test.ts::it: outer > inner > works", "src/p.ts::parse"));
    // module level still belongs to the file
    assert!(has(&o, EdgeKind::Tests, "src/p.test.ts", "src/p.ts::parse"));
    // containment: file > outer > inner > case; a repeated title is numbered
    assert!(has(&o, EdgeKind::Contains, "src/p.test.ts", outer));
    assert!(has(&o, EdgeKind::Contains, outer, inner));
    assert!(has(&o, EdgeKind::Contains, inner, "src/p.test.ts::it: outer > inner > works"));
    assert!(!has(&o, EdgeKind::Contains, "src/p.test.ts", inner));
    assert!(has(&o, EdgeKind::Contains, "src/p.test.ts", "src/p.test.ts::describe: outer#2"), "{:?}", o.edges_by_qn);
}

#[test]
fn suite_context_and_test_describe_are_describe_blocks_and_titles_are_cut_on_a_char_boundary() {
    let long = "é".repeat(120);
    let src = format!("import {{ parse }} from './p';\nsuite('s', () => {{ parse(1); }});\ncontext('c', () => {{ parse(1); }});\ntest.describe('t', () => {{ parse(1); }});\ndescribe('{long}', () => {{ parse(1); }});\n");
    let o = build(&[("src/p.ts", SRC), ("src/p.test.ts", &src)]);
    for n in ["s", "c", "t"] {
        assert!(has(&o, EdgeKind::Calls, &format!("src/p.test.ts::describe: {n}"), "src/p.ts::parse"), "{n}");
    }
    let cut = o.nodes.iter().find(|n| n.name.starts_with("describe: éé")).expect("long describe");
    assert_eq!(cut.name.chars().count(), "describe: ".len() + 80);
}

#[test]
fn a_jsx_element_is_a_rendering_call_to_an_imported_or_same_file_component_only() {
    let o = build(&[
        ("src/Button.tsx", "export function Button() { return null; }\n"),
        ("src/Other.tsx", "export function Stray() { return null; }\n"),
        (
            "src/App.tsx",
            "import { Button } from './Button';\nfunction Local() { return null; }\nexport function App() {\n  return <div><Button /><Local>x</Local><Stray /><Ghost.Part /></div>;\n}\nexport const Page = () => <Button />;\n",
        ),
    ]);
    let app = "src/App.tsx::App";
    assert!(o.edges_by_qn.contains(&(EdgeKind::Calls, app.into(), "src/Button.tsx::Button".into(), 1.0)), "{:?}", o.edges_by_qn);
    assert!(o.edges_by_qn.contains(&(EdgeKind::Calls, app.into(), "src/App.tsx::Local".into(), 1.0)));
    assert!(o.edges_by_qn.iter().all(|e| !(e.1 == app && e.2.contains("Stray"))), "an unimported name binds nothing");
    assert!(o.edges_by_qn.iter().all(|e| !(e.1 == app && e.2.ends_with("::div"))));
    // an arrow bound to a const is no function node in the vendored extractor: its elements belong to the file
    assert!(o.edges_by_qn.contains(&(EdgeKind::Calls, "src/App.tsx".into(), "src/Button.tsx::Button".into(), 1.0)), "{:?}", o.edges_by_qn);
}

#[test]
fn a_hook_resolves_like_any_call() {
    let o = build(&[
        ("src/hooks.ts", "export function useThing() { return 1; }\n"),
        ("src/C.tsx", "import { useThing } from './hooks';\nexport function C() { const t = useThing(); return t; }\n"),
    ]);
    assert!(o.edges_by_qn.contains(&(EdgeKind::Calls, "src/C.tsx::C".into(), "src/hooks.ts::useThing".into(), 1.0)));
}

#[test]
fn a_class_field_arrow_is_a_method_and_an_imported_class_reaches_it_through_its_base() {
    let o = build(&[
        ("src/base.ts", "export class Core {\n  request = (p: string) => p;\n  other() {}\n}\n"),
        ("src/app.ts", "import { Core } from './base';\nexport class App extends Core {}\n"),
        ("src/decoy.ts", "export class App {\n  request() {}\n}\n"),
        ("src/app.test.ts", "import { App } from './app';\nit('r', () => {\n  const app = new App();\n  app.request('/');\n});\n"),
    ]);
    let m = o.nodes.iter().find(|n| n.qualified_name == "src/base.ts::request").expect("field method");
    assert_eq!(m.kind, NodeKind::Method);
    // the import decides which `App` is meant; the same-named decoy class is not it
    assert!(has(&o, EdgeKind::Calls, "src/app.test.ts::it: r", "src/base.ts::request"), "{:?}", o.edges_by_qn);
    assert!(!has(&o, EdgeKind::Calls, "src/app.test.ts::it: r", "src/decoy.ts::request"));
}

#[test]
fn a_local_that_hides_an_imported_class_binds_nothing() {
    let o = build(&[
        ("src/a.ts", "export class Base {\n  static go() {}\n}\n"),
        (
            "src/s.ts",
            "import { Base } from './a';\nexport function viaConst() { const Base = make(); Base.go(); }\nexport function viaParam(Base: any) { Base.go(); }\nexport function viaCatch() { try { run(); } catch (Base) { Base.go(); } }\nexport function viaDestructure() { const { Base } = make(); Base.go(); }\nexport function real() { Base.go(); }\n",
        ),
    ]);
    for f in ["viaConst", "viaParam", "viaCatch", "viaDestructure"] {
        assert!(!o.edges_by_qn.iter().any(|e| e.0 == EdgeKind::Calls && e.1 == format!("src/s.ts::{f}") && e.2 == "src/a.ts::go"), "{f}: {:?}", o.edges_by_qn);
    }
    assert!(has(&o, EdgeKind::Calls, "src/s.ts::real", "src/a.ts::go"), "the unshadowed call still binds: {:?}", o.edges_by_qn);
}

#[test]
fn a_resolved_class_without_the_method_binds_nothing_even_if_another_class_has_it() {
    let o = build(&[
        ("src/a.ts", "export class NoM {\n  other() {}\n}\n"),
        ("src/c.ts", "export class NoM {\n  go() {}\n}\n"),
        ("src/d.ts", "import { NoM } from './a';\nexport function f() { NoM.go(); }\n"),
    ]);
    assert!(!o.edges_by_qn.iter().any(|e| e.0 == EdgeKind::Calls && e.1 == "src/d.ts::f" && e.2.ends_with("::go")), "{:?}", o.edges_by_qn);
}

#[test]
fn a_private_class_field_function_is_no_method_and_binds_nothing() {
    let o = build(&[
        ("src/f.ts", "export class Fld {\n  #p = () => 2;\n  q = () => 3;\n}\n"),
        ("src/u.ts", "import { Fld } from './f';\nexport function f() { Fld.p(); Fld.q(); }\n"),
    ]);
    assert!(!o.nodes.iter().any(|n| n.qualified_name == "src/f.ts::p" || n.qualified_name == "src/f.ts::#p"));
    assert!(!o.edges_by_qn.iter().any(|e| e.0 == EdgeKind::Calls && e.2.ends_with("::p")), "{:?}", o.edges_by_qn);
    assert!(has(&o, EdgeKind::Calls, "src/u.ts::f", "src/f.ts::q"));
}

#[test]
fn describe_each_makes_no_node() {
    let o = build(&[
        ("src/p.ts", SRC),
        ("src/p.test.ts", "import { parse } from './p';\ndescribe.each([1, 2])('t %s', (n) => { parse(n); });\ndescribe.skip('s', () => { parse(1); });\n"),
    ]);
    let names: Vec<_> = o.nodes.iter().filter(|n| n.name.starts_with("describe")).map(|n| n.name.as_str()).collect();
    assert_eq!(names, vec!["describe: s"], "{names:?}");
}

#[test]
fn a_type_only_import_is_not_a_component() {
    let o = build(&[
        ("src/T.tsx", "export function Typed() { return null; }\nexport function Real() { return null; }\n"),
        ("src/A.tsx", "import type { Typed } from './T';\nimport { type Typed as T2, Real } from './T';\nexport function A() { return <div><Typed /><T2 /><Real /></div>; }\n"),
    ]);
    assert!(has(&o, EdgeKind::Calls, "src/A.tsx::A", "src/T.tsx::Real"), "{:?}", o.edges_by_qn);
    assert!(!has(&o, EdgeKind::Calls, "src/A.tsx::A", "src/T.tsx::Typed"), "{:?}", o.edges_by_qn);
}

#[test]
fn loop_bindings_and_hoisted_var_hide_an_imported_class() {
    let o = build(&[
        ("src/a.ts", "export class Base {\n  static go() {}\n}\n"),
        (
            "src/s.ts",
            "import { Base } from './a';\nexport function ofBlock(xs: any[]) { for (const Base of xs) { Base.go(); } }\nexport function ofBare(xs: any[]) { for (const Base of xs) Base.go(); }\nexport function inKey(o: any) { for (const Base in o) Base.go(); }\nexport function cFor() { for (let Base = make(); ok(); ) { Base.go(); } }\nexport function hoisted(x: boolean) { if (x) { var Base = make(); } Base.go(); }\nexport function real() { Base.go(); }\n",
        ),
    ]);
    for f in ["ofBlock", "ofBare", "inKey", "cFor", "hoisted"] {
        assert!(!o.edges_by_qn.iter().any(|e| e.0 == EdgeKind::Calls && e.1 == format!("src/s.ts::{f}") && e.2 == "src/a.ts::go"), "{f}: {:?}", o.edges_by_qn);
    }
    assert!(has(&o, EdgeKind::Calls, "src/s.ts::real", "src/a.ts::go"));
}
