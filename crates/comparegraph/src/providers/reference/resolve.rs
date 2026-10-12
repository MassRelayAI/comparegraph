use super::ParsedFile;
use crate::domain::EdgeKind;
use std::collections::HashMap;

/// Qualified names: a file is its rel_path; a symbol is `<rel_path>::<name>`; a repeated name in one file gets `#2`, `#3`, …
pub fn qualified_names(file: &ParsedFile) -> Vec<String> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    file.symbols
        .iter()
        .map(|s| {
            let n = seen
                .entry(s.name.as_str())
                .and_modify(|c| *c += 1)
                .or_insert(1);
            if *n == 1 {
                format!("{}::{}", file.rel_path, s.name)
            } else {
                format!("{}::{}#{}", file.rel_path, s.name, n)
            }
        })
        .collect()
}

fn resolve_import_target(from: &str, source: &str, files: &HashMap<String, ()>) -> Option<String> {
    if from.ends_with(".py") {
        return resolve_py_import(from, source, files);
    }
    if !source.starts_with('.') {
        return None;
    } // external module: no edge
    let base = std::path::Path::new(from)
        .parent()
        .unwrap_or_else(|| std::path::Path::new(""));
    let joined = base.join(source);
    let mut parts: Vec<String> = Vec::new();
    for c in joined.components() {
        match c {
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::Normal(p) => parts.push(p.to_string_lossy().to_string()),
            _ => {}
        }
    }
    let stem = parts.join("/");
    [
        stem.clone(),
        format!("{stem}.ts"),
        format!("{stem}.tsx"),
        format!("{stem}.js"),
        format!("{stem}.mjs"),
        format!("{stem}.py"),
        format!("{stem}/index.ts"),
        format!("{stem}/index.js"),
        format!("{stem}/__init__.py"),
    ]
    .into_iter()
    .find(|cand| files.contains_key(cand))
}

/// Python: `.core` / `..a.b` are relative to the importing file's package; `a.b` is a repo-relative dotted path. External modules find no file, so no edge.
fn resolve_py_import(from: &str, source: &str, files: &HashMap<String, ()>) -> Option<String> {
    let dots = source.chars().take_while(|c| *c == '.').count();
    let rest = &source[dots..];
    let mut parts: Vec<&str> = Vec::new();
    if dots > 0 {
        parts.extend(
            from.split('/')
                .collect::<Vec<_>>()
                .split_last()?
                .1
                .iter()
                .copied(),
        );
        for _ in 1..dots {
            parts.pop()?;
        }
    }
    parts.extend(rest.split('.').filter(|p| !p.is_empty()));
    let stem = parts.join("/");
    let cands = if stem.is_empty() {
        vec!["__init__.py".to_string()]
    } else {
        vec![format!("{stem}.py"), format!("{stem}/__init__.py")]
    };
    cands.into_iter().find(|c| files.contains_key(c))
}

