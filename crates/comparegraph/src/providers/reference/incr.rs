//! The incremental plan: which files' edges are recomputed and which are copied from the previous run.
//!
//! An edge's identity is (kind, source qn, target qn); the store hash reads nothing else (row ids are per run). So an edge can be
//! copied whenever resolving it again would give the same triple and confidence. Resolution of a file's imports and calls reads, from
//! OTHER files: their paths (import targets), their symbol names and kinds (callee lookup, name-only guess, duplicate counts), and a
//! few per-file analyses (Python returns, TypeScript declared types, Rust structs) that are reached only through a symbol name the
//! caller's text mentions. So a file outside the changed set can change only if its text mentions a name of a changed file that other
//! files can see (all its symbol names when its symbol list changed, else its class, method and type names; see `plan`), or a name
//! segment of the path of an added or removed file (an import that now resolves, or no longer does).
//!
//! The affected set is the changed and added files plus every unchanged file that mentions one of those names. Cases this cannot see
//! are answered with a full resolve and the `incremental_fallback` limitation: a changed or removed `__init__.py` (a Python
//! re-export names things the file does not define), a changed `Cargo.toml` (the crate map), or an affected set over half the files.
use super::ParsedFile;
use crate::domain::Limitation;
use crate::providers::PrevGraph;
use std::collections::HashSet;

/// The code of the limitation that records the `go.mod` files a run resolved Go imports with.
pub const GO_MODULES: &str = "go_modules";

/// `path=sha256` for every `go.mod` the walker collected, sorted, `;`-joined; empty when there is none. Go import resolution reads
/// these files but they are not parsed files, so the run records this string and the next build compares it.
pub fn go_fingerprint(root: &std::path::Path, paths: &[String]) -> String {
    let mut v: Vec<String> = paths
        .iter()
        .map(|p| {
            let h = std::fs::read(root.join(p)).map_or_else(|_| "unreadable".to_string(), |b| crate::providers::reference::hex_sha256(&b));
            format!("{p}={h}")
        })
        .collect();
    v.sort();
    v.join(";")
}

/// The paths whose entries differ between two fingerprints.
fn fingerprint_diff(a: &str, b: &str) -> String {
    let ea: HashSet<&str> = a.split(';').filter(|x| !x.is_empty()).collect();
    let eb: HashSet<&str> = b.split(';').filter(|x| !x.is_empty()).collect();
    let mut d: Vec<&str> = ea.symmetric_difference(&eb).map(|e| e.split('=').next().unwrap_or(e)).collect();
    d.sort();
    d.dedup();
    d.join(", ")
}

pub enum Plan {
    /// no previous run to build on: resolve everything, silently
    Full,
    /// a previous run exists but cannot be used safely
    Fallback(Limitation),
    /// recompute the edges whose source is in the set; copy the rest
    Partial(HashSet<String>),
}

fn idents(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|t| !t.is_empty())
}

