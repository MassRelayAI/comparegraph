use super::super::BuildOptions;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    pub rel_path: String,
    pub language: String,
    pub bytes: u64,
}

/// Language id by extension — the ids `vendor::parser::parse` accepts.
pub fn language_for(path: &Path) -> Option<&'static str> {
    if path.file_name().and_then(|n| n.to_str()) == Some("Dockerfile") {
        return Some("dockerfile");
    }
    let ext = path.extension()?.to_str()?;
    Some(match ext {
        "ts" => "typescript",
        "tsx" => "tsx",
        "js" | "cjs" => "javascript",
        "mjs" => "mjs",
        "jsx" => "jsx",
        "py" => "python",
        "go" => "go",
        "rs" => "rust",
        "rb" => "ruby",
        "tf" | "hcl" => "hcl",
        "yml" | "yaml" => "yaml",
        "c" | "h" => "c",
        "m" => "objc",
        "java" => "java",
        "php" => "php",
        "swift" => "swift",
        "sh" | "bash" => "bash",
        "pl" | "pm" => "perl",
        "css" => "css",
        "scss" => "scss",
        "html" | "htm" => "html",
        "toml" => "toml",
        "sql" => "sql",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "cs" => "csharp",
        "kt" | "kts" => "kotlin",
        "dart" => "dart",
        "scala" => "scala",
        "r" | "R" => "r",
        "graphql" | "gql" => "graphql",
        "ps1" => "powershell",
        _ => return None,
    })
}

const ALWAYS_SKIP: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".git",
    "vendor",
    "__pycache__",
    ".venv",
    "venv",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WalkResult {
    pub files: Vec<SourceFile>,
    pub truncated: usize,
    pub oversize: usize,
    pub skipped_dirs: usize,
    /// files skipped by name (`*.min.js`, `*.min.css`, `*.bundle.js`)
    pub minified: usize,
}

/// A minified or bundled artifact by file name.
pub fn is_minified_name(path: &Path) -> bool {
    let n = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
    n.ends_with(".min.js") || n.ends_with(".min.css") || n.ends_with(".bundle.js")
}

/// A single line this long is generated or minified code, never source a person wrote.
pub const MINIFIED_LINE_CHARS: usize = 5_000;

/// Gitignore-aware walk (the `ignore` crate honours .gitignore and .ignore). Collects everything, sorts by rel_path, THEN truncates to
/// `max_files`, so the kept set is deterministic whatever the directory order. Reports what it dropped.
pub fn walk(root: &Path, opts: &BuildOptions) -> WalkResult {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let skipped_dirs = Arc::new(AtomicUsize::new(0));
    let counter = skipped_dirs.clone();
    let mut res = WalkResult::default();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(true)
        .require_git(false)
        .git_ignore(true)
        .git_exclude(true)
        .filter_entry(move |e| {
            let skip = e.depth() > 0
                && e.file_name()
                    .to_str()
                    .map(|n| ALWAYS_SKIP.contains(&n))
                    .unwrap_or(false);
            if skip && e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                counter.fetch_add(1, Ordering::Relaxed);
            }
            !skip
        })
        .build();
    for entry in walker.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let Some(language) = language_for(entry.path()) else {
            continue;
        };
        if is_minified_name(entry.path()) {
            res.minified += 1;
            continue;
        }
        let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if bytes > opts.max_file_bytes {
            res.oversize += 1;
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/");
        res.files.push(SourceFile {
            rel_path: rel,
            language: language.to_string(),
            bytes,
        });
    }
    res.files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    if res.files.len() > opts.max_files {
        res.truncated = res.files.len() - opts.max_files;
        res.files.truncate(opts.max_files);
    }
    res.skipped_dirs = skipped_dirs.load(Ordering::Relaxed);
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn walk_respects_gitignore_size_cap_and_language_map() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("src")).unwrap();
        std::fs::create_dir_all(d.path().join("node_modules/x")).unwrap();
        std::fs::write(d.path().join(".gitignore"), "ignored.ts\n").unwrap();
        std::fs::write(d.path().join("src/a.ts"), "export function f() {}\n").unwrap();
        std::fs::write(d.path().join("src/b.py"), "def g():\n    pass\n").unwrap();
        std::fs::write(d.path().join("src/ignored.ts"), "export function h() {}\n").unwrap();
        std::fs::write(d.path().join("src/big.ts"), "x".repeat(2000)).unwrap();
        std::fs::write(d.path().join("src/notes.txt"), "no language\n").unwrap();
        std::fs::write(d.path().join("node_modules/x/i.js"), "module.exports = 1\n").unwrap();
        let res = walk(
            d.path(),
            &BuildOptions {
                max_file_bytes: 1000,
                max_files: 100,
                ..BuildOptions::default()
            },
        );
        assert_eq!((res.oversize, res.truncated, res.skipped_dirs), (1, 0, 1));
        let files = res.files;
        let mut rel: Vec<_> = files.iter().map(|f| f.rel_path.as_str()).collect();
        rel.sort();
        assert_eq!(rel, vec!["src/a.ts", "src/b.py"]);
        assert_eq!(
            files
                .iter()
                .find(|f| f.rel_path == "src/b.py")
                .unwrap()
                .language,
            "python"
        );
        assert_eq!(language_for(std::path::Path::new("x.tsx")), Some("tsx"));
        assert_eq!(language_for(std::path::Path::new("x.unknown")), None);
    }
    #[test]
    fn minified_names_are_skipped_and_counted() {
        let d = tempfile::tempdir().unwrap();
        for f in ["a.js", "a.min.js", "b.bundle.js", "c.min.css", "d.css"] {
            std::fs::write(d.path().join(f), "x{}\n").unwrap();
        }
        let res = walk(d.path(), &BuildOptions::default());
        let mut rel: Vec<_> = res.files.iter().map(|f| f.rel_path.as_str()).collect();
        rel.sort();
        assert_eq!(rel, vec!["a.js", "d.css"]);
        assert_eq!(res.minified, 3);
    }
    #[test]
    fn truncation_keeps_the_alphabetically_first_files_and_counts_the_rest() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("b.py"), "def b():\n    pass\n").unwrap();
        std::fs::write(d.path().join("a.py"), "def a():\n    pass\n").unwrap();
        let res = walk(
            d.path(),
            &BuildOptions {
                max_file_bytes: 1000,
                max_files: 1,
                ..BuildOptions::default()
            },
        );
        assert_eq!(
            res.files
                .iter()
                .map(|f| f.rel_path.as_str())
                .collect::<Vec<_>>(),
            vec!["a.py"]
        );
        assert_eq!(res.truncated, 1);
    }
}
