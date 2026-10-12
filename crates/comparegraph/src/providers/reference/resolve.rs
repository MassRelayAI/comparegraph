use super::rust_ctx::{self, RustInfo};
use super::vendor::{CallSite, ResolvedImport, SymbolKind};
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

pub(super) fn resolve_import_target(from: &str, source: &str, files: &HashMap<String, ()>) -> Option<String> {
    if from.ends_with(".py") {
        return resolve_py_import(from, source, files);
    }
    if from.ends_with(".java") {
        return resolve_java_import(source, files);
    }
    if from.ends_with(".go") {
        return resolve_go_dir(source, files).into_iter().next();
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

/// Java: `a.b.C` is the file whose path ends in `a/b/C.java` under any source root; the shortest (then smallest) match wins. Wildcards find nothing.
fn resolve_java_import(source: &str, files: &HashMap<String, ()>) -> Option<String> {
    if source.ends_with('*') || source.is_empty() {
        return None;
    }
    let suffix = format!("{}.java", source.replace('.', "/"));
    let tail = format!("/{suffix}");
    files
        .keys()
        .filter(|f| **f == suffix || f.ends_with(&tail))
        .min_by(|a, b| (a.len(), *a).cmp(&(b.len(), *b)))
        .cloned()
}

/// Go: the `.go` files (sorted) of the one repo directory an import path names: the directory whose path ends with the whole import path, else the one whose last segment equals the path's last segment. Several candidate directories → none.
pub(super) fn resolve_go_dir(source: &str, files: &HashMap<String, ()>) -> Vec<String> {
    let last = source.rsplit('/').next().unwrap_or(source);
    if last.is_empty() || source.starts_with('.') {
        return vec![];
    }
    let mut dirs: HashMap<&str, Vec<&String>> = HashMap::new();
    for f in files.keys().filter(|f| f.ends_with(".go")) {
        let dir = f.rsplit_once('/').map_or("", |d| d.0);
        dirs.entry(dir).or_default().push(f);
    }
    let pick = |pred: &dyn Fn(&str) -> bool| -> Option<Vec<String>> {
        let hits: Vec<&&str> = dirs.keys().filter(|d| pred(d)).collect();
        if hits.len() != 1 {
            return None;
        }
        let mut v: Vec<String> = dirs[*hits[0]].iter().map(|s| (*s).clone()).collect();
        v.sort();
        Some(v)
    };
    let tail = format!("/{source}");
    pick(&|d| d == source || d.ends_with(&tail))
        .or_else(|| pick(&|d| d.rsplit('/').next() == Some(last)))
        .unwrap_or_default()
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

/// TypeScript and JavaScript call each other; every other language is its own family.
fn lang_family(l: &str) -> &str {
    match l {
        "typescript" | "tsx" | "javascript" | "jsx" => "js",
        o => o,
    }
}

/// The repo files an import resolves to: the module itself, plus (Python `from pkg import sub`) a submodule file named by a specifier.
fn import_files(from: &str, imp: &ResolvedImport, files: &HashMap<String, ()>) -> Vec<String> {
    if from.ends_with(".go") {
        return resolve_go_dir(&imp.source, files);
    }
    let mut out: Vec<String> = resolve_import_target(from, &imp.source, files).into_iter().collect();
    if from.ends_with(".py") && !imp.is_default {
        for s in &imp.specifiers {
            let sep = if imp.source.ends_with('.') { "" } else { "." };
            if let Some(t) = resolve_import_target(from, &format!("{}{sep}{s}", imp.source), files) {
                if !out.contains(&t) {
                    out.push(t);
                }
            }
        }
    }
    out
}

/// Does receiver `r` name this import as a module? TS/JS: a namespace (`* as u`) or default import; Python: `import utils` or a `from pkg import sub` submodule;
/// Java: the imported class (static call); Go: the package name (last path segment).
fn is_module_alias(from: &str, imp: &ResolvedImport, r: &str) -> bool {
    let tail = |s: &str| s.rsplit([' ', '.', '/']).next().unwrap_or(s).to_string();
    if from.ends_with(".java") {
        return imp.specifiers.iter().any(|s| tail(s) == r);
    }
    if from.ends_with(".py") && !imp.is_default {
        return imp.specifiers.iter().any(|s| s == r) && imp.source != r;
    }
    (imp.is_default || imp.specifiers.iter().any(|s| s.starts_with("* as ")))
        && imp.specifiers.iter().any(|s| tail(s) == r)
}

/// A symbol named `name` in the first of `targets` that defines it → (qn, unique in that file).
fn symbol_in(
    targets: &[String],
    file_syms: &HashMap<&str, HashMap<&str, (String, bool)>>,
    name: &str,
) -> Option<(String, bool)> {
    targets
        .iter()
        .find_map(|t| file_syms.get(t.as_str()).and_then(|m| m.get(name)).cloned())
}

/// `self.m()` / `this.m()`: the method `callee` inside the smallest class (same file) whose line span contains the caller's.
fn class_method(f: &ParsedFile, q: &[String], caller: &str, callee: &str) -> Option<String> {
    let c = f.symbols.iter().find(|s| s.name == caller)?;
    let class = f
        .symbols
        .iter()
        .filter(|s| matches!(s.kind, SymbolKind::Class) && s.line_start <= c.line_start && c.line_end <= s.line_end)
        .min_by_key(|s| s.line_end - s.line_start)?;
    f.symbols
        .iter()
        .zip(q)
        .find(|(s, _)| {
            s.name == callee
                && !matches!(s.kind, SymbolKind::Class)
                && class.line_start <= s.line_start
                && s.line_end <= class.line_end
        })
        .map(|(_, qn)| qn.clone())
}

/// `r.m()` where the caller's own lines declare `r` with a class type (`Square r`, `r: Square`, `r = new Square(`): the method `callee`
/// inside that class, wherever the class lives. A declared type only; no inference.
fn typed_receiver_method(files: &[ParsedFile], qns: &[Vec<String>], f: &ParsedFile, caller: &str, recv: &str, callee: &str) -> Option<String> {
    if !recv.chars().all(|c| c.is_alphanumeric() || c == '_') || recv.chars().next().is_none_or(char::is_uppercase) {
        return None;
    }
    let c = f.symbols.iter().find(|s| s.name == caller)?;
    let lo = (c.line_start as usize).saturating_sub(1);
    let hi = (c.line_end as usize).min(f.lines.len());
    let text = f.lines.get(lo..hi)?.join("\n");
    let toks: Vec<&str> = text
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .filter(|t| !t.is_empty())
        .collect();
    let ty = toks.windows(2).find_map(|w| {
        // `Type recv` (Java, Go-style `recv Type` handled below) and `recv Type` / `recv: Type`
        if w[1] == recv && w[0].chars().next().is_some_and(char::is_uppercase) {
            Some(w[0])
        } else if w[0] == recv && w[1].chars().next().is_some_and(char::is_uppercase) {
            Some(w[1])
        } else {
            None
        }
    })?;
    for (g, q) in files.iter().zip(qns) {
        if g.language != f.language {
            continue;
        }
        let Some(class) = g.symbols.iter().find(|s| s.name == ty && matches!(s.kind, SymbolKind::Class)) else {
            continue;
        };
        return g
            .symbols
            .iter()
            .zip(q)
            .find(|(s, _)| {
                s.name == callee
                    && !matches!(s.kind, SymbolKind::Class)
                    && class.line_start <= s.line_start
                    && s.line_end <= class.line_end
            })
            .map(|(_, qn)| qn.clone());
    }
    None
}

struct RustCall<'a> {
    files: &'a [ParsedFile],
    qns: &'a [Vec<String>],
    rust: &'a [Option<RustInfo>],
    file_set: &'a HashMap<String, ()>,
    file_syms: &'a HashMap<&'a str, HashMap<&'a str, (String, bool)>>,
}

impl RustCall<'_> {
    /// The method `name` defined inside an `impl ty` block: `prefer` file first, else the one crate-wide match.
    fn method(&self, prefer: usize, ty: &str, name: &str) -> Option<String> {
        let in_file = |i: usize| -> Option<String> {
            let info = self.rust[i].as_ref()?;
            let f = &self.files[i];
            f.symbols.iter().zip(&self.qns[i]).find_map(|(s, qn)| {
                (s.name == name
                    && matches!(s.kind, SymbolKind::Method)
                    && info.impls.iter().any(|(t, a, b)| t == ty && *a <= s.line_start && s.line_end <= *b))
                .then(|| qn.clone())
            })
        };
        if let Some(t) = in_file(prefer) {
            return Some(t);
        }
        let root = rust_ctx::crate_src(&self.files[prefer].rel_path);
        let mut hits = (0..self.files.len())
            .filter(|i| *i != prefer && self.rust[*i].is_some())
            .filter(|i| rust_ctx::crate_src(&self.files[*i].rel_path) == root)
            .filter_map(in_file);
        let first = hits.next()?;
        hits.next().is_none().then_some(first)
    }

    fn item(&self, file: &str, name: &str) -> Option<String> {
        self.file_syms.get(file)?.get(name).map(|e| e.0.clone())
    }

    /// A type name seen from file `fi` → (file defining it, bare type name): through `use`, else this file.
    fn type_home(&self, fi: usize, info: &RustInfo, path: &[String]) -> Option<(usize, String)> {
        let from = &self.files[fi].rel_path;
        let mut full = path.to_vec();
        if let Some((_, p)) = info.uses.iter().find(|(a, _)| Some(a) == path.first()) {
            full = p.iter().cloned().chain(path[1..].iter().cloned()).collect();
        }
        if let Some((file, Some(item))) = rust_ctx::resolve_path(from, &full, self.file_set) {
            let idx = self.files.iter().position(|f| f.rel_path == file)?;
            return Some((idx, item));
        }
        let last = path.last()?;
        (path.len() == 1 && self.file_syms.get(from.as_str()).is_some_and(|m| m.contains_key(last.as_str())))
            .then(|| (fi, last.clone()))
    }

    fn resolve(&self, fi: usize, info: &RustInfo, c: &CallSite) -> Option<String> {
        let f = &self.files[fi];
        let caller = f.symbols.iter().find(|s| s.name == c.caller)?;
        let in_caller = |a: u32, b: u32| a <= caller.line_start && caller.line_end <= b;
        let impl_ty = info
            .impls
            .iter()
            .filter(|(_, a, b)| in_caller(*a, *b))
            .min_by_key(|(_, a, b)| b - a)
            .map(|(t, _, _)| t.as_str());
        let Some(recv) = c.receiver.as_deref() else {
            // `Name(...)`: a function or tuple struct brought in by `use`
            let (_, p) = info.uses.iter().find(|(a, _)| *a == c.callee)?;
            let (file, item) = rust_ctx::resolve_path(&f.rel_path, p, self.file_set)?;
            return self.item(&file, item.as_deref()?);
        };
        let ty = match recv {
            "self" | "Self" => impl_ty.map(|t| (fi, t.to_string())),
            v if !v.contains("::") && v.chars().next().is_some_and(char::is_lowercase) => {
                let var = info
                    .fns
                    .iter()
                    .filter(|g| in_caller(g.start, g.end))
                    .min_by_key(|g| g.end - g.start)
                    .and_then(|g| g.vars.get(v));
                match var {
                    Some(t) => self.type_home(fi, info, std::slice::from_ref(t)),
                    None => None,
                }
            }
            _ => None,
        };
        if matches!(recv, "self" | "Self") && ty.is_none() {
            return None;
        }
        let path: Vec<String> = recv.split("::").map(str::to_string).collect();
        let ty = ty.or_else(|| self.type_home(fi, info, &path));
        if let Some((home, name)) = ty {
            return self.method(home, &name, &c.callee).or_else(|| {
                // `Name::new(...)` with no impl method found: the type itself
                (recv != "self").then(|| self.item(&self.files[home].rel_path, &name)).flatten()
            });
        }
        // `util::f(...)`: a module reached through `use` / `mod`
        let mut full = path.clone();
        if let Some((_, p)) = info.uses.iter().find(|(a, _)| Some(a) == path.first()) {
            full = p.iter().cloned().chain(path[1..].iter().cloned()).collect();
        }
        match rust_ctx::resolve_path(&f.rel_path, &full, self.file_set)? {
            (file, None) => self.item(&file, &c.callee),
            _ => None,
        }
    }
}

/// Edges by qualified name: (kind, source_qn, target_qn, confidence).
pub fn resolve_edges(files: &[ParsedFile]) -> Vec<(EdgeKind, String, String, f64)> {
    let file_set: HashMap<String, ()> = files.iter().map(|f| (f.rel_path.clone(), ())).collect();
    // name → (qn, duplicates_in_that_file)
    let mut by_name: HashMap<&str, Vec<(String, bool, &str)>> = HashMap::new();
    let mut out = Vec::new();
    let qns: Vec<Vec<String>> = files.iter().map(qualified_names).collect();
    // file → name → (first qn, name unique in that file)
    let mut file_syms: HashMap<&str, HashMap<&str, (String, bool)>> = HashMap::new();
    for (f, q) in files.iter().zip(&qns) {
        let m = file_syms.entry(f.rel_path.as_str()).or_default();
        for (s, qn) in f.symbols.iter().zip(q) {
            m.entry(s.name.as_str())
                .and_modify(|e| e.1 = false)
                .or_insert((qn.clone(), true));
        }
    }
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
                    .push((qn.clone(), counts[s.name.as_str()] > 1, lang_family(&f.language)));
            }
        }
    }
    let rust: Vec<Option<RustInfo>> = files
        .iter()
        .map(|f| (f.language == "rust").then(|| rust_ctx::analyze(&f.lines.join("\n"))))
        .collect();
    for (fi, (f, q)) in files.iter().zip(&qns).enumerate() {
        if let Some(info) = &rust[fi] {
            for t in rust_ctx::mod_files(&f.rel_path, &info.mods, &file_set) {
                out.push((EdgeKind::Imports, f.rel_path.clone(), t, 1.0));
            }
            for (_, path) in &info.uses {
                if let Some((t, _)) = rust_ctx::resolve_path(&f.rel_path, path, &file_set) {
                    if t != f.rel_path {
                        out.push((EdgeKind::Imports, f.rel_path.clone(), t, 1.0));
                    }
                }
            }
        }
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
        // the caller's imports, resolved once: (file, specifier, is_default, source)
        let imports: Vec<(&ResolvedImport, Vec<String>)> = f
            .imports
            .iter()
            .map(|i| (i, import_files(&f.rel_path, i, &file_set)))
            .collect();
        for c in &f.call_sites {
            let Some(caller_qn) = local.get(c.caller.as_str()) else {
                continue;
            };
            let recv = c.receiver.as_deref();
            if let Some(info) = &rust[fi] {
                let rc = RustCall { files, qns: &qns, rust: &rust, file_set: &file_set, file_syms: &file_syms };
                if let Some(t) = rc.resolve(fi, info, c) {
                    out.push((EdgeKind::Calls, (*caller_qn).clone(), t, 1.0));
                    continue;
                }
                // `HashMap::new()`: a path ending in a type that is not ours is an external call, never a name guess
                if recv.is_some_and(|r| {
                    r.contains("::") || r.chars().next().is_some_and(char::is_uppercase)
                }) && recv
                    .and_then(|r| r.rsplit("::").next())
                    .is_some_and(|l| l.chars().next().is_some_and(char::is_uppercase) && l != "Self")
                {
                    continue;
                }
            }
            // 1. self.m() / this.m(): a method of the caller's enclosing class
            if matches!(recv, Some("self" | "this")) {
                if let Some(t) = class_method(f, q, &c.caller, &c.callee) {
                    out.push((EdgeKind::Calls, (*caller_qn).clone(), t, 1.0));
                    continue;
                }
            }
            // 1b. r.m() where r is declared with a class type in the caller
            if let Some(r) = recv {
                if f.language != "rust" {
                    if let Some(t) = typed_receiver_method(files, &qns, f, &c.caller, r, &c.callee) {
                        out.push((EdgeKind::Calls, (*caller_qn).clone(), t, 1.0));
                        continue;
                    }
                }
            }
            // 2. mod.func(): `mod` is an imported module alias → that module's own symbol
            if let Some(r) = recv {
                let mut hit = None;
                for (imp, targets) in &imports {
                    if targets.is_empty() || !is_module_alias(&f.rel_path, imp, r) {
                        continue;
                    }
                    if let Some(t) = symbol_in(targets, &file_syms, &c.callee) {
                        hit = Some(t);
                        break;
                    }
                }
                if let Some((t, unique)) = hit {
                    out.push((EdgeKind::Calls, (*caller_qn).clone(), t, if unique { 1.0 } else { 0.5 }));
                    continue;
                }
            }
            // 3. an imported name: the file the import resolves to defines it
            if recv.is_none() {
                let mut hit = None;
                for (imp, targets) in &imports {
                    if targets.is_empty() || !imp.specifiers.contains(&c.callee) {
                        continue;
                    }
                    if let Some(t) = symbol_in(targets, &file_syms, &c.callee) {
                        hit = Some(t);
                        break;
                    }
                }
                if let Some((t, unique)) = hit {
                    out.push((EdgeKind::Calls, (*caller_qn).clone(), t, if unique { 1.0 } else { 0.5 }));
                    continue;
                }
            }
            // 4. same file; 5. unique across the repo; a duplicated target name halves the confidence
            if let Some(t) = local.get(c.callee.as_str()) {
                let conf = if local_counts[c.callee.as_str()] > 1 {
                    0.5
                } else {
                    1.0
                };
                out.push((EdgeKind::Calls, (*caller_qn).clone(), (*t).clone(), conf));
                continue;
            }
            // name-only guess: same language family only (a Python call never lands on a JS bundle's method)
            let fam = lang_family(&f.language);
            let cands: Vec<&(String, bool, &str)> = by_name
                .get(c.callee.as_str())
                .map(|v| v.iter().filter(|e| e.2 == fam).collect())
                .unwrap_or_default();
            match cands.as_slice() {
                [(t, dup, _)] => out.push((
                    EdgeKind::Calls,
                    (*caller_qn).clone(),
                    t.clone(),
                    if *dup { 0.5 } else { 1.0 },
                )),
                [first, ..] => out.push((EdgeKind::Calls, (*caller_qn).clone(), first.0.clone(), 0.5)),
                [] => {}
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

    fn call(caller: &str, callee: &str, recv: Option<&str>) -> CallSite {
        CallSite {
            caller: caller.into(),
            callee: callee.into(),
            receiver: recv.map(Into::into),
            line: 2,
            kind: "calls".into(),
        }
    }
    fn imp(from: &str, source: &str, specs: &[&str], default: bool) -> ResolvedImport {
        ResolvedImport {
            source: source.into(),
            specifiers: specs.iter().map(|s| s.to_string()).collect(),
            line: 1,
            is_default: default,
            from_file: from.into(),
        }
    }
    fn has_call(e: &[(EdgeKind, String, String, f64)], a: &str, b: &str, c: f64) -> bool {
        e.contains(&(EdgeKind::Calls, a.into(), b.into(), c))
    }
    fn span(name: &str, kind: SymbolKind, a: u32, b: u32) -> Symbol {
        Symbol { name: name.into(), kind, line_start: a, line_end: b, signature: None, doc: None }
    }

    #[test]
    fn imported_run_beats_the_other_run() {
        let a = pf("src/a.ts", vec![sym("f", 1)], vec![call("f", "run", None)], vec![imp("src/a.ts", "./b", &["run"], false)]);
        let b = pf("src/b.ts", vec![sym("run", 1)], vec![], vec![]);
        let c = pf("src/c.ts", vec![sym("run", 1)], vec![], vec![]);
        let e = resolve_edges(&[a, b, c]);
        assert!(has_call(&e, "src/a.ts::f", "src/b.ts::run", 1.0), "{e:?}");
        assert_eq!(e.iter().filter(|x| x.0 == EdgeKind::Calls).count(), 1);
    }
    #[test]
    fn self_call_resolves_to_the_enclosing_class_method() {
        let mut a = pf(
            "m.py",
            vec![
                span("A", SymbolKind::Class, 1, 6),
                span("helper", SymbolKind::Method, 2, 3),
                span("run", SymbolKind::Method, 4, 6),
                span("B", SymbolKind::Class, 7, 9),
                span("helper", SymbolKind::Method, 8, 9),
            ],
            vec![call("run", "helper", Some("self"))],
            vec![],
        );
        a.language = "python".into();
        let e = resolve_edges(&[a]);
        assert!(has_call(&e, "m.py::run", "m.py::helper", 1.0), "{e:?}");
        assert!(!has_call(&e, "m.py::run", "m.py::helper#2", 1.0));
        assert!(!has_call(&e, "m.py::run", "m.py::helper#2", 0.5));
    }
    #[test]
    fn self_call_in_second_class_picks_its_own_method() {
        let a = pf(
            "m.ts",
            vec![
                span("A", SymbolKind::Class, 1, 3),
                span("helper", SymbolKind::Method, 2, 3),
                span("B", SymbolKind::Class, 4, 8),
                span("helper", SymbolKind::Method, 5, 6),
                span("run", SymbolKind::Method, 7, 8),
            ],
            vec![call("run", "helper", Some("this"))],
            vec![],
        );
        let e = resolve_edges(&[a]);
        assert!(has_call(&e, "m.ts::run", "m.ts::helper#2", 1.0), "{e:?}");
    }
    #[test]
    fn declared_receiver_type_picks_that_class_method_over_a_namesake() {
        let mut a = pf(
            "A.java",
            vec![
                Symbol { kind: SymbolKind::Class, line_end: 3, ..span("Shape", SymbolKind::Class, 1, 3) },
                span("area", SymbolKind::Method, 2, 2),
                Symbol { kind: SymbolKind::Class, ..span("Square", SymbolKind::Class, 4, 7) },
                span("area", SymbolKind::Method, 5, 6),
                span("check", SymbolKind::Method, 8, 10),
            ],
            vec![call("check", "area", Some("s"))],
            vec![],
        );
        a.language = "java".into();
        a.lines = ["", "", "", "", "", "", "", "double check(Square s) {", "  return s.area();", "}"].map(String::from).to_vec();
        let e = resolve_edges(&[a]);
        assert!(has_call(&e, "A.java::check", "A.java::area#2", 1.0), "{e:?}");
        assert!(!has_call(&e, "A.java::check", "A.java::area", 0.5), "{e:?}");
    }
    #[test]
    fn python_import_module_call() {
        let mut a = pf("pkg/a.py", vec![sym("f", 1)], vec![call("f", "parse", Some("utils"))], vec![imp("pkg/a.py", "utils", &["utils"], true)]);
        a.language = "python".into();
        let u = pf("utils.py", vec![sym("parse", 1)], vec![], vec![]);
        let other = pf("other.py", vec![sym("parse", 1)], vec![], vec![]);
        let e = resolve_edges(&[a, u, other]);
        assert!(has_call(&e, "pkg/a.py::f", "utils.py::parse", 1.0), "{e:?}");
    }
    #[test]
    fn python_from_package_import_submodule() {
        let mut a = pf("a.py", vec![sym("f", 1)], vec![call("f", "parse", Some("utils"))], vec![imp("a.py", "pkg", &["pkg", "utils"], false)]);
        a.language = "python".into();
        let init = pf("pkg/__init__.py", vec![], vec![], vec![]);
        let u = pf("pkg/utils.py", vec![sym("parse", 1)], vec![], vec![]);
        let e = resolve_edges(&[a, init, u]);
        assert!(has_call(&e, "a.py::f", "pkg/utils.py::parse", 1.0), "{e:?}");
    }
    #[test]
    fn ts_namespace_import_call() {
        let a = pf("src/a.ts", vec![sym("f", 1)], vec![call("f", "parse", Some("u"))], vec![imp("src/a.ts", "./u", &["* as * as u"], false)]);
        let u = pf("src/u.ts", vec![sym("parse", 1)], vec![], vec![]);
        let o = pf("src/o.ts", vec![sym("parse", 1)], vec![], vec![]);
        let e = resolve_edges(&[a, u, o]);
        assert!(has_call(&e, "src/a.ts::f", "src/u.ts::parse", 1.0), "{e:?}");
    }
    #[test]
    fn java_import_resolves_under_any_source_root() {
        let files = |l: &[&str]| l.iter().map(|s| (s.to_string(), ())).collect::<HashMap<_, _>>();
        let set = files(&["src/main/java/a/b/C.java", "x/Other.java"]);
        assert_eq!(resolve_import_target("src/main/java/z/D.java", "a.b.C", &set).as_deref(), Some("src/main/java/a/b/C.java"));
        assert_eq!(resolve_import_target("z/D.java", "a.b.*", &set), None);
        assert_eq!(resolve_import_target("z/D.java", "java.util.List", &set), None);
        let a = pf("z/D.java", vec![sym("run", 1)], vec![call("run", "go", Some("C"))], vec![imp("z/D.java", "a.b.C", &["C"], false)]);
        let c = pf("src/main/java/a/b/C.java", vec![sym("go", 1)], vec![], vec![]);
        let e = resolve_edges(&[a, c]);
        assert!(has_call(&e, "z/D.java::run", "src/main/java/a/b/C.java::go", 1.0), "{e:?}");
    }
    #[test]
    fn go_import_matches_a_directory_by_last_segment() {
        let a = pf("cmd/main.go", vec![sym("run", 1)], vec![call("run", "Parse", Some("util"))], vec![imp("cmd/main.go", "example.com/m/internal/util", &["util"], true)]);
        let u = pf("internal/util/p.go", vec![sym("Parse", 1)], vec![], vec![]);
        let o = pf("internal/other/p.go", vec![sym("Parse", 1)], vec![], vec![]);
        let e = resolve_edges(&[a, u, o]);
        assert!(has_call(&e, "cmd/main.go::run", "internal/util/p.go::Parse", 1.0), "{e:?}");
        assert!(e.contains(&(EdgeKind::Imports, "cmd/main.go".into(), "internal/util/p.go".into(), 1.0)));
    }
    #[test]
    fn name_only_guess_stays_inside_the_language_family() {
        let a = pf("src/a.ts", vec![sym("f", 1)], vec![call("f", "get", Some("m"))], vec![]);
        let mut b = pf("assets/d3.js", vec![sym("get", 1)], vec![], vec![]);
        b.language = "javascript".into();
        let mut c = pf("lib/x.py", vec![sym("get", 1)], vec![], vec![]);
        c.language = "python".into();
        let e = resolve_edges(&[a, b, c]);
        assert!(has_call(&e, "src/a.ts::f", "assets/d3.js::get", 1.0), "{e:?}");
        assert!(!e.iter().any(|x| x.0 == EdgeKind::Calls && x.2 == "lib/x.py::get"));
    }
    #[test]
    fn rust_use_mod_and_impl_context_resolve_at_one() {
        use crate::providers::reference::vendor::parse;
        let lib = "mod util;\nuse crate::util::Parser;\nuse crate::util::free as fr;\nstruct App;\nimpl App {\n    fn go(&self, p: &Parser) {\n        let q = Parser::new();\n        p.parse();\n        self.helper();\n        Self::assoc();\n        fr();\n        util::free();\n    }\n    fn helper(&self) {}\n    fn assoc() {}\n}\n";
        let util = "pub struct Parser;\nimpl Parser {\n    pub fn new() -> Parser { Parser }\n    pub fn parse(&self) {}\n}\npub fn free() {}\nstruct Other;\nimpl Other { fn parse(&self) {} fn new() {} }\n";
        let mk = |path: &str, src: &str| {
            let r = parse(path, src, "rust").unwrap();
            ParsedFile {
                rel_path: path.into(),
                language: "rust".into(),
                lines: src.lines().map(String::from).collect(),
                symbols: r.symbols,
                call_sites: r.call_sites,
                imports: r.imports,
            }
        };
        let e = resolve_edges(&[mk("c/src/lib.rs", lib), mk("c/src/util.rs", util)]);
        let go = "c/src/lib.rs::go";
        for t in ["c/src/util.rs::new", "c/src/util.rs::parse", "c/src/lib.rs::helper", "c/src/lib.rs::assoc", "c/src/util.rs::free"] {
            assert!(has_call(&e, go, t, 1.0), "{t}: {e:?}");
        }
        assert!(!e.iter().any(|x| x.0 == EdgeKind::Calls && (x.2.ends_with("::new#2") || x.2.ends_with("::parse#2"))));
        assert!(e.contains(&(EdgeKind::Imports, "c/src/lib.rs".into(), "c/src/util.rs".into(), 1.0)));
    }
    #[test]
    fn rust_external_type_path_call_is_not_a_name_guess() {
        let mut a = pf("src/a.rs", vec![sym("f", 1)], vec![call("f", "new", Some("HashMap"))], vec![]);
        a.language = "rust".into();
        let mut b = pf("src/b.rs", vec![sym("new", 1)], vec![], vec![]);
        b.language = "rust".into();
        let e = resolve_edges(&[a, b]);
        assert!(!e.iter().any(|x| x.0 == EdgeKind::Calls), "{e:?}");
    }
}