fn is_js_like(path: &str) -> bool {
    matches!(path.rsplit_once('.').map(|x| x.1), Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts"))
}

/// A TS/JS re-export: `export {..} from`, `export * from`, `export type {..} from`, also split over lines (`export {` ... `} from`).
fn has_reexport(lines: &[String]) -> bool {
    let mut open = false;
    for l in lines {
        let t = l.trim_start();
        if t.starts_with("export") && t.contains(" from ") {
            return true;
        }
        if t.starts_with("export {") || t.starts_with("export type {") {
            open = !t.contains('}');
        } else if open && t.starts_with('}') && t.contains(" from ") {
            return true;
        } else if open && t.starts_with('}') {
            open = false;
        }
    }
    false
}

fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// Names that are in nearly every file of their language and decide nothing cross-file.
fn ignorable(name: &str) -> bool {
    matches!(name, "__init__" | "constructor")
}

/// `hashes`: path → content hash of the file node this build just made (the same hash the previous run stored).
pub fn plan(prev: Option<&PrevGraph>, parsed: &[ParsedFile], hashes: &std::collections::HashMap<&str, &str>, go_fp: &str) -> Plan {
    let Some(prev) = prev else { return Plan::Full };
    let fallback = |why: String| Plan::Fallback(Limitation::new("incremental_fallback", why));
    if prev.go_fp != go_fp {
        return fallback(format!("{} changed: the Go module table is built from go.mod files", fingerprint_diff(&prev.go_fp, go_fp)));
    }
    let mut changed: Vec<&ParsedFile> = Vec::new();
    let mut added_or_removed_paths: Vec<&str> = Vec::new();
    let mut names: HashSet<String> = HashSet::new();
    let mut touched: HashSet<String> = HashSet::new();
    let mut now: HashSet<&str> = HashSet::new();
    for f in parsed {
        now.insert(f.rel_path.as_str());
        match prev.file_hashes.get(&f.rel_path) {
            Some(h) if Some(h.as_str()) == hashes.get(f.rel_path.as_str()).copied() => {}
            Some(_) => changed.push(f),
            None => {
                changed.push(f);
                added_or_removed_paths.push(&f.rel_path);
            }
        }
    }
    let removed: Vec<&String> = prev.file_hashes.keys().filter(|p| !now.contains(p.as_str())).collect();
    for p in &removed {
        added_or_removed_paths.push(p);
    }
    for p in changed.iter().map(|f| f.rel_path.as_str()).chain(removed.iter().map(|p| p.as_str())) {
        let b = basename(p);
        if b == "__init__.py" {
            return fallback(format!("{p} changed: a Python package file may re-export names it does not define"));
        }
        if b == "Cargo.toml" {
            return fallback(format!("{p} changed: the Rust crate map is built from manifests"));
        }
        touched.insert(p.to_string());
    }
    // A barrel names things it does not define: a changed re-export moves its importers' targets without any symbol of the barrel
    // changing, and a removed `index.*` took its old re-exports with it (old text is not available).
    for f in &changed {
        if is_js_like(&f.rel_path) && has_reexport(&f.lines) {
            return fallback(format!("{} changed and re-exports from another module", f.rel_path));
        }
    }
    for p in &removed {
        if is_js_like(p) && basename(p).starts_with("index.") {
            return fallback(format!("{p} removed: an index file may have been a re-export barrel"));
        }
    }
    // A changed file whose symbol list is the same (name, kind, signature; as a sorted list) as before offers other files the same names with
    // the same meaning; the only cross-file facts a body can still carry are the field and member types of a class, a struct or a
    // type, so only the names of non-function symbols (classes, methods, types) count as changed. When the list differs, every name
    // of the old and the new list counts. Added and removed files count every name they held.
    let sig = |f: &ParsedFile| -> Vec<(String, String, Option<String>)> {
        let mut v: Vec<_> = f.symbols.iter().map(|s| (s.name.clone(), super::kind_of(&s.kind).as_str().to_string(), s.signature.clone())).collect();
        v.sort();
        v
    };
    for f in &changed {
        let new = sig(f);
        let old = prev.symbols_by_file.get(&f.rel_path);
        let same = old.is_some_and(|o| {
            let mut o = o.clone();
            o.sort();
            o == new
        });
        if same {
            names.extend(new.into_iter().filter(|(_, k, _)| k != "function").map(|(n, _, _)| n));
        } else {
            names.extend(new.into_iter().map(|(n, _, _)| n));
            names.extend(old.into_iter().flatten().map(|(n, _, _)| n.clone()));
        }
    }
    for p in &removed {
        names.extend(prev.symbols_by_file.get(*p).into_iter().flatten().map(|(n, _, _)| n.clone()));
    }
    for p in &added_or_removed_paths {
        let stem = basename(p).rsplit_once('.').map_or(basename(p), |(s, _)| s);
        names.insert(stem.to_string());
        for seg in p.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|t| !t.is_empty()) {
            names.insert(seg.to_string());
        }
    }
    names.retain(|n| !ignorable(n));
    let mut affected = touched;
    let changed_paths: HashSet<&str> = changed.iter().map(|f| f.rel_path.as_str()).collect();
    let hits: Vec<bool> = super::resolve::par_map(
        parsed.len(),
        &|i| parsed[i].rel_path.clone(),
        &|i| {
            let f = &parsed[i];
            !changed_paths.contains(f.rel_path.as_str()) && f.lines.iter().any(|l| idents(l).any(|t| names.contains(t)))
        },
    );
    for (f, hit) in parsed.iter().zip(hits) {
        if hit {
            affected.insert(f.rel_path.clone());
        }
    }
    // A relative import can name a file with no identifier in its text (`import '..'`, `from '.'`, `from . import x`): an added or
    // removed file that such an import now resolves to (an index file for a directory, a sibling module for Python) is invisible to
    // the name test above. Every unchanged file with a relative specifier that lands on the file, or on its directory, is affected.
    if !added_or_removed_paths.is_empty() {
        let dir_of = |p: &str| p.rsplit_once('/').map_or(String::new(), |(d, _)| d.to_string());
        let mut stems: HashSet<String> = HashSet::new();
        for p in &added_or_removed_paths {
            let no_ext = p.rsplit_once('.').map_or(*p, |(s, _)| s);
            stems.insert(no_ext.to_string());
            stems.insert(dir_of(p));
        }
        let py_dirs: Vec<String> = added_or_removed_paths.iter().filter(|p| p.ends_with(".py")).map(|p| dir_of(p)).collect();
        for f in parsed {
            if affected.contains(&f.rel_path) {
                continue;
            }
            let hit = f.imports.iter().any(|i| {
                if !i.source.starts_with('.') {
                    return false;
                }
                if f.rel_path.ends_with(".py") {
                    let d = dir_of(&f.rel_path);
                    return py_dirs.iter().any(|a| a.starts_with(&d) || d.starts_with(a.as_str()));
                }
                stems.contains(&super::resolve::relative_stem(&f.rel_path, &i.source))
            });
            if hit {
                affected.insert(f.rel_path.clone());
            }
        }
    }
    if parsed.len() >= 4 && affected.len() * 2 > parsed.len() {
        return fallback(format!("{} of {} files are affected by the change", affected.len(), parsed.len()));
    }
    Plan::Partial(affected)
}
