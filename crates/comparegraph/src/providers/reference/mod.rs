pub mod builtins;
pub mod decl_types;
pub mod go_ctx;
pub mod incr;
pub mod inherit;
pub mod js_ctx;
pub mod module_ctx;
pub mod py_ctx;
pub mod resolve;
pub mod rust_ctx;
pub mod tests_link;
pub mod vendor;
pub mod walker;
#[cfg(test)]
mod tests_recall;
#[cfg(test)]
mod tests_unicode;
#[cfg(test)]
mod tests_a2;
#[cfg(test)]
mod tests_cases;
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

type CachedParse = (
    Vec<vendor::Symbol>,
    Vec<vendor::CallSite>,
    Vec<vendor::ResolvedImport>,
);

pub(crate) fn hex_sha256(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
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
pub(crate) fn is_test_path(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    l.starts_with("tests/")
        || l.starts_with("test/")
        || l.ends_with("test.java")
        || l.contains("/tests/")
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

/// `COMPAREGRAPH_PROFILE=1`: one stderr line per pass with wall time and a count.
pub fn profile_on() -> bool {
    std::env::var_os("COMPAREGRAPH_PROFILE").is_some_and(|v| v != "0" && !v.is_empty())
}
pub fn profile_line(pass: &str, t: std::time::Instant, count: usize) {
    if profile_on() {
        eprintln!("[profile] {pass:<22} {:>9.3}s  count={count}", t.elapsed().as_secs_f64());
    }
}

thread_local! {
    static CURRENT_FILE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    #[cfg(test)]
    pub(super) static PANIC_AT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}
/// Record the file a resolution pass is working on, so a panic in it can name the file (see `build`).
pub(super) fn mark_file(path: &str) {
    CURRENT_FILE.with(|c| {
        let mut c = c.borrow_mut();
        c.clear();
        c.push_str(path);
    });
    #[cfg(test)]
    if PANIC_AT.with(|p| p.borrow().as_deref() == Some(path)) {
        panic!("injected panic at {path}");
    }
}
fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "non-text panic".into())
}

