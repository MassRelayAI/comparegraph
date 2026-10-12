pub mod inherit;
pub mod resolve;
pub mod vendor;
pub mod walker;
use super::{
    BuildOptions, BuildOutput, Capabilities, GraphProvider, ProviderError, ProviderStatus,
};
use crate::domain::{content_hash, EdgeKind, Limitation, Node, NodeKind};
use std::path::Path;

pub struct ParsedFile {
    pub rel_path: String,
    pub language: String,
    pub lines: Vec<String>,
    pub symbols: Vec<vendor::Symbol>,
    pub call_sites: Vec<vendor::CallSite>,
    pub imports: Vec<vendor::ResolvedImport>,
}

#[derive(Debug, Default)]
pub struct ReferenceProvider;

fn kind_of(k: &vendor::SymbolKind) -> NodeKind {
    match k {
        vendor::SymbolKind::Function => NodeKind::Function,
        vendor::SymbolKind::Class => NodeKind::Class,
        vendor::SymbolKind::Method => NodeKind::Method,
        vendor::SymbolKind::Interface | vendor::SymbolKind::Type => NodeKind::Type,
    }
}
fn is_test_path(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    l.contains("/tests/")
        || l.contains("/test/")
        || l.contains("/__tests__/")
        || l.contains(".test.")
        || l.contains(".spec.")
        || l.contains("_test.")
        || l.starts_with("test_")
        || l.contains("/test_")
}

/// The vendored python extractor reports a method both as Method and as Function at the same line; keep the Method.
fn dedupe_symbols(symbols: Vec<vendor::Symbol>) -> Vec<vendor::Symbol> {
    let is_method = |s: &vendor::Symbol| matches!(s.kind, vendor::SymbolKind::Method);
    let methods: std::collections::HashSet<(u32, String)> = symbols
        .iter()
        .filter(|s| is_method(s))
        .map(|s| (s.line_start, s.name.clone()))
        .collect();
    symbols
        .into_iter()
        .filter(|s| is_method(s) || !methods.contains(&(s.line_start, s.name.clone())))
        .collect()
}

/// Clamp a symbol span into a file of `n_lines` lines: start >= 1, end >= start, end <= max(n_lines, 1). The bool says the end was cut.
pub fn clamp_span(line_start: u32, line_end: u32, n_lines: u32) -> (u32, u32, bool) {
    let (ls, le) = (line_start.max(1), line_end.max(line_start).max(1));
    let cap = n_lines.max(1);
    if le > cap {
        (ls.min(cap), cap, true)
    } else {
        (ls, le, false)
    }
}

