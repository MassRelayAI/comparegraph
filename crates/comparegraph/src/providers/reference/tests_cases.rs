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
    assert!(!o.nodes.iter().any(|n| n.name.contains("describe")));
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
    assert!(has(&o, EdgeKind::Tests, "src/p.test.ts", "src/p.ts::parse"), "hook call stays file-sourced");
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