impl GraphProvider for ReferenceProvider {
    fn name(&self) -> &str {
        "reference"
    }
    fn version(&self) -> String {
        // the vendored parser's hash is part of the cache key: a parser change with the same crate version must not reuse old parses
        format!("{}+{}", env!("CARGO_PKG_VERSION"), env!("COMPAREGRAPH_PARSER_HASH"))
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
        let go_mods = go_ctx::GoModules::new(
            walked
                .go_mods
                .iter()
                .filter_map(|p| {
                    let text = std::fs::read_to_string(repo_root.join(p)).ok()?;
                    Some((p.rsplit_once('/').map_or("", |d| d.0).to_string(), go_ctx::parse_go_mod(&text)?))
                })
                .collect(),
        );
        let go_fp = incr::go_fingerprint(repo_root, &walked.go_mods);
        let mut out = BuildOutput {
            files_seen: files.len(),
            ..Default::default()
        };
        if !go_fp.is_empty() {
            out.limitations.push(Limitation::new(incr::GO_MODULES, go_fp.clone()));
        }
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
        let t_parse = std::time::Instant::now();
        let mut minified = walked.minified;
        let version = self.version();
        let mut fresh: Vec<(String, String, String, Vec<u8>)> = Vec::new();
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
            if source.lines().any(|l| l.len() > walker::MINIFIED_LINE_CHARS) {
                minified += 1;
                continue;
            }
            let hash = hex_sha256(source.as_bytes());
            let cached = opts
                .cache
                .as_ref()
                .and_then(|c| c.get(&hash, &f.language, &version))
                .and_then(|b| serde_json::from_slice::<CachedParse>(&b).ok());
            let parsed_now = match cached {
                Some(c) => {
                    out.cache_hits += 1;
                    Ok(vendor::ParseResult {
                        symbols: c.0,
                        call_sites: c.1,
                        imports: c.2,
                    })
                }
                None => {
                    let r = vendor::parse(&f.rel_path, &source, &f.language);
                    if let (Ok(r), true) = (&r, opts.cache.is_some()) {
                        out.cache_misses += 1;
                        if let Ok(b) = serde_json::to_vec(&(&r.symbols, &r.call_sites, &r.imports))
                        {
                            fresh.push((hash, f.language.clone(), version.clone(), b));
                        }
                    }
                    r
                }
            };
            match parsed_now {
                Ok(r) => {
                    let mut symbols = dedupe_symbols(r.symbols);
                    // methods written as class fields bound to a function, which the extractor does not report
                    if matches!(f.language.as_str(), "typescript" | "tsx" | "javascript" | "jsx" | "mjs") {
                        for (name, line_start, line_end) in js_ctx::class_field_functions(&source, &f.language) {
                            if !symbols.iter().any(|s| s.name == name && s.line_start == line_start) {
                                symbols.push(vendor::Symbol { name, kind: vendor::SymbolKind::Method, line_start, line_end, signature: None, doc: None });
                            }
                        }
                    }
                    // one node per JS/TS test case, after the extractor's symbols (the resolver counts on that order)
                    if is_test_path(&f.rel_path) {
                        symbols.extend(js_ctx::test_cases(&source, &f.language).0.into_iter().map(|c| vendor::Symbol {
                            name: c.name,
                            kind: vendor::SymbolKind::Function,
                            line_start: c.line_start,
                            line_end: c.line_end,
                            signature: None,
                            doc: None,
                        }));
                    }
                    parsed.push(ParsedFile {
                        rel_path: f.rel_path.clone(),
                        language: f.language.clone(),
                        lines: source.lines().map(String::from).collect(),
                        symbols,
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
        if minified > 0 {
            out.limitations.push(Limitation::new(
                "skipped_minified",
                format!("{minified} minified or bundled files (*.min.js, *.min.css, *.bundle.js, or a line over {} chars)", walker::MINIFIED_LINE_CHARS),
            ));
        }
        if let Some(c) = &opts.cache {
            c.put_many(&fresh);
        }
        profile_line("read+parse", t_parse, parsed.len());
        CURRENT_FILE.with(|c| c.borrow_mut().clear());
        let resolved = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let t = std::time::Instant::now();
            for pf in &parsed {
                mark_file(&pf.rel_path);
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
                    provider_ref: (pf.language == "python" && module_ctx::has_main_guard(&pf.lines)).then(|| crate::providers::ENTRY_FILE_REF.to_string()),
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
            profile_line("nodes", t, out.nodes.len());
            let t = std::time::Instant::now();
            let hashes: std::collections::HashMap<&str, &str> = out
                .nodes
                .iter()
                .filter(|n| matches!(n.kind, NodeKind::File | NodeKind::Test))
                .map(|n| (n.path.as_str(), n.content_hash.as_str()))
                .collect();
            let plan = incr::plan(opts.prev.as_deref(), &parsed, &hashes, &go_fp);
            drop(hashes);
            if let incr::Plan::Partial(a) = &plan {
                profile_line("incr.plan", t, a.len());
            }
            let raws = inherit::find_all(&parsed);
            profile_line("inherit_find", t, raws.len());
            let t = std::time::Instant::now();
            match plan {
                incr::Plan::Partial(affected) => {
                    let prev = opts.prev.as_deref().expect("a partial plan has a previous run");
                    out.edges_by_qn = resolve::resolve_edges_in(&parsed, go_mods.clone(), Some(&affected), &raws);
                    // the previous run's import and call edges of every unaffected source file; `contains` edges were just rebuilt, and a
                    // `tests` edge from a file's own module-level call (target a symbol) is a converted call, so it is copied too
                    // (naming-convention `tests` edges are rebuilt below by `tests_edges`, over all files)
                    for (k, s, t2, c) in &prev.edges {
                        let keep = match k {
                            EdgeKind::Imports | EdgeKind::Calls => true,
                            // a file node's qualified name is its path, so `path_of_qn[s] == s` means the source is a file node
                            // (a converted module-level call); a naming-link `tests` edge has a file target, never a `::` one
                            EdgeKind::Tests => t2.contains("::") && prev.path_of_qn.get(s).is_some_and(|p| p == s),
                            _ => false,
                        };
                        let Some(src_path) = prev.path_of_qn.get(s) else { continue };
                        if keep && !affected.contains(src_path) {
                            out.edges_by_qn.push((*k, s.clone(), t2.clone(), *c));
                            out.edges_copied += 1;
                        }
                    }
                    out.touched_paths = Some(affected);
                }
                incr::Plan::Fallback(l) => {
                    out.limitations.push(l);
                    out.edges_by_qn = resolve::resolve_edges_in(&parsed, go_mods.clone(), None, &raws);
                }
                incr::Plan::Full => out.edges_by_qn = resolve::resolve_edges_in(&parsed, go_mods.clone(), None, &raws),
            }
            resolve::dedup_max_confidence(&mut out.edges_by_qn);
            profile_line("resolve_edges", t, out.edges_by_qn.len());
            let t = std::time::Instant::now();
            let inh = inherit::inherit_edges_with(&parsed, &raws);
            profile_line("inherit_edges", t, inh.len());
            out.edges_by_qn.extend(inh);
            let t = std::time::Instant::now();
            let links = tests_link::tests_edges(&parsed, &out.edges_by_qn);
            profile_line("tests_edges", t, links.len());
            out.edges_by_qn.extend(links);
            let t = std::time::Instant::now();
            resolve::dedup_max_confidence(&mut out.edges_by_qn);
            profile_line("dedup", t, out.edges_by_qn.len());
            {
                let paths: std::collections::HashSet<&str> = parsed.iter().map(|p| p.rel_path.as_str()).collect();
                if out.edges_by_qn.iter().any(|e| e.0 == EdgeKind::Calls && paths.contains(e.1.as_str())) {
                    out.limitations.push(Limitation::new(
                        "entry_file_heuristic",
                        "a call made at module level counts as a use only from an entry file (main, __main__, cli, index, or a Python script with an `if __name__ == \"__main__\":` guard); elsewhere it does not rescue its callee in dead-code",
                    ));
                }
            }
            if parsed.iter().any(|p| p.language == "go") {
                out.limitations.push(Limitation::new(
                    "no_inheritance_go",
                    "go has structural typing: no extends/implements edges are produced",
                ));
            }
        }));
        if let Err(p) = resolved {
            let file = CURRENT_FILE.with(|c| c.borrow().clone());
            let at = if file.is_empty() { "an unknown file".to_string() } else { file };
            return Err(ProviderError::Panicked(at, panic_text(p.as_ref())));
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
                    ..BuildOptions::default()
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