impl GraphProvider for ReferenceProvider {
    fn name(&self) -> &str {
        "reference"
    }
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
    fn detect(&self) -> ProviderStatus {
        ProviderStatus {
            installed: true,
            version: self.version(),
            detail: "in-process tree-sitter".into(),
        }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            languages: [
                "typescript",
                "tsx",
                "javascript",
                "jsx",
                "mjs",
                "python",
                "go",
                "rust",
                "ruby",
                "hcl",
                "yaml",
                "c",
                "objc",
                "java",
                "php",
                "swift",
                "bash",
                "perl",
                "css",
                "scss",
                "html",
                "toml",
                "dockerfile",
                "sql",
                "cpp",
                "csharp",
                "kotlin",
                "dart",
                "scala",
                "r",
                "graphql",
                "powershell",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            edge_kinds: vec![
                EdgeKind::Contains,
                EdgeKind::Imports,
                EdgeKind::Calls,
                EdgeKind::Extends,
                EdgeKind::Implements,
            ],
            communities: false,
            embeddings: false,
        }
    }
    fn build(&self, repo_root: &Path, opts: &BuildOptions) -> Result<BuildOutput, ProviderError> {
        let walked = walker::walk(repo_root, opts);
        let files = walked.files;
        let mut out = BuildOutput {
            files_seen: files.len(),
            ..Default::default()
        };
        for (code, n, what) in [
            (
                "truncated_files",
                walked.truncated,
                "files dropped past max_files",
            ),
            (
                "skipped_oversize",
                walked.oversize,
                "files over max_file_bytes",
            ),
            (
                "skipped_dir",
                walked.skipped_dirs,
                "directories skipped (node_modules, target, vendor, …)",
            ),
        ] {
            if n > 0 {
                out.limitations
                    .push(Limitation::new(code, format!("{n} {what}")));
            }
        }
        let mut parsed: Vec<ParsedFile> = Vec::new();
        for f in &files {
            let bytes = match std::fs::read(repo_root.join(&f.rel_path)) {
                Ok(b) => b,
                Err(e) => {
                    out.limitations.push(Limitation::new(
                        "skipped_file",
                        format!("{}: {e}", f.rel_path),
                    ));
                    continue;
                }
            };
            let Ok(source) = String::from_utf8(bytes) else {
                out.limitations.push(Limitation::new(
                    "skipped_file",
                    format!("{}: not utf-8", f.rel_path),
                ));
                continue;
            };
            match vendor::parse(&f.rel_path, &source, &f.language) {
                Ok(r) => {
                    parsed.push(ParsedFile {
                        rel_path: f.rel_path.clone(),
                        language: f.language.clone(),
                        lines: source.lines().map(String::from).collect(),
                        symbols: dedupe_symbols(r.symbols),
                        call_sites: r.call_sites,
                        imports: r.imports,
                    });
                    out.files_parsed += 1;
                }
                Err(e) => out.limitations.push(Limitation::new(
                    "skipped_file",
                    format!("{}: {e}", f.rel_path),
                )),
            }
        }
        for pf in &parsed {
            let n_lines = pf.lines.len() as u32;
            let refs: Vec<&str> = pf.lines.iter().map(String::as_str).collect();
            out.nodes.push(Node {
                id: None,
                kind: if is_test_path(&pf.rel_path) {
                    NodeKind::Test
                } else {
                    NodeKind::File
                },
                name: pf
                    .rel_path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&pf.rel_path)
                    .to_string(),
                qualified_name: pf.rel_path.clone(),
                path: pf.rel_path.clone(),
                line_start: 1,
                line_end: n_lines.max(1),
                content_hash: content_hash(&refs),
                language: pf.language.clone(),
                provider_ref: None,
            });
            // vendored Symbol lines are already 1-based (tree-sitter row + 1)
            for (s, qn) in pf.symbols.iter().zip(resolve::qualified_names(pf)) {
                let (ls, le, clamped) = clamp_span(s.line_start, s.line_end, n_lines);
                if clamped {
                    out.limitations.push(Limitation::new(
                        "clamped_line_end",
                        format!("{}:{} {} → {}", pf.rel_path, s.name, s.line_end, le),
                    ));
                }
                let slice: Vec<&str> = pf.lines
                    [(ls as usize - 1).min(pf.lines.len())..(le as usize).min(pf.lines.len())]
                    .iter()
                    .map(String::as_str)
                    .collect();
                out.nodes.push(Node {
                    id: None,
                    kind: kind_of(&s.kind),
                    name: s.name.clone(),
                    qualified_name: qn,
                    path: pf.rel_path.clone(),
                    line_start: ls,
                    line_end: le,
                    content_hash: content_hash(&slice),
                    language: pf.language.clone(),
                    provider_ref: s.signature.clone(),
                });
            }
        }
        out.edges_by_qn = resolve::resolve_edges(&parsed);
        out.edges_by_qn.extend(inherit::inherit_edges(&parsed));
        out.edges_by_qn
            .sort_by(|a, b| (a.0.as_str(), &a.1, &a.2).cmp(&(b.0.as_str(), &b.1, &b.2)));
        out.edges_by_qn.dedup();
        if parsed.iter().any(|p| p.language == "go") {
            out.limitations.push(Limitation::new(
                "no_inheritance_go",
                "go has structural typing: no extends/implements edges are produced",
            ));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::NodeKind;
    fn repo_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (p, c) in files {
            let full = d.path().join(p);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, c).unwrap();
        }
        d
    }
    #[test]
    fn builds_file_and_symbol_nodes_with_hashes_and_contains_edges() {
        let d = repo_with(&[(
            "src/a.ts",
            "export function f() {\n  return g();\n}\nexport function g() { return 1; }\n",
        )]);
        let out = ReferenceProvider
            .build(d.path(), &BuildOptions::default())
            .unwrap();
        assert_eq!(out.files_parsed, 1);
        let f = out
            .nodes
            .iter()
            .find(|n| n.qualified_name == "src/a.ts::f")
            .unwrap();
        assert_eq!(f.kind, NodeKind::Function);
        assert_eq!((f.line_start, f.line_end), (1, 3));
        assert_eq!(f.content_hash.len(), 64);
        assert!(out
            .nodes
            .iter()
            .any(|n| n.kind == NodeKind::File && n.qualified_name == "src/a.ts"));
        assert!(out.edges_by_qn.contains(&(
            EdgeKind::Contains,
            "src/a.ts".into(),
            "src/a.ts::f".into(),
            1.0
        )));
        assert!(out.edges_by_qn.contains(&(
            EdgeKind::Calls,
            "src/a.ts::f".into(),
            "src/a.ts::g".into(),
            1.0
        )));
    }
    #[test]
    fn skips_unparseable_file_and_records_it() {
        let d = repo_with(&[("src/ok.py", "def f():\n    pass\n"), ("src/bad.py", "")]);
        std::fs::write(d.path().join("src/bin.py"), [0xff, 0xfe, 0x00, 0x80]).unwrap();
        let out = ReferenceProvider
            .build(d.path(), &BuildOptions::default())
            .unwrap();
        assert_eq!(
            out.files_parsed, 2,
            "empty file parses to no symbols but is not an error"
        );
        assert!(
            out.limitations
                .iter()
                .any(|l| l.code == "skipped_file" && l.detail.contains("src/bin.py")),
            "{:?}",
            out.limitations
        );
    }
    #[test]
    fn clamps_line_end_to_file_length() {
        // a symbol whose grammar reports an end line past EOF is clamped and the clamp is recorded
        let d = repo_with(&[("src/a.py", "def f():\n    pass")]); // no trailing newline → 2 lines
        let out = ReferenceProvider
            .build(d.path(), &BuildOptions::default())
            .unwrap();
        let f = out
            .nodes
            .iter()
            .find(|n| n.qualified_name == "src/a.py::f")
            .unwrap();
        assert!(
            f.line_end >= f.line_start && f.line_end <= 2,
            "line_end {} past EOF",
            f.line_end
        );
    }
    #[test]
    fn clamp_span_cuts_an_overshoot_and_leaves_the_rest() {
        assert_eq!(clamp_span(1, 9, 2), (1, 2, true));
        assert_eq!(clamp_span(3, 9, 2), (2, 2, true));
        assert_eq!(clamp_span(1, 2, 2), (1, 2, false));
        assert_eq!(clamp_span(0, 0, 0), (1, 1, false));
    }
    #[test]
    fn walker_limitations_are_recorded() {
        let d = repo_with(&[
            ("a.py", "def a():\n    pass\n"),
            ("b.py", "def b():\n    pass\n"),
            ("node_modules/x.js", "1"),
        ]);
        let out = ReferenceProvider
            .build(
                d.path(),
                &BuildOptions {
                    max_file_bytes: 1_000_000,
                    max_files: 1,
                },
            )
            .unwrap();
        assert!(
            out.limitations
                .iter()
                .any(|l| l.code == "truncated_files" && l.detail.starts_with("1 ")),
            "{:?}",
            out.limitations
        );
        assert!(
            out.limitations.iter().any(|l| l.code == "skipped_dir"),
            "{:?}",
            out.limitations
        );
    }
    #[test]
    fn duplicate_qualified_names_are_suffixed() {
        let d = repo_with(&[("src/a.py", "def f():\n    pass\n\ndef f():\n    pass\n")]);
        let out = ReferenceProvider
            .build(d.path(), &BuildOptions::default())
            .unwrap();
        let names: Vec<_> = out
            .nodes
            .iter()
            .filter(|n| n.kind != NodeKind::File)
            .map(|n| n.qualified_name.as_str())
            .collect();
        assert_eq!(names, vec!["src/a.py::f", "src/a.py::f#2"]);
    }
    #[test]
    fn python_method_is_one_node_of_kind_method() {
        let d = repo_with(&[(
            "a.py",
            "class S:\n    def push(self, x):\n        return x\n",
        )]);
        let out = ReferenceProvider
            .build(d.path(), &BuildOptions::default())
            .unwrap();
        let ms: Vec<_> = out.nodes.iter().filter(|n| n.name == "push").collect();
        assert_eq!(
            ms.len(),
            1,
            "{:?}",
            out.nodes
                .iter()
                .map(|n| &n.qualified_name)
                .collect::<Vec<_>>()
        );
        assert_eq!(ms[0].kind, NodeKind::Method);
        assert!(!ms[0].qualified_name.contains('#'));
    }
    #[test]
    fn empty_repo_is_a_build_with_zero_files_not_an_error() {
        let d = tempfile::tempdir().unwrap();
        let out = ReferenceProvider
            .build(d.path(), &BuildOptions::default())
            .unwrap();
        assert_eq!(
            (out.files_seen, out.files_parsed, out.nodes.len()),
            (0, 0, 0)
        ); // the APPLICATION refuses 0 files (Task 5); the provider reports
    }
}
