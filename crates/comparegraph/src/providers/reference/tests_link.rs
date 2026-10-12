//! `tests` edges: from test code to the non-test code it exercises.
//! (a) a resolved `calls` edge whose source lives in a test file and whose target lives in a non-test file keeps its confidence;
//! (b) naming convention at 0.5: test file `test_foo.py` / `foo_test.go` / `foo.test.ts` / `FooTest.java` / `tests/foo.rs` → the
//! unique non-test file with stem `foo`/`Foo`; test function `test_bar` → the unique non-test symbol named `bar`.
use super::{is_test_path, resolve, rust_ctx, ParsedFile};
use crate::domain::EdgeKind;
use std::collections::HashMap;

type Edge = (EdgeKind, String, String, f64);

fn stem(path: &str) -> &str {
    let base = path.rsplit('/').next().unwrap_or(path);
    base.rsplit_once('.').map_or(base, |(s, _)| s)
}

/// The name a test file is about, by convention.
fn subject_of_test_file(path: &str) -> Option<String> {
    let mut s = stem(path).to_string();
    // foo.test.ts / foo.spec.ts leave "foo.test" after one strip
    for suf in [".test", ".spec"] {
        if let Some(x) = s.strip_suffix(suf) {
            s = x.to_string();
        }
    }
    let s = if let Some(x) = s.strip_prefix("test_") {
        x.to_string()
    } else if let Some(x) = s.strip_suffix("_test") {
        x.to_string()
    } else if let Some(x) = s.strip_suffix("Test").filter(|x| !x.is_empty()) {
        x.to_string()
    } else if path.starts_with("tests/") || path.contains("/tests/") || stem(path) != s {
        s
    } else {
        return None;
    };
    (!s.is_empty()).then_some(s)
}

pub fn tests_edges(files: &[ParsedFile], edges: &[Edge]) -> Vec<Edge> {
    // Rust `#[test]` functions and inline `#[cfg(test)] mod`s are test code wherever they live
    let spans: HashMap<&str, Vec<(u32, u32)>> = files
        .iter()
        .filter(|f| f.language == "rust" && !is_test_path(&f.rel_path))
        .map(|f| (f.rel_path.as_str(), rust_ctx::analyze(&f.lines.join("\n")).tests))
        .filter(|(_, t)| !t.is_empty())
        .collect();
    let mut sym_line: HashMap<String, u32> = HashMap::new();
    let mut sym_path: HashMap<String, &str> = HashMap::new();
    let mut by_name: HashMap<&str, Vec<String>> = HashMap::new();
    let mut by_stem: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut test_syms: Vec<(&str, String)> = Vec::new(); // (name, qn) in test files
    for f in files {
        let test = is_test_path(&f.rel_path);
        if !test {
            by_stem
                .entry(stem(&f.rel_path))
                .or_default()
                .push(&f.rel_path);
        }
        for (s, qn) in f.symbols.iter().zip(resolve::qualified_names(f)) {
            sym_path.insert(qn.clone(), &f.rel_path);
            sym_line.insert(qn.clone(), s.line_start);
            if test {
                test_syms.push((&s.name, qn));
            } else {
                by_name.entry(&s.name).or_default().push(qn);
            }
        }
    }
    let mut out: Vec<Edge> = Vec::new();
    for (k, a, b, c) in edges {
        if *k != EdgeKind::Calls {
            continue;
        }
        if let (Some(pa), Some(pb)) = (sym_path.get(a), sym_path.get(b)) {
            let in_test = |p: &str, qn: &str| {
                is_test_path(p)
                    || spans.get(p).is_some_and(|v| sym_line.get(qn).is_some_and(|l| v.iter().any(|(x, y)| x <= l && l <= y)))
            };
            if in_test(pa, a) && !in_test(pb, b) {
                out.push((EdgeKind::Tests, a.clone(), b.clone(), *c));
            }
        }
    }
    for f in files.iter().filter(|f| is_test_path(&f.rel_path)) {
        if let Some(subj) = subject_of_test_file(&f.rel_path) {
            if let Some([only]) = by_stem.get(subj.as_str()).map(Vec::as_slice) {
                out.push((
                    EdgeKind::Tests,
                    f.rel_path.clone(),
                    (*only).to_string(),
                    0.5,
                ));
            }
        }
    }
    for (name, qn) in test_syms {
        if let Some(bar) = name.strip_prefix("test_").filter(|b| !b.is_empty()) {
            if let Some([only]) = by_name.get(bar).map(Vec::as_slice) {
                out.push((EdgeKind::Tests, qn, only.clone(), 0.5));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::domain::EdgeKind;
    use crate::providers::reference::ReferenceProvider;
    use crate::providers::{BuildOptions, GraphProvider};
    fn build(files: &[(&str, &str)]) -> Vec<(EdgeKind, String, String, f64)> {
        let d = tempfile::tempdir().unwrap();
        for (p, c) in files {
            let full = d.path().join(p);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, c).unwrap();
        }
        let out = ReferenceProvider
            .build(d.path(), &BuildOptions::default())
            .unwrap();
        out.edges_by_qn
            .into_iter()
            .filter(|e| e.0 == EdgeKind::Tests)
            .collect()
    }
    fn has(e: &[(EdgeKind, String, String, f64)], a: &str, b: &str, c: f64) -> bool {
        e.iter().any(|x| x.1 == a && x.2 == b && x.3 == c)
    }
    #[test]
    fn python_test_calling_parse_links_at_call_confidence() {
        let e = build(&[
            ("src/parse.py", "def parse(x):\n    return x\n"),
            (
                "tests/test_it.py",
                "from src.parse import parse\n\ndef test_it():\n    parse(1)\n",
            ),
        ]);
        assert!(
            has(&e, "tests/test_it.py::test_it", "src/parse.py::parse", 1.0),
            "{e:?}"
        );
    }
    #[test]
    fn test_file_links_to_same_stem_file_by_name() {
        let e = build(&[
            ("src/foo.ts", "export function a() { return 1; }\n"),
            ("src/foo.test.ts", "export function t() { return 2; }\n"),
        ]);
        assert!(has(&e, "src/foo.test.ts", "src/foo.ts", 0.5), "{e:?}");
    }
    #[test]
    fn test_function_links_to_unique_symbol_by_name() {
        let e = build(&[
            ("src/m.py", "def bar():\n    pass\n"),
            ("tests/test_x.py", "def test_bar():\n    pass\n"),
        ]);
        assert!(
            has(&e, "tests/test_x.py::test_bar", "src/m.py::bar", 0.5),
            "{e:?}"
        );
    }
    #[test]
    fn ambiguous_name_makes_no_edge_and_no_edge_into_a_test_file() {
        let e = build(&[
            ("a/foo.py", "def bar():\n    pass\n"),
            ("b/foo.py", "def bar():\n    pass\n"),
            (
                "tests/test_foo.py",
                "def helper():\n    pass\n\ndef test_bar():\n    pass\n",
            ),
            (
                "tests/test_other.py",
                "from tests.test_foo import helper\n\ndef test_z():\n    helper()\n",
            ),
        ]);
        assert!(e.is_empty(), "{e:?}");
    }
    #[test]
    fn rust_tests_dir_file_links_by_stem() {
        let e = build(&[
            ("src/foo.rs", "pub fn a() {}\n"),
            ("tests/foo.rs", "fn t() {}\n"),
        ]);
        assert!(has(&e, "tests/foo.rs", "src/foo.rs", 0.5), "{e:?}");
    }
}