/// Edges by qualified name: (kind, source_qn, target_qn, confidence).
pub fn resolve_edges(files: &[ParsedFile]) -> Vec<(EdgeKind, String, String, f64)> {
    let file_set: HashMap<String, ()> = files.iter().map(|f| (f.rel_path.clone(), ())).collect();
    // name → (qn, duplicates_in_that_file)
    let mut by_name: HashMap<&str, Vec<(String, bool)>> = HashMap::new();
    let mut out = Vec::new();
    let qns: Vec<Vec<String>> = files.iter().map(qualified_names).collect();
    for (f, q) in files.iter().zip(&qns) {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for s in &f.symbols {
            *counts.entry(s.name.as_str()).or_insert(0) += 1;
        }
        for (s, qn) in f.symbols.iter().zip(q) {
            out.push((EdgeKind::Contains, f.rel_path.clone(), qn.clone(), 1.0));
            if !qn.contains('#') {
                by_name
                    .entry(s.name.as_str())
                    .or_default()
                    .push((qn.clone(), counts[s.name.as_str()] > 1));
            }
        }
    }
    for (f, q) in files.iter().zip(&qns) {
        for imp in &f.imports {
            if let Some(t) = resolve_import_target(&f.rel_path, &imp.source, &file_set) {
                out.push((EdgeKind::Imports, f.rel_path.clone(), t, 1.0));
            }
        }
        // first occurrence wins (matches the un-suffixed qualified name); a repeated callee name in the file is ambiguous
        let mut local: HashMap<&str, &String> = HashMap::new();
        let mut local_counts: HashMap<&str, usize> = HashMap::new();
        for (s, qn) in f.symbols.iter().zip(q) {
            local.entry(s.name.as_str()).or_insert(qn);
            *local_counts.entry(s.name.as_str()).or_insert(0) += 1;
        }
        for c in &f.call_sites {
            let Some(caller_qn) = local.get(c.caller.as_str()) else {
                continue;
            };
            // 1. same file; 2. unique across the repo; a duplicated target name halves the confidence
            if let Some(t) = local.get(c.callee.as_str()) {
                let conf = if local_counts[c.callee.as_str()] > 1 {
                    0.5
                } else {
                    1.0
                };
                out.push((EdgeKind::Calls, (*caller_qn).clone(), (*t).clone(), conf));
                continue;
            }
            match by_name.get(c.callee.as_str()) {
                Some(v) if v.len() == 1 => {
                    let (t, dup) = &v[0];
                    out.push((
                        EdgeKind::Calls,
                        (*caller_qn).clone(),
                        t.clone(),
                        if *dup { 0.5 } else { 1.0 },
                    ));
                }
                Some(v) if !v.is_empty() => {
                    out.push((EdgeKind::Calls, (*caller_qn).clone(), v[0].0.clone(), 0.5));
                }
                _ => {}
            }
        }
    }
    out.sort_by(|a, b| (a.0.as_str(), &a.1, &a.2).cmp(&(b.0.as_str(), &b.1, &b.2)));
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::reference::vendor::{CallSite, ResolvedImport, Symbol, SymbolKind};
    fn sym(name: &str, ls: u32) -> Symbol {
        Symbol {
            name: name.into(),
            kind: SymbolKind::Function,
            line_start: ls,
            line_end: ls + 1,
            signature: None,
            doc: None,
        }
    }
    fn pf(
        path: &str,
        symbols: Vec<Symbol>,
        calls: Vec<CallSite>,
        imports: Vec<ResolvedImport>,
    ) -> ParsedFile {
        ParsedFile {
            rel_path: path.into(),
            language: "typescript".into(),
            lines: vec![],
            symbols,
            call_sites: calls,
            imports,
        }
    }
    #[test]
    fn contains_import_and_call_edges_with_confidence() {
        let a = pf(
            "src/a.ts",
            vec![sym("f", 1)],
            vec![CallSite {
                caller: "f".into(),
                callee: "g".into(),
                receiver: None,
                line: 2,
                kind: "call".into(),
            }],
            vec![ResolvedImport {
                source: "./b".into(),
                specifiers: vec!["g".into()],
                line: 1,
                is_default: false,
                from_file: "src/a.ts".into(),
            }],
        );
        let b = pf("src/b.ts", vec![sym("g", 1), sym("g", 5)], vec![], vec![]);
        let edges = resolve_edges(&[a, b]);
        assert!(edges.contains(&(
            EdgeKind::Contains,
            "src/a.ts".into(),
            "src/a.ts::f".into(),
            1.0
        )));
        assert!(edges.contains(&(EdgeKind::Imports, "src/a.ts".into(), "src/b.ts".into(), 1.0)));
        // g is duplicated in b.ts → the call resolves to the first with confidence 0.5
        assert!(edges.contains(&(
            EdgeKind::Calls,
            "src/a.ts::f".into(),
            "src/b.ts::g".into(),
            0.5
        )));
        assert!(edges.contains(&(
            EdgeKind::Contains,
            "src/b.ts".into(),
            "src/b.ts::g#2".into(),
            1.0
        )));
    }
    #[test]
    fn same_file_duplicate_callee_resolves_to_first_at_half_confidence() {
        let a = pf(
            "src/a.ts",
            vec![sym("f", 1), sym("f", 5), sym("g", 9)],
            vec![CallSite {
                caller: "g".into(),
                callee: "f".into(),
                receiver: None,
                line: 10,
                kind: "call".into(),
            }],
            vec![],
        );
        let edges = resolve_edges(&[a]);
        assert!(
            edges.contains(&(
                EdgeKind::Calls,
                "src/a.ts::g".into(),
                "src/a.ts::f".into(),
                0.5
            )),
            "{edges:?}"
        );
    }
    #[test]
    fn python_relative_import_resolves_to_sibling_module() {
        let imp = |s: &str| ResolvedImport {
            source: s.into(),
            specifiers: vec![],
            line: 1,
            is_default: false,
            from_file: "pkg/cli.py".into(),
        };
        let mut a = pf(
            "pkg/cli.py",
            vec![],
            vec![],
            vec![imp(".core"), imp("pkg.core"), imp("os.path")],
        );
        a.language = "python".into();
        let b = pf("pkg/core.py", vec![], vec![], vec![]);
        let edges = resolve_edges(&[a, b]);
        assert_eq!(
            edges
                .iter()
                .filter(|e| e.0 == EdgeKind::Imports && e.1 == "pkg/cli.py" && e.2 == "pkg/core.py")
                .count(),
            1
        );
        assert_eq!(edges.iter().filter(|e| e.0 == EdgeKind::Imports).count(), 1);
    }
    #[test]
    fn unresolved_import_and_call_produce_no_edge() {
        let a = pf(
            "src/a.ts",
            vec![sym("f", 1)],
            vec![CallSite {
                caller: "f".into(),
                callee: "nowhere".into(),
                receiver: None,
                line: 2,
                kind: "call".into(),
            }],
            vec![ResolvedImport {
                source: "lodash".into(),
                specifiers: vec![],
                line: 1,
                is_default: true,
                from_file: "src/a.ts".into(),
            }],
        );
        let edges = resolve_edges(&[a]);
        assert_eq!(
            edges.iter().filter(|e| e.0 != EdgeKind::Contains).count(),
            0
        );
    }
}
