use super::builtins;
use super::decl_types::{self, TsDecls};
use super::go_ctx::{self, GoIndex};
use super::js_ctx;
use super::py_ctx::{self, PyInfo};
use super::rust_ctx::{self, RustInfo};
use super::vendor::{self, CallSite, ResolvedImport, SymbolKind};
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

/// The repo's file paths plus indexes built once per run: path → position, boundary-suffix tables for the Java, Go and Python
/// suffix searches, and class name → files holding such a class. It derefs to the plain path set the other passes take.
pub struct FileSet {
    map: HashMap<String, ()>,
    pos: HashMap<String, usize>,
    /// Java: every '/'-boundary suffix (and the whole path) of a `.java` file → the (shortest, then smallest) file with that suffix.
    java: HashMap<String, String>,
    /// Go: directory → its `.go` files (sorted); every '/'-boundary suffix of a directory → the directories; last segment → the directories.
    go_dirs: HashMap<String, Vec<String>>,
    go_suffix: HashMap<String, Vec<String>>,
    go_last: HashMap<String, Vec<String>>,
    /// Python: every proper '/'-boundary suffix of a `.py` file → (count, first file).
    py_tails: HashMap<String, (usize, String)>,
    /// Class name → indexes (ascending, once each) of the files defining a symbol of kind Class with that name.
    classes: HashMap<String, Vec<usize>>,
    /// Go: the `go.mod` module paths, set by the provider (`with_go_mods`); empty = path heuristics only.
    go_mods: super::go_ctx::GoModules,
}

impl std::ops::Deref for FileSet {
    type Target = HashMap<String, ()>;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

fn boundary_suffixes(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('/').map(move |(i, _)| &path[i + 1..])
}

impl FileSet {
    pub fn with_go_mods(mut self, m: super::go_ctx::GoModules) -> FileSet {
        self.go_mods = m;
        self
    }
    pub fn new(files: &[ParsedFile]) -> FileSet {
        let mut fs = FileSet::from_paths(files.iter().map(|f| f.rel_path.as_str()));
        for (i, f) in files.iter().enumerate() {
            for s in &f.symbols {
                if matches!(s.kind, SymbolKind::Class) {
                    let v = fs.classes.entry(s.name.clone()).or_default();
                    if v.last() != Some(&i) {
                        v.push(i);
                    }
                }
            }
        }
        fs
    }
    pub fn from_paths<'a>(paths: impl Iterator<Item = &'a str>) -> FileSet {
        let mut fs = FileSet {
            map: HashMap::new(),
            pos: HashMap::new(),
            java: HashMap::new(),
            go_dirs: HashMap::new(),
            go_suffix: HashMap::new(),
            go_last: HashMap::new(),
            py_tails: HashMap::new(),
            classes: HashMap::new(),
            go_mods: super::go_ctx::GoModules::default(),
        };
        for (i, p) in paths.enumerate() {
            fs.map.insert(p.to_string(), ());
            fs.pos.entry(p.to_string()).or_insert(i);
        }
        let keys: Vec<String> = fs.map.keys().cloned().collect();
        for f in &keys {
            if f.ends_with(".java") {
                for suf in std::iter::once(f.as_str()).chain(boundary_suffixes(f)) {
                    match fs.java.get(suf) {
                        Some(cur) if (cur.len(), cur) <= (f.len(), f) => {}
                        _ => {
                            fs.java.insert(suf.to_string(), f.clone());
                        }
                    }
                }
            }
            if f.ends_with(".go") {
                let dir = f.rsplit_once('/').map_or("", |d| d.0);
                fs.go_dirs.entry(dir.to_string()).or_default().push(f.clone());
            }
            if f.ends_with(".py") {
                for suf in boundary_suffixes(f) {
                    let e = fs.py_tails.entry(suf.to_string()).or_insert((0, f.clone()));
                    e.0 += 1;
                }
            }
        }
        for v in fs.go_dirs.values_mut() {
            v.sort();
        }
        let dirs: Vec<String> = fs.go_dirs.keys().cloned().collect();
        for d in dirs {
            for suf in std::iter::once(d.as_str()).chain(boundary_suffixes(&d)) {
                fs.go_suffix.entry(suf.to_string()).or_default().push(d.clone());
            }
            fs.go_last.entry(d.rsplit('/').next().unwrap_or("").to_string()).or_default().push(d.clone());
        }
        fs
    }
    /// Index of the file with this path (the first, when a path repeats).
    pub fn pos(&self, path: &str) -> Option<usize> {
        self.pos.get(path).copied()
    }
}

/// The path (no extension) a relative specifier names from `from`: `.`/`..`/`./`/`../x` segments applied to the importer's directory.
pub(super) fn relative_stem(from: &str, source: &str) -> String {
    let base = std::path::Path::new(from).parent().unwrap_or_else(|| std::path::Path::new(""));
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
    parts.join("/")
}

pub(super) fn resolve_import_target(from: &str, source: &str, files: &FileSet) -> Option<String> {
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
    let stem = relative_stem(from, source);
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
fn resolve_java_import(source: &str, files: &FileSet) -> Option<String> {
    if source.ends_with('*') || source.is_empty() {
        return None;
    }
    let suffix = format!("{}.java", source.replace('.', "/"));
    files.java.get(&suffix).cloned()
}

/// Go: the `.go` files (sorted) of the one repo directory an import path names: the directory whose path ends with the whole import path, else the one whose last segment equals the path's last segment. Several candidate directories → none.
pub(super) fn resolve_go_dir(source: &str, files: &FileSet) -> Vec<String> {
    // the repo's own `go.mod` module path decides first: a path under it names a directory, a path outside every module is external
    match files.go_mods.dir_of(source) {
        super::go_ctx::ModuleHit::Dir(d) => {
            return files.go_dirs.get(&d).cloned().unwrap_or_default();
        }
        super::go_ctx::ModuleHit::External => return vec![],
        super::go_ctx::ModuleHit::NoModules => {}
    }
    let last = source.rsplit('/').next().unwrap_or(source);
    if last.is_empty() || source.starts_with('.') {
        return vec![];
    }
    // a unique directory is the answer; two hits (distinct directories) is none
    let pick = |hits: Option<&Vec<String>>| -> Option<Vec<String>> {
        match hits.map(Vec::as_slice) {
            Some([d]) => Some(files.go_dirs[d].clone()),
            _ => None,
        }
    };
    pick(files.go_suffix.get(source)).or_else(|| pick(files.go_last.get(last))).unwrap_or_default()
}

/// Python: `.core` / `..a.b` are relative to the importing file's package; `a.b` is a repo-relative dotted path. External modules find no file, so no edge.
fn resolve_py_import(from: &str, source: &str, files: &FileSet) -> Option<String> {
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
    if let Some(c) = cands.iter().find(|c| files.contains_key(*c)) {
        return Some(c.clone());
    }
    // `import srclight.server` with the package under `src/`: a dotted path of two or more names is the one file that ends with it
    if dots == 0 && parts.len() >= 2 {
        let mut n = 0;
        let mut first: Option<&String> = None;
        for c in &cands {
            if let Some((k, f)) = files.py_tails.get(c.as_str()) {
                n += k;
                first.get_or_insert(f);
            }
        }
        return if n == 1 { first.cloned() } else { None };
    }
    None
}

/// The file of submodule `name` of Python module `source` (`from source import name`).
fn py_submodule(from: &str, source: &str, name: &str, files: &FileSet) -> Option<String> {
    let sep = if source.ends_with('.') { "" } else { "." };
    resolve_py_import(from, &format!("{source}{sep}{name}"), files)
}

/// TypeScript and JavaScript call each other; every other language is its own family.
pub(super) fn lang_family(l: &str) -> &str {
    match l {
        "typescript" | "tsx" | "javascript" | "jsx" | "mjs" => "js",
        o => o,
    }
}

/// The repo files an import resolves to: the module itself, plus (Python `from pkg import sub`) a submodule file named by a specifier.
fn import_files(from: &str, imp: &ResolvedImport, files: &FileSet) -> Vec<String> {
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

/// The class a declaration in `text` gives `recv`: `recv: Type`, `recv?: Type`, `recv = new Type(`, `recv = Type(`, or (Java/C#) `Type recv` followed by `=`, `;`, `,` or `)`.
/// An argument next to a constructor (`new Wrapper(inner)`) is none of these.
pub(super) fn declared_type<'a>(text: &'a str, recv: &str) -> Option<&'a str> {
    let is_id = |c: char| c.is_alphanumeric() || c == '_';
    let upper = |t: &str| t.chars().next().is_some_and(char::is_uppercase);
    let ident_at = |from: usize| -> &'a str {
        let rest = &text[from..];
        let end = rest.find(|c: char| !is_id(c)).unwrap_or(rest.len());
        &rest[..end]
    };
    let mut start = 0;
    while let Some(off) = text[start..].find(recv) {
        let i = start + off;
        let j = i + recv.len();
        start = j;
        if text[..i].chars().next_back().is_some_and(|c| is_id(c) || c == '.') || text[j..].chars().next().is_some_and(is_id) {
            continue;
        }
        let after = text[j..].trim_start_matches([' ', '\t']);
        // `recv: Type` / `recv?: Type`
        if let Some(r) = after.strip_prefix("?:").or_else(|| after.strip_prefix(':')) {
            let r = r.trim_start();
            let t = ident_at(text.len() - r.len());
            if upper(t) {
                return Some(t);
            }
        }
        // `recv = new Type(` / `recv = Type(`
        if let Some(r) = after.strip_prefix('=').filter(|r| !r.starts_with('=')) {
            let r = r.trim_start();
            let r = r.strip_prefix("new ").map_or(r, str::trim_start);
            let t = ident_at(text.len() - r.len());
            if upper(t) && r[t.len()..].trim_start().starts_with('(') {
                return Some(t);
            }
        }
        // `Type recv` then `=`, `;`, `,` or `)`
        if after.starts_with(['=', ';', ',', ')']) && !after.starts_with("==") {
            let before = text[..i].trim_end_matches([' ', '\t']);
            if before.len() < i {
                let t_start = before.rfind(|c: char| !is_id(c)).map_or(0, |k| k + before[k..].chars().next().map_or(1, char::len_utf8));
                let t = &before[t_start..];
                if upper(t) {
                    return Some(t);
                }
            }
        }
    }
    None
}

/// `r.m()` where the caller's own lines declare `r` with a class type (see `declared_type`): the method `callee`
/// inside that class, wherever the class lives. A declared type only; no inference.
fn typed_receiver_method(files: &[ParsedFile], qns: &[Vec<String>], file_set: &FileSet, f: &ParsedFile, caller: &str, recv: &str, callee: &str) -> Option<String> {
    if !recv.chars().all(|c| c.is_alphanumeric() || c == '_') || recv.chars().next().is_none_or(char::is_uppercase) {
        return None;
    }
    let c = f.symbols.iter().find(|s| s.name == caller)?;
    let lo = (c.line_start as usize).saturating_sub(1);
    let hi = (c.line_end as usize).min(f.lines.len());
    let text = f.lines.get(lo..hi)?.join("\n");
    let ty = declared_type(&text, recv)?;
    for &gi in file_set.classes.get(ty).into_iter().flatten() {
        let (g, q) = (&files[gi], &qns[gi]);
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

/// The symbol a call site sits in: the smallest symbol of the caller's name whose lines contain the call (two classes may both have a `render`).
fn caller_at<'a>(f: &'a ParsedFile, c: &CallSite) -> Option<&'a vendor::Symbol> {
    f.symbols.iter().filter(|s| s.name == c.caller && s.line_start <= c.line && c.line <= s.line_end).min_by_key(|s| s.line_end - s.line_start)
}

/// The file holding class `ty` as file `gi` means it: defined there, else brought in by one of its imports. An unresolved name is none.
fn declared_class_home(files: &[ParsedFile], file_set: &FileSet, gi: usize, ty: &str) -> Option<usize> {
    let has = |i: usize| files[i].symbols.iter().any(|s| s.name == ty && matches!(s.kind, SymbolKind::Class));
    if has(gi) {
        return Some(gi);
    }
    let g = &files[gi];
    g.imports
        .iter()
        .filter(|i| i.specifiers.iter().any(|s| s == ty))
        .flat_map(|i| import_files(&g.rel_path, i, file_set))
        .find_map(|t| file_set.pos(&t).filter(|h| has(*h)))
}

/// `this.<recv>.<callee>()` as the syntax tree sees it: the declared type of the field in the smallest class holding the call line.
fn field_type(d: &TsDecls, recv: &str, c: &CallSite) -> Option<String> {
    d.member_calls.iter().find(|m| m.0 == c.line && m.1 == recv && m.2 == c.callee)?;
    let class = d.classes.iter().filter(|k| k.start <= c.line && c.line <= k.end).min_by_key(|k| k.end - k.start)?;
    class.fields.get(recv).cloned()
}

/// `const x = make(); x.m()`: the declared return type of `make` (this file, else the file an import of `make` points to) and the
/// file whose imports must resolve that type name. A local declaration named `make` hides an imported one.
fn local_return_type(files: &[ParsedFile], file_set: &FileSet, ts: &[Option<TsDecls>], fi: usize, recv: &str, caller: &vendor::Symbol) -> Option<(String, usize)> {
    let (f, d) = (&files[fi], ts[fi].as_ref()?);
    let mut locals = d.locals.iter().filter(|l| l.1 == recv && caller.line_start <= l.0 && l.0 <= caller.line_end);
    let func = &locals.next()?.2;
    if locals.any(|l| &l.2 != func) {
        return None;
    }
    if let Some(t) = d.returns.get(func) {
        return Some((t.clone(), fi));
    }
    if d.untyped.iter().any(|u| &u.1 == func && (u.2 || (caller.line_start <= u.0 && u.0 <= caller.line_end))) {
        return None;
    }
    f.imports.iter().filter(|i| i.specifiers.iter().any(|s| s == func)).flat_map(|i| import_files(&f.rel_path, i, file_set)).find_map(|t| {
        let gi = file_set.pos(&t)?;
        Some((ts[gi].as_ref()?.returns.get(func)?.clone(), gi))
    })
}

/// `this.reader.read()` / `x.read()` (JS/TS) where the type comes from a DECLARATION: a class field (`private reader: Reader;`,
/// `reader = new Reader()`, a constructor parameter property) or the declared return type of the function `x = make()` called.
/// Never inferred from a body; a type that does not resolve to a class of the repo types nothing.
fn declared_member_method(files: &[ParsedFile], qns: &[Vec<String>], file_set: &FileSet, ts: &[Option<TsDecls>], fi: usize, c: &CallSite) -> Option<String> {
    let (f, recv) = (&files[fi], c.receiver.as_deref()?);
    let d = ts[fi].as_ref()?;
    let caller = caller_at(f, c)?;
    let (ty, def) = match field_type(d, recv, c) {
        Some(t) => (t, fi),
        None if d.member_calls.iter().any(|m| m.0 == c.line && m.1 == recv && m.2 == c.callee) => return None,
        None => local_return_type(files, file_set, ts, fi, recv, caller)?,
    };
    let gi = declared_class_home(files, file_set, def, &ty)?;
    let g = &files[gi];
    let class = g.symbols.iter().find(|s| s.name == ty && matches!(s.kind, SymbolKind::Class))?;
    g.symbols
        .iter()
        .zip(&qns[gi])
        .find(|(s, _)| s.name == c.callee && !matches!(s.kind, SymbolKind::Class) && class.line_start <= s.line_start && s.line_end <= class.line_end)
        .map(|(_, qn)| qn.clone())
}

/// `Foo.bar()` where `Foo` is a class defined in the repo (same language): the method `callee` inside it, only when unique.
fn static_class_method(files: &[ParsedFile], qns: &[Vec<String>], file_set: &FileSet, f: &ParsedFile, recv: &str, callee: &str) -> Option<String> {
    if !recv.chars().next().is_some_and(char::is_uppercase) || !recv.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let mut hits = vec![];
    for &gi in file_set.classes.get(recv).into_iter().flatten() {
        let (g, q) = (&files[gi], &qns[gi]);
        if lang_family(&g.language) != lang_family(&f.language) {
            continue;
        }
        for class in g.symbols.iter().filter(|s| s.name == recv && matches!(s.kind, SymbolKind::Class)) {
            for (s, qn) in g.symbols.iter().zip(q) {
                if s.name == callee
                    && !matches!(s.kind, SymbolKind::Class)
                    && class.line_start <= s.line_start
                    && s.line_end <= class.line_end
                {
                    hits.push(qn.clone());
                }
            }
        }
    }
    (hits.len() == 1).then(|| hits.remove(0))
}

/// `Foo.bar()` / `foo.bar()` with `foo: Foo`, where `Foo` is the class this file defines or imports (the import decides, so two
/// classes called `Node` in different folders never collide): the method `callee` inside it, else inside its base classes
/// (`extends` edges at 1.0, at most eight levels). `Some(None)`: the class is known and has no such method; `None`: not decided. `bases` maps a class qualified name to its base, computed on first use.
fn imported_class_method(
    files: &[ParsedFile],
    qns: &[Vec<String>],
    file_set: &FileSet,
    fi: usize,
    ty: &str,
    callee: &str,
    bases: &mut Option<HashMap<String, String>>,
) -> Option<Option<String>> {
    if !ty.chars().next().is_some_and(char::is_uppercase) || !ty.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let gi = declared_class_home(files, file_set, fi, ty)?;
    let mut at = (gi, files[gi].symbols.iter().position(|s| s.name == ty && matches!(s.kind, SymbolKind::Class))?);
    for _ in 0..8 {
        let (g, ci) = at;
        let class = &files[g].symbols[ci];
        let hit: Vec<&String> = files[g]
            .symbols
            .iter()
            .zip(&qns[g])
            .filter(|(s, _)| s.name == callee && !matches!(s.kind, SymbolKind::Class) && class.line_start <= s.line_start && s.line_end <= class.line_end)
            .map(|(_, q)| q)
            .collect();
        if let [only] = hit.as_slice() {
            return Some(Some((*only).clone()));
        }
        if !hit.is_empty() {
            return Some(None);
        }
        let map = bases.get_or_insert_with(|| {
            super::inherit::inherit_edges(files).into_iter().filter(|e| e.0 == EdgeKind::Extends && e.3 == 1.0).map(|e| (e.1, e.2)).collect()
        });
        // a chain that breaks (no base resolved: none written, or an alias such as `export { A as B }`) decides nothing
        let Some(base) = map.get(&qns[g][ci]) else {
            // no `extends` written: the class is complete and lacks the method; a base that did not resolve decides nothing
            let written = super::inherit::find(&files[g].language, &files[g].lines.join("\n")).iter().any(|r| r.kind == EdgeKind::Extends && r.sub == class.name);
            return if written { None } else { Some(None) };
        };
        let next = files.iter().enumerate().find_map(|(h, hf)| qns[h].iter().position(|q| q == base).filter(|&j| matches!(hf.symbols[j].kind, SymbolKind::Class)).map(|j| (h, j)))?;
        at = next;
    }
    Some(None)
}

struct PyCall<'a> {
    py: &'a [Option<PyInfo>],
    files: &'a [ParsedFile],
    qns: &'a [Vec<String>],
    file_set: &'a FileSet,
    file_syms: &'a HashMap<&'a str, HashMap<&'a str, (String, bool)>>,
}

impl PyCall<'_> {
    fn module_files(&self, from: &str, module: &str, name: Option<&str>) -> Vec<String> {
        let mut out: Vec<String> = resolve_py_import(from, module, self.file_set).into_iter().collect();
        if let Some(n) = name {
            let sep = if module.ends_with('.') { "" } else { "." };
            if let Some(t) = resolve_py_import(from, &format!("{module}{sep}{n}"), self.file_set) {
                if !out.contains(&t) {
                    out.push(t);
                }
            }
        }
        out
    }

    /// The method `callee` inside class `class` of file `gi`.
    fn method_of(&self, gi: usize, class: &str, callee: &str) -> Option<String> {
        let g = &self.files[gi];
        let c = g.symbols.iter().find(|s| s.name == class && matches!(s.kind, SymbolKind::Class))?;
        g.symbols
            .iter()
            .zip(&self.qns[gi])
            .find(|(s, _)| s.name == callee && !matches!(s.kind, SymbolKind::Class) && c.line_start <= s.line_start && s.line_end <= c.line_end)
            .map(|(_, qn)| qn.clone())
    }

    /// The file holding the class a name means in file `fi`: through an import (alias or not), else this file, else the one Python class of that name.
    fn class_home(&self, fi: usize, info: &PyInfo, ty: &str) -> Option<(usize, String)> {
        let from = &self.files[fi].rel_path;
        let has_class = |gi: usize, n: &str| self.files[gi].symbols.iter().any(|s| s.name == n && matches!(s.kind, SymbolKind::Class));
        for imp in info.imports.iter().filter(|i| i.alias == ty) {
            let real = imp.name.as_deref().unwrap_or(ty);
            for t in self.module_files(from, &imp.module, None) {
                if let Some(gi) = self.file_set.pos(&t) {
                    if has_class(gi, real) {
                        return Some((gi, real.to_string()));
                    }
                }
            }
        }
        if has_class(fi, ty) {
            return Some((fi, ty.to_string()));
        }
        let mut hits = self.file_set.classes.get(ty).into_iter().flatten().copied().filter(|gi| self.files[*gi].language == "python");
        let first = hits.next()?;
        hits.next().is_none().then(|| (first, ty.to_string()))
    }

    /// The file and real name of a function imported into file `fi` under the name `func` (`from m import f as func`).
    fn imported_fn(&self, fi: usize, info: &PyInfo, func: &str) -> Option<(usize, String)> {
        let imp = info.imports.iter().find(|i| i.alias == func && i.name.is_some())?;
        let real = imp.name.as_deref()?;
        let gi = self.module_files(&self.files[fi].rel_path, &imp.module, None).iter().find_map(|t| self.file_set.pos(t))?;
        Some((gi, real.to_string()))
    }

    /// The class a function is annotated to return: defined in this file, or imported (alias or not).
    fn returned_class(&self, fi: usize, info: &PyInfo, func: &str) -> Option<String> {
        if let Some(t) = info.returns.get(func) {
            return Some(t.clone());
        }
        let (gi, real) = self.imported_fn(fi, info, func)?;
        self.py[gi].as_ref()?.returns.get(&real).cloned()
    }

    /// Element `ix` of `n` of what `func` is annotated to return as a tuple (this file, or imported).
    fn returned_tuple_class(&self, fi: usize, info: &PyInfo, func: &str, ix: usize, n: usize) -> Option<String> {
        let pick = |els: &Vec<String>| (els.len() == n).then(|| els.get(ix).cloned()).flatten().filter(|t| !t.is_empty());
        if let Some(els) = info.tuples.get(func) {
            return pick(els);
        }
        let (gi, real) = self.imported_fn(fi, info, func)?;
        self.py[gi].as_ref()?.tuples.get(&real).and_then(pick)
    }

    /// Is `recv` a local (or parameter) of the caller, hiding any import of the same name?
    fn shadowed(&self, info: &PyInfo, f: &ParsedFile, caller: &str, recv: &str) -> bool {
        let Some(c) = f.symbols.iter().find(|s| s.name == caller) else { return false };
        info.scopes.iter().any(|s| s.start <= c.line_start && c.line_end <= s.end && s.vars.contains_key(recv))
    }

    /// The declared or constructed type of `recv` as seen from the caller: its own scopes (innermost first), the enclosing class, the module.
    fn var_type(&self, info: &PyInfo, caller: &vendor::Symbol, recv: &str) -> Option<String> {
        let (a, b) = (caller.line_start, caller.line_end);
        let mut scopes: Vec<_> = info.scopes.iter().filter(|s| s.start <= a && b <= s.end).collect();
        scopes.sort_by_key(|s| s.end - s.start);
        if let Some(t) = scopes.iter().find_map(|s| s.vars.get(recv)) {
            return Some(t.clone()).filter(|t| !t.is_empty());
        }
        let class = info.classes.iter().filter(|c| c.start <= a && b <= c.end).min_by_key(|c| c.end - c.start);
        class.and_then(|c| c.attrs.get(recv)).or_else(|| info.module_vars.get(recv)).cloned()
    }

    /// (target qn, confidence) for a call in a Python file whose callee is known through a declared type or an import alias.
    fn resolve(&self, fi: usize, info: &PyInfo, c: &CallSite) -> Option<(String, f64)> {
        let f = &self.files[fi];
        let Some(recv) = c.receiver.as_deref() else {
            // `from m import f as g; g()`
            let imp = info.imports.iter().find(|i| i.alias == c.callee && i.name.as_deref().is_some_and(|n| n != c.callee))?;
            let real = imp.name.as_deref()?;
            let targets = self.module_files(&f.rel_path, &imp.module, None);
            let (t, unique) = symbol_in(&targets, self.file_syms, real)?;
            return Some((t, if unique { 1.0 } else { 0.5 }));
        };
        if matches!(recv, "self" | "cls" | "super") || !recv.chars().all(|ch| ch.is_alphanumeric() || ch == '_') {
            return None;
        }
        let caller = f.symbols.iter().find(|s| s.name == c.caller);
        if caller.is_none() && c.caller != js_ctx::CALLER_FILE {
            return None;
        }
        // a class named in the file or imported (`Foo(...).m()` chained on a constructor, `K.static()`), else a variable of declared type
        // (a module-level call sees only the module's own variables)
        let ty = if py_ctx::class_like(recv) {
            Some(recv.to_string())
        } else {
            match caller {
                Some(caller) => self.var_type(info, caller, recv),
                None => info.module_vars.get(recv).cloned(),
            }
        };
        // `x = make(...)` where `make` is annotated `-> Foo`: x is a Foo
        let ty = match ty.as_deref().and_then(|t| t.strip_prefix("()")) {
            Some(func) => match func.split_once('#') {
                Some((func, at)) => at
                    .split_once('/')
                    .and_then(|(i, n)| Some((i.parse().ok()?, n.parse().ok()?)))
                    .and_then(|(i, n)| self.returned_tuple_class(fi, info, func, i, n)),
                None => self.returned_class(fi, info, func),
            },
            None => ty,
        };
        if let Some(ty) = ty {
            if let Some((gi, class)) = self.class_home(fi, info, &ty) {
                if let Some(t) = self.method_of(gi, &class, &c.callee) {
                    return Some((t, 1.0));
                }
            }
        }
        // `import a.b as c; c.f()` / `from pkg import sub as c; c.f()`: the module's own symbol
        if self.shadowed(info, f, &c.caller, recv) {
            return None;
        }
        for imp in info.imports.iter().filter(|i| i.alias == recv) {
            let Some(real) = imp.name.as_deref() else {
                let targets = self.module_files(&f.rel_path, &imp.module, None);
                if let Some((t, unique)) = symbol_in(&targets, self.file_syms, &c.callee) {
                    return Some((t, if unique { 1.0 } else { 0.5 }));
                }
                continue;
            };
            // `from m import real`: a submodule file `m.real`, else an object of `m` (a class, or an instance assignment), never a function
            if let Some(sub) = py_submodule(&f.rel_path, &imp.module, real, self.file_set) {
                if let Some((t, unique)) = symbol_in(&[sub], self.file_syms, &c.callee) {
                    return Some((t, if unique { 1.0 } else { 0.5 }));
                }
                continue;
            }
            if let Some(t) = self.object_member(&f.rel_path, &imp.module, real, &c.callee) {
                return Some((t, 1.0));
            }
        }
        None
    }

    /// `from module import real; real.callee()`: `real` is a class of the module (its method), or a module-level instance
    /// (`real = Ring(...)`, `real: Ring`, `real = make()` with `-> Ring`) whose class holds the method.
    fn object_member(&self, from: &str, module: &str, real: &str, callee: &str) -> Option<String> {
        let gi = self.module_files(from, module, None).iter().find_map(|t| self.file_set.pos(t))?;
        if let Some(t) = self.method_of(gi, real, callee) {
            return Some(t);
        }
        let ginfo = self.py[gi].as_ref()?;
        let ty = ginfo.module_vars.get(real).filter(|t| !t.is_empty())?;
        let ty = match ty.strip_prefix("()") {
            Some(func) if !func.contains('#') => self.returned_class(gi, ginfo, func)?,
            Some(_) => return None,
            None => ty.clone(),
        };
        let (hi, class) = self.class_home(gi, ginfo, &ty)?;
        self.method_of(hi, &class, callee)
    }
}

struct RustCall<'a> {
    files: &'a [ParsedFile],
    qns: &'a [Vec<String>],
    rust: &'a [Option<RustInfo>],
    file_set: &'a FileSet,
    cm: &'a rust_ctx::CrateMap,
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
        let root = self.cm.crate_key(&self.files[prefer].rel_path);
        let mut hits = (0..self.files.len())
            .filter(|i| *i != prefer && self.rust[*i].is_some())
            .filter(|i| self.cm.crate_key(&self.files[*i].rel_path) == root)
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
        if let Some((file, Some(item))) = rust_ctx::resolve_path(from, &full, self.file_set, self.cm) {
            let idx = self.file_set.pos(&file)?;
            return Some((idx, item));
        }
        let last = path.last()?;
        // a type of this file: a lowercase single name is a variable that happens to share a function's name, never a type
        (path.len() == 1
            && last.chars().next().is_some_and(char::is_uppercase)
            && self.file_syms.get(from.as_str()).is_some_and(|m| m.contains_key(last.as_str())))
            .then(|| (fi, last.clone()))
    }

    /// `Type::assoc()` with no `use` for `Type`: the one type of that name defined in this crate (same module, or through the crate-name path).
    fn crate_type_home(&self, fi: usize, info: &RustInfo, path: &[String]) -> Option<(usize, String)> {
        let last = path.last()?;
        let first = path.first()?;
        if !last.chars().next().is_some_and(char::is_uppercase) || info.uses.iter().any(|(a, _)| a == first) {
            return None;
        }
        let own_path = path.len() == 1 || matches!(first.as_str(), "crate" | "self" | "super") || self.cm.by_name.contains_key(first.as_str());
        if !own_path {
            return None;
        }
        let key = self.cm.crate_key(&self.files[fi].rel_path);
        let mut homes = (0..self.files.len()).filter(|i| {
            self.rust[*i].is_some()
                && self.cm.crate_key(&self.files[*i].rel_path) == key
                && self.file_syms.get(self.files[*i].rel_path.as_str()).is_some_and(|m| m.contains_key(last.as_str()))
        });
        let home = homes.next()?;
        homes.next().is_none().then(|| (home, last.clone()))
    }

    /// The type a call `make()` / `util::make()` is DECLARED to return, seen from the file defining the function: (file, type name).
    fn returned_home(&self, fi: usize, info: &RustInfo, path: &str) -> Option<(usize, String)> {
        let path: Vec<String> = path.split("::").map(str::to_string).collect();
        let (def, name) = if path.len() == 1 && info.returns.contains_key(&path[0]) {
            (fi, path[0].clone())
        } else {
            let mut full = path.clone();
            if let Some((_, p)) = info.uses.iter().find(|(a, _)| Some(a) == path.first()) {
                full = p.iter().cloned().chain(path[1..].iter().cloned()).collect();
            }
            let (file, item) = rust_ctx::resolve_path(&self.files[fi].rel_path, &full, self.file_set, self.cm)?;
            (self.file_set.pos(&file)?, item?)
        };
        let dinfo = self.rust[def].as_ref()?;
        let ty = dinfo.returns.get(&name)?;
        self.type_home(def, dinfo, std::slice::from_ref(ty))
    }

    /// The type DECLARED for field `field` of struct `strukt` (this file, else the one such struct of the crate): (file, type name).
    fn field_home(&self, fi: usize, strukt: &str, field: &str) -> Option<(usize, String)> {
        let key = self.cm.crate_key(&self.files[fi].rel_path);
        let has = |i: usize| self.rust[i].as_ref().is_some_and(|r| r.structs.get(strukt).is_some_and(|m| m.contains_key(field)));
        let def = if has(fi) {
            fi
        } else {
            let mut hits = (0..self.files.len()).filter(|i| self.rust[*i].is_some() && self.cm.crate_key(&self.files[*i].rel_path) == key && has(*i));
            let first = hits.next()?;
            hits.next().is_none().then_some(first)?
        };
        let dinfo = self.rust[def].as_ref()?;
        let ty = dinfo.structs.get(strukt)?.get(field)?;
        self.type_home(def, dinfo, std::slice::from_ref(ty))
    }

    fn resolve(&self, fi: usize, info: &RustInfo, c: &CallSite) -> Option<String> {
        let f = &self.files[fi];
        let caller = caller_at(f, c)?;
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
            let (file, item) = rust_ctx::resolve_path(&f.rel_path, p, self.file_set, self.cm)?;
            return self.item(&file, item.as_deref()?);
        };
        let ty = match recv {
            "self" | "Self" => impl_ty.map(|t| (fi, t.to_string())),
            // `self.reader.read()`: the receiver the vendored extractor names `reader` is a field of the enclosing impl's struct
            v if !v.contains("::")
                && v.chars().next().is_some_and(char::is_lowercase)
                && impl_ty.is_some()
                && info.self_calls.iter().any(|m| m.0 == c.line && m.1 == v && m.2 == c.callee) =>
            {
                self.field_home(fi, impl_ty?, v)
            }
            v if !v.contains("::") && v.chars().next().is_some_and(char::is_lowercase) => {
                let var = info
                    .fns
                    .iter()
                    .filter(|g| in_caller(g.start, g.end))
                    .min_by_key(|g| g.end - g.start)
                    .and_then(|g| g.vars.get(v));
                match var {
                    Some(t) => match t.strip_prefix("()") {
                        Some(func) => self.returned_home(fi, info, func),
                        None => self.type_home(fi, info, std::slice::from_ref(t)),
                    },
                    None => None,
                }
            }
            _ => None,
        };
        if matches!(recv, "self" | "Self") && ty.is_none() {
            return None;
        }
        let path: Vec<String> = recv.split("::").map(str::to_string).collect();
        let ty = ty.or_else(|| self.type_home(fi, info, &path)).or_else(|| self.crate_type_home(fi, info, &path));
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
        match rust_ctx::resolve_path(&f.rel_path, &full, self.file_set, self.cm)? {
            (file, None) => self.item(&file, &c.callee),
            _ => None,
        }
    }
}

/// Edges by qualified name: (kind, source_qn, target_qn, confidence).
/// `f(0..n)` on up to 8 threads (`COMPAREGRAPH_THREADS` overrides the 8), results in index order. A panic in `f(i)` is re-raised on the calling thread after
/// `mark_file(name(i))`, so the build still names the file (the lowest index when several fail).
pub(super) fn par_map<T: Send>(n: usize, name: &(dyn Fn(usize) -> String + Sync), f: &(dyn Fn(usize) -> T + Sync)) -> Vec<T> {
    par_map_capped(thread_cap(std::env::var("COMPAREGRAPH_THREADS").ok().as_deref()), n, name, f)
}
/// The per-file thread cap: 8, or `COMPAREGRAPH_THREADS` when it is a whole number in 1..=64 (anything else is ignored).
fn thread_cap(env: Option<&str>) -> usize {
    env.and_then(|v| v.trim().parse::<usize>().ok()).filter(|n| (1..=64).contains(n)).unwrap_or(8)
}
fn par_map_capped<T: Send>(cap: usize, n: usize, name: &(dyn Fn(usize) -> String + Sync), f: &(dyn Fn(usize) -> T + Sync)) -> Vec<T> {
    let threads = std::thread::available_parallelism().map_or(1, |p| p.get()).min(cap).min(n.max(1));
    if threads <= 1 {
        return (0..n).map(f).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let parts: Vec<Vec<(usize, std::thread::Result<T>)>> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..threads)
            .map(|_| {
                sc.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if i >= n {
                            break;
                        }
                        mine.push((i, std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(i)))));
                    }
                    mine
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().expect("worker panics are caught per item")).collect()
    });
    let mut slots: Vec<Option<std::thread::Result<T>>> = (0..n).map(|_| None).collect();
    for (i, r) in parts.into_iter().flatten() {
        slots[i] = Some(r);
    }
    slots
        .into_iter()
        .enumerate()
        .map(|(i, r)| match r.expect("every index is taken once") {
            Ok(v) => v,
            Err(p) => {
                super::mark_file(&name(i));
                std::panic::resume_unwind(p)
            }
        })
        .collect()
}

pub fn resolve_edges(files: &[ParsedFile]) -> Vec<(EdgeKind, String, String, f64)> {
    resolve_edges_in(files, super::go_ctx::GoModules::default(), None)
}

/// `resolve_edges` with the repo's `go.mod` module paths (plain data: the per-file workers read it by reference).
pub fn resolve_edges_with(files: &[ParsedFile], go_mods: super::go_ctx::GoModules) -> Vec<(EdgeKind, String, String, f64)> {
    resolve_edges_in(files, go_mods, None)
}

/// `only`: resolve imports and calls only for the files in the set (every index is still built over all files; `contains` edges
/// are always produced). The incremental build copies the other files' edges from the previous run; it passes the same module table
/// a full build does.
pub fn resolve_edges_in(
    files: &[ParsedFile],
    go_mods: super::go_ctx::GoModules,
    only: Option<&std::collections::HashSet<String>>,
) -> Vec<(EdgeKind, String, String, f64)> {
    let file_set = FileSet::new(files).with_go_mods(go_mods);
    // name → (qn, duplicates_in_that_file)
    let mut by_name: HashMap<&str, Vec<(String, bool, &str, bool)>> = HashMap::new();
    let mut out = Vec::new();
    let qns: Vec<Vec<String>> = files.iter().map(qualified_names).collect();
    let names = |i: usize| files[i].rel_path.clone();
    let py: Vec<Option<PyInfo>> = par_map(files.len(), &names, &|i| {
        let f = &files[i];
        (f.language == "python").then(|| py_ctx::analyze(&f.lines.join("\n")))
    });
    // TS/JS `export … from` statements per file (read once; the re-export rounds below borrow their names)
    let reexp: Vec<(usize, Vec<js_ctx::ReExport>)> = files
        .iter()
        .enumerate()
        .filter(|(_, f)| matches!(f.language.as_str(), "typescript" | "javascript" | "tsx" | "jsx"))
        .map(|(i, f)| (i, js_ctx::reexports(&f.lines.join("\n"))))
        .filter(|(_, r)| !r.is_empty())
        .collect();
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
    // Python re-exports: `from .impl import run` inside `pkg/__init__.py` makes `run` a name of the package. At most three rounds
    // (a chain of up to three levels); a name the package defines itself is never replaced, so a cycle adds nothing and ends.
    for _ in 0..3 {
        let mut adds: Vec<(&str, &str, (String, bool))> = vec![];
        for (f, info) in files.iter().zip(&py) {
            let Some(info) = info.as_ref().filter(|_| f.rel_path.rsplit('/').next() == Some("__init__.py")) else { continue };
            for imp in info.imports.iter().filter(|i| i.name.is_some()) {
                let Some(real) = imp.name.as_deref() else { continue };
                let Some(t) = resolve_py_import(&f.rel_path, &imp.module, &file_set).filter(|t| *t != f.rel_path) else { continue };
                if file_syms.get(f.rel_path.as_str()).is_some_and(|m| m.contains_key(imp.alias.as_str())) {
                    continue;
                }
                if let Some(hit) = symbol_in(&[t], &file_syms, real) {
                    adds.push((f.rel_path.as_str(), imp.alias.as_str(), hit));
                }
            }
        }
        if adds.is_empty() {
            break;
        }
        for (file, alias, hit) in adds {
            file_syms.entry(file).or_default().entry(alias).or_insert(hit);
        }
    }
    // TS/JS re-exports: `export { render } from './render'` in an index file makes `render` a name of that index (three rounds; a
    // name the file defines itself is never replaced), so an import of the barrel reaches the real function at 1.0.
    // ES: a name two `export *` statements supply from different targets is excluded from the barrel (`blocked`, for good).
    let mut blocked: std::collections::HashSet<(&str, &str)> = std::collections::HashSet::new();
    for _ in 0..3 {
        let mut adds: Vec<(&str, &str, (String, bool), bool)> = vec![];
        for (i, list) in &reexp {
            let f = &files[*i];
            for re in list {
                let Some(t) = resolve_import_target(&f.rel_path, &re.source, &file_set).filter(|t| *t != f.rel_path) else { continue };
                let pairs: Vec<(&str, &str)> = if re.star {
                    file_syms.get(t.as_str()).map(|m| m.keys().copied().filter(|k| *k != "default").map(|k| (k, k)).collect()).unwrap_or_default()
                } else {
                    re.names.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect()
                };
                for (real, alias) in pairs {
                    if blocked.contains(&(f.rel_path.as_str(), alias)) || file_syms.get(f.rel_path.as_str()).is_some_and(|m| m.contains_key(alias)) {
                        continue;
                    }
                    if let Some(hit) = symbol_in(std::slice::from_ref(&t), &file_syms, real) {
                        adds.push((f.rel_path.as_str(), alias, hit, re.star));
                    }
                }
            }
        }
        if adds.is_empty() {
            break;
        }
        for (file, alias, _, _) in &adds {
            let stars: std::collections::HashSet<&str> = adds.iter().filter(|a| a.3 && a.0 == *file && a.1 == *alias).map(|a| a.2 .0.as_str()).collect();
            if stars.len() > 1 {
                blocked.insert((*file, *alias));
            }
        }
        for (file, alias, hit, _) in adds {
            if !blocked.contains(&(file, alias)) {
                file_syms.entry(file).or_default().entry(alias).or_insert(hit);
            }
        }
    }
    for (f, q) in files.iter().zip(&qns) {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for s in &f.symbols {
            *counts.entry(s.name.as_str()).or_insert(0) += 1;
        }
        for (i, (s, qn)) in f.symbols.iter().zip(q).enumerate() {
            // a describe block contains the describes and cases written inside it (they follow it in symbol order); else the file does
            let parent = (s.name.starts_with("it: ") || s.name.starts_with("describe: "))
                .then(|| (0..i).rev().find(|&j| f.symbols[j].name.starts_with("describe: ") && f.symbols[j].line_start <= s.line_start && s.line_end <= f.symbols[j].line_end))
                .flatten();
            out.push((EdgeKind::Contains, parent.map_or_else(|| f.rel_path.clone(), |j| q[j].clone()), qn.clone(), 1.0));
            if !qn.contains('#') {
                by_name
                    .entry(s.name.as_str())
                    .or_default()
                    .push((qn.clone(), counts[s.name.as_str()] > 1, lang_family(&f.language), matches!(s.kind, SymbolKind::Method)));
            }
        }
    }
    // the only targets a table reference may bind: top-level functions (never a method, never a class member, never a `#n` duplicate)
    let fn_qns: std::collections::HashSet<&str> = files
        .iter()
        .zip(&qns)
        .flat_map(|(f, q)| {
            let classes: Vec<&vendor::Symbol> = f.symbols.iter().filter(|c| matches!(c.kind, SymbolKind::Class)).collect();
            f.symbols
                .iter()
                .zip(q)
                .filter(move |(s, qn)| {
                    matches!(s.kind, SymbolKind::Function)
                        && !qn.contains('#')
                        && !classes.iter().any(|c| c.line_start <= s.line_start && s.line_end <= c.line_end && !(c.line_start == s.line_start && c.line_end == s.line_end))
                })
                .map(|(_, qn)| qn.as_str())
        })
        .collect();
    let ts: Vec<Option<TsDecls>> = par_map(files.len(), &names, &|i| decl_types::analyze(&files[i].lines.join("\n"), &files[i].language));
    let mut cm = rust_ctx::CrateMap::default();
    for f in files.iter().filter(|f| f.language == "toml" && f.rel_path.rsplit('/').next() == Some("Cargo.toml")) {
        cm.add_manifest(f.rel_path.rsplit_once('/').map_or("", |d| d.0), &f.lines);
    }
    let rust: Vec<Option<RustInfo>> = par_map(files.len(), &names, &|i| {
        let f = &files[i];
        (f.language == "rust").then(|| rust_ctx::analyze(&f.lines.join("\n")))
    });
    // per-file case calls and module-level calls: tree-sitter passes with no cross-file input
    let per_file_calls: Vec<(Vec<CallSite>, Vec<CallSite>)> = par_map(files.len(), &names, &|i| {
        let f = &files[i];
        let case_calls = if super::is_test_path(&f.rel_path) { js_ctx::test_cases(&f.lines.join("\n"), &f.language).1 } else { vec![] };
        (case_calls, super::module_ctx::module_calls(&f.lines.join("\n"), &f.language))
    });
    let mut bases: Option<HashMap<String, String>> = None; // class → base class, computed on first use
    let go = GoIndex::build(files, &qns, &|src| resolve_go_dir(src, &file_set).first().map(|f| f.rsplit_once('/').map_or("", |d| d.0).to_string()));
    for (fi, (f, q)) in files.iter().zip(&qns).enumerate() {
        if only.is_some_and(|s| !s.contains(f.rel_path.as_str())) {
            continue;
        }
        super::mark_file(&f.rel_path);
        if let Some(info) = &rust[fi] {
            for t in rust_ctx::mod_files(&f.rel_path, &info.mods, &file_set) {
                out.push((EdgeKind::Imports, f.rel_path.clone(), t, 1.0));
            }
            for (_, path) in &info.uses {
                if let Some((t, _)) = rust_ctx::resolve_path(&f.rel_path, path, &file_set, &cm) {
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
        // the calls in `it(...)` / `test(...)` bodies belong to no symbol: they are the test file's own (item: tests, not calls)
        let (case_calls, module_calls) = &per_file_calls[fi];
        let edges_before = out.len();
        let mut locals: Option<Vec<(String, u32, u32)>> = None; // names bound locally, computed on first use
        // a JSX element `<Foo />` is a rendering call from the smallest function, case or describe holding it (else the file)
        let mut jsx: Vec<(CallSite, Option<usize>)> = js_ctx::jsx_elements(&f.lines.join("\n"), &f.language)
            .into_iter()
            .map(|(name, line)| {
                let owner = f
                    .symbols
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| matches!(s.kind, SymbolKind::Function | SymbolKind::Method) && s.line_start <= line && line <= s.line_end)
                    .min_by_key(|(i, s)| (s.line_end - s.line_start, std::cmp::Reverse(*i)))
                    .map(|(i, _)| i);
                (CallSite { caller: js_ctx::CALLER_FILE.into(), callee: name, receiver: None, line, kind: "render".into() }, owner)
            })
            .collect();
        // a type-only import is no component
        if !jsx.is_empty() {
            let type_only = js_ctx::type_only_imports(&f.lines.join("\n"), &f.language);
            jsx.retain(|(c, _)| !type_only.contains(&c.callee) || local.contains_key(c.callee.as_str()));
        }
        let sited = f.call_sites.iter().chain(case_calls).chain(module_calls).map(|c| (c, None)).chain(jsx.iter().map(|(c, o)| (c, *o)));
        for (c, owner) in sited {
            // Go repeats a method name across receiver types in one file: the caller is the symbol whose lines hold the call
            let owner = owner.or_else(|| {
                (f.language == "go" && c.caller != js_ctx::CALLER_FILE)
                    .then(|| {
                        f.symbols
                            .iter()
                            .enumerate()
                            .filter(|(_, s)| s.name == c.caller && s.line_start <= c.line && c.line <= s.line_end)
                            .min_by_key(|(i, s)| (s.line_end - s.line_start, *i))
                            .map(|(i, _)| i)
                    })
                    .flatten()
            });
            let caller_qn: &String = if let Some(i) = owner {
                &q[i]
            } else if c.caller == js_ctx::CALLER_FILE {
                &f.rel_path
            } else if c.caller.starts_with(js_ctx::CASE_PREFIX) {
                // a case is found by its own name and start line; no match, no edge
                let hit = js_ctx::parse_case_key(&c.caller)
                    .and_then(|(l, n)| f.symbols.iter().position(|s| s.name == n && s.line_start == l));
                match hit {
                    Some(i) => &q[i],
                    None => continue,
                }
            } else if let Some(q) = local.get(c.caller.as_str()) {
                q
            } else {
                continue;
            };
            let recv = c.receiver.as_deref();
            // a table reference names a function: bound only through an import or in this file, never by a name guess
            if c.kind == "refs" {
                let hit: Option<(String, bool)> = if let Some(r) = recv {
                    // `m.f` in a table: through a module alias only
                    let mut hit = None;
                    if let Some(pi) = &py[fi] {
                        let pc = PyCall { py: &py, files, qns: &qns, file_set: &file_set, file_syms: &file_syms };
                        let probe = CallSite { kind: "calls".into(), ..c.clone() };
                        hit = pc.resolve(fi, pi, &probe).map(|(t, conf)| (t, conf == 1.0));
                    }
                    hit.or_else(|| {
                        imports.iter().filter(|(imp, t)| !t.is_empty() && is_module_alias(&f.rel_path, imp, r)).find_map(|(_, t)| symbol_in(t, &file_syms, &c.callee))
                    })
                } else {
                    let imported = imports.iter().filter(|(imp, targets)| !targets.is_empty() && imp.specifiers.contains(&c.callee)).find_map(|(_, targets)| symbol_in(targets, &file_syms, &c.callee));
                    imported.or_else(|| local.get(c.callee.as_str()).map(|t| ((*t).clone(), local_counts[c.callee.as_str()] == 1)))
                };
                if let Some((t, true)) = hit.filter(|(t, _)| fn_qns.contains(t.as_str())) {
                    out.push((EdgeKind::Calls, caller_qn.clone(), t, 1.0));
                }
                continue;
            }
            if let Some(info) = &rust[fi] {
                let rc = RustCall { files, qns: &qns, rust: &rust, file_set: &file_set, cm: &cm, file_syms: &file_syms };
                if let Some(t) = rc.resolve(fi, info, c) {
                    out.push((EdgeKind::Calls, caller_qn.clone(), t, 1.0));
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
            let mut shadowed_callee = false;
            // Python: a receiver of declared or constructed type, a chained constructor, an import alias
            if let Some(pi) = &py[fi] {
                let pc = PyCall { py: &py, files, qns: &qns, file_set: &file_set, file_syms: &file_syms };
                if let Some((t, conf)) = pc.resolve(fi, pi, c) {
                    out.push((EdgeKind::Calls, caller_qn.clone(), t, conf));
                    continue;
                }
                // a local variable or parameter of this name hides a module alias of the same name: `daemon = make(); daemon.run()`
                if c.receiver.as_deref().is_some_and(|r| !matches!(r, "self" | "cls") && pc.shadowed(pi, f, &c.caller, r)) {
                    continue;
                }
                // `def f(normalise=None): normalise(x)`: a parameter or local is not the repo function of that name in ANOTHER file
                // (a same-file function stays bound: a pytest fixture parameter calls the fixture it is named after)
                shadowed_callee = c.receiver.is_none() && pc.shadowed(pi, f, &c.caller, &c.callee);
            }
            // Go: the package is the unit. An unqualified call, `pkg.F()` and `x.M()` on a declared receiver bind through the package
            // table and declared types (1.0); an interface receiver, a conversion, a builtin and an unknown receiver bind nothing
            if let Some(g) = go.as_ref().filter(|_| f.language == "go") {
                match g.resolve(fi, c) {
                    go_ctx::Outcome::Bound(t) => {
                        out.push((EdgeKind::Calls, caller_qn.clone(), t, 1.0));
                        continue;
                    }
                    go_ctx::Outcome::Nothing => continue,
                    go_ctx::Outcome::Fallthrough => {}
                }
            }
            // 1. self.m() / this.m(): a method of the caller's enclosing class
            if matches!(recv, Some("self" | "this")) || (f.language == "python" && recv == Some("cls")) {
                if let Some(t) = class_method(f, q, &c.caller, &c.callee) {
                    out.push((EdgeKind::Calls, caller_qn.clone(), t, 1.0));
                    continue;
                }
            }
            // 1b. r.m() where r is declared with a class type in the caller
            if let Some(r) = recv {
                if f.language != "rust" {
                    if let Some(t) = typed_receiver_method(files, &qns, &file_set, f, &c.caller, r, &c.callee) {
                        out.push((EdgeKind::Calls, caller_qn.clone(), t, 1.0));
                        continue;
                    }
                }
            }
            // 1c. a receiver typed by a declared return type or a declared field (`this.reader.m()`, `const x = make(); x.m()`)
            if recv.is_some() {
                if let Some(t) = declared_member_method(files, &qns, &file_set, &ts, fi, c) {
                    out.push((EdgeKind::Calls, caller_qn.clone(), t, 1.0));
                    continue;
                }
            }
            // 2. mod.func(): `mod` is an imported module alias → that module's own symbol
            if let Some(r) = recv {
                let mut hit = None;
                for (imp, targets) in &imports {
                    if targets.is_empty() || !is_module_alias(&f.rel_path, imp, r) {
                        continue;
                    }
                    // Python `from m import r`: `r` is a module only when `m.r` is a file; otherwise it is an object of `m` (decided above)
                    let sub: Vec<String>;
                    let targets = if f.rel_path.ends_with(".py") && !imp.is_default {
                        sub = py_submodule(&f.rel_path, &imp.source, r, &file_set).into_iter().collect();
                        &sub
                    } else {
                        targets
                    };
                    if let Some(t) = symbol_in(targets, &file_syms, &c.callee) {
                        hit = Some(t);
                        break;
                    }
                }
                if let Some((t, unique)) = hit {
                    out.push((EdgeKind::Calls, caller_qn.clone(), t, if unique { 1.0 } else { 0.5 }));
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
                    out.push((EdgeKind::Calls, caller_qn.clone(), t, if unique { 1.0 } else { 0.5 }));
                    continue;
                }
            }
            // A method call on a receiver of unknown type is never bound by name: `x.get()` and `other.search_symbols()` write
            // no edge. `self`/`this` were decided above, typed receivers and module aliases before this point.
            if let Some(r) = recv {
                if f.language != "rust" {
                    // a local, parameter or catch binding of this name hides the class: nothing binds
                    if r.chars().next().is_some_and(char::is_uppercase) && locals.get_or_insert_with(|| js_ctx::local_bindings(&f.lines.join("\n"), &f.language)).iter().any(|b| b.0 == r && b.1 <= c.line && c.line <= b.2) {
                        continue;
                    }
                    // the class is the one this file defines or imports: its method (or its base's), else nothing
                    if let Some(found) = imported_class_method(files, &qns, &file_set, fi, r, &c.callee, &mut bases) {
                        if let Some(t) = found {
                            out.push((EdgeKind::Calls, caller_qn.clone(), t, 1.0));
                        }
                        continue;
                    }
                }
                // `Foo.bar()`: a class named in the repo (same language), the method inside it
                if let Some(t) = static_class_method(files, &qns, &file_set, f, r, &c.callee) {
                    out.push((EdgeKind::Calls, caller_qn.clone(), t, 1.0));
                }
                continue;
            }
            // 4. same file; 5. unique across the repo; a duplicated target name halves the confidence
            if let Some(t) = local.get(c.callee.as_str()) {
                let conf = if local_counts[c.callee.as_str()] > 1 {
                    0.5
                } else {
                    1.0
                };
                out.push((EdgeKind::Calls, caller_qn.clone(), (*t).clone(), conf));
                continue;
            }
            // a rendering call binds only through an import or this file, never by a name guess
            if c.kind == "render" {
                continue;
            }
            // name-only guess: same language family only (a Python call never lands on a JS bundle's method)
            let fam = lang_family(&f.language);
            // a builtin / standard-library name is the language's own function, not a same-named function elsewhere in the repo
            if shadowed_callee || builtins::is_builtin(fam, &c.callee) {
                continue;
            }
            let js = fam == lang_family("typescript");
            let cands: Vec<&(String, bool, &str, bool)> = by_name
                .get(c.callee.as_str())
                .map(|v| v.iter().filter(|e| e.2 == fam && !e.3 && !(js && !name_guess_target(files, &qns, &file_set, &e.0, c.kind == "instantiates"))).collect())
                .unwrap_or_default();
            // JS/TS: a copy under benchmarks/, examples/, test dirs is not the function the caller meant; one real candidate or none
            let cands: Vec<_> = if js && !in_side_dir(&f.rel_path) { cands.into_iter().filter(|e| !in_side_dir(e.0.split("::").next().unwrap_or(""))).collect() } else { cands };
            // a name-only rule never yields 1.0; JS/TS: the same name in several files is ambiguous, so no edge
            let first = if js { (cands.len() == 1).then(|| cands[0]) } else { cands.first().copied() };
            if let Some(first) = first {
                out.push((EdgeKind::Calls, caller_qn.clone(), first.0.clone(), 0.5));
            }
        }
        let tail: Vec<_> = out.drain(edges_before..).collect();
        out.extend(tail.into_iter().filter_map(|mut e| {
            if e.0 == EdgeKind::Calls && e.1 == f.rel_path && super::is_test_path(&f.rel_path) {
                // a test file calling another test file's helper tests nothing
                if super::is_test_path(e.2.split("::").next().unwrap_or("")) {
                    return None;
                }
                e.0 = EdgeKind::Tests;
            }
            Some(e)
        }));
    }
    dedup_max_confidence(&mut out);
    out
}

/// A directory component that marks sample, benchmark or test code: a name-only guess never lands on a copy there.
fn in_side_dir(path: &str) -> bool {
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    parts.iter().any(|d| matches!(*d, "benchmarks" | "examples" | "example" | "bench" | "__tests__" | "fixtures") || d.starts_with("test"))
}

/// Can a JS/TS bare name (or `new X()`) in another file name this symbol? Functions and classes; never a type alias or an interface,
/// never a variable (the extractor's kind decides: a `const`/`let`/`var` arrow is a function, a data binding has no symbol); `new` binds only a class.
fn name_guess_target(files: &[ParsedFile], qns: &[Vec<String>], file_set: &FileSet, qn: &str, new_expr: bool) -> bool {
    let Some(gi) = qn.split("::").next().and_then(|p| file_set.pos(p)) else { return true };
    let Some(si) = qns[gi].iter().position(|q| q == qn) else { return true };
    let s = &files[gi].symbols[si];
    if new_expr {
        return matches!(s.kind, SymbolKind::Class);
    }
    !matches!(s.kind, SymbolKind::Interface | SymbolKind::Type)
}

/// One row per (kind, source, target), sorted, keeping the highest confidence.
pub fn dedup_max_confidence(out: &mut Vec<(EdgeKind, String, String, f64)>) {
    out.sort_by(|a, b| {
        (a.0.as_str(), &a.1, &a.2)
            .cmp(&(b.0.as_str(), &b.1, &b.2))
            .then(b.3.total_cmp(&a.3))
    });
    out.dedup_by(|b, a| a.0 == b.0 && a.1 == b.1 && a.2 == b.2);
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
    // The indexes behind FileSet must answer exactly what the linear scans they replaced answered.
    fn linear_java(source: &str, keys: &[&str]) -> Option<String> {
        if source.ends_with('*') || source.is_empty() {
            return None;
        }
        let suffix = format!("{}.java", source.replace('.', "/"));
        let tail = format!("/{suffix}");
        keys.iter().filter(|f| **f == suffix || f.ends_with(&tail)).min_by(|a, b| (a.len(), **a).cmp(&(b.len(), **b))).map(|s| s.to_string())
    }
    fn linear_go(source: &str, keys: &[&str]) -> Vec<String> {
        let last = source.rsplit('/').next().unwrap_or(source);
        if last.is_empty() || source.starts_with('.') {
            return vec![];
        }
        let mut dirs: HashMap<&str, Vec<&str>> = HashMap::new();
        for f in keys.iter().filter(|f| f.ends_with(".go")) {
            dirs.entry(f.rsplit_once('/').map_or("", |d| d.0)).or_default().push(f);
        }
        let pick = |pred: &dyn Fn(&str) -> bool| -> Option<Vec<String>> {
            let hits: Vec<&&str> = dirs.keys().filter(|d| pred(d)).collect();
            if hits.len() != 1 {
                return None;
            }
            let mut v: Vec<String> = dirs[*hits[0]].iter().map(|s| s.to_string()).collect();
            v.sort();
            Some(v)
        };
        let tail = format!("/{source}");
        pick(&|d| d == source || d.ends_with(&tail)).or_else(|| pick(&|d| d.rsplit('/').next() == Some(last))).unwrap_or_default()
    }
    #[test]
    fn file_set_suffix_indexes_match_the_linear_scans() {
        let keys = [
            "src/main/java/a/b/C.java", "other/a/b/C.java", "a/b/C.java", "x/Other.java", "src/z/Base.java", "lib/z/Base.java",
            "cmd/app/main.go", "cmd/app/util.go", "pkg/log/log.go", "internal/log/log.go", "pkg/store/s.go", "vendor/pkg/store/s.go", "root.go",
            "pkg/a/b.py", "pkg/a/b/__init__.py", "src/srclight/server.py", "lib/srclight/server.py", "tools/only/mod.py", "pkg/__init__.py",
        ];
        let fs = FileSet::from_paths(keys.iter().copied());
        let map: HashMap<String, ()> = keys.iter().map(|k| (k.to_string(), ())).collect();
        for src in ["a.b.C", "z.Base", "x.Other", "a.b.*", "", "q.R", "C"] {
            assert_eq!(resolve_java_import(src, &fs), linear_java(src, &keys), "java {src}");
        }
        for src in ["app", "cmd/app", "log", "pkg/log", "store", "pkg/store", "nope", "../x", "", "a/"] {
            assert_eq!(resolve_go_dir(src, &fs), linear_go(src, &keys), "go {src}");
        }
        // python: the dotted-suffix branch (two or more names, no relative dots): unique file ending in the path
        assert_eq!(resolve_py_import("x.py", "srclight.server", &fs), None, "two matches");
        assert_eq!(resolve_py_import("x.py", "only.mod", &fs).as_deref(), Some("tools/only/mod.py"));
        for src in ["a.b", "srclight.server", "only.mod", "pkg.a.b", "b.c", "a.b.c"] {
            let tails = [format!("/{}.py", src.replace('.', "/")), format!("/{}/__init__.py", src.replace('.', "/"))];
            let hits: Vec<&&str> = keys.iter().filter(|k| tails.iter().any(|t| k.ends_with(t.as_str()))).collect();
            let exact = [format!("{}.py", src.replace('.', "/")), format!("{}/__init__.py", src.replace('.', "/"))];
            let want = exact.iter().find(|c| keys.contains(&c.as_str())).cloned().or_else(|| (hits.len() == 1).then(|| hits[0].to_string()));
            assert_eq!(resolve_py_import("x.py", src, &fs), want, "py {src}");
        }
        assert_eq!(fs.pos("pkg/log/log.go"), Some(8));
        assert_eq!(map.len(), fs.len());
    }
    #[test]
    fn thread_cap_env_overrides_eight_and_ignores_garbage() {
        assert_eq!(thread_cap(None), 8);
        assert_eq!(thread_cap(Some("1")), 1);
        assert_eq!(thread_cap(Some("64")), 64);
        assert_eq!(thread_cap(Some("65")), 8);
        for bad in ["0", "", "abc", "-3", "1.5"] {
            assert_eq!(thread_cap(Some(bad)), 8, "{bad}");
        }
        let one = par_map_capped(1, 100, &|i| i.to_string(), &|i| i * 2);
        let many = par_map_capped(8, 100, &|i| i.to_string(), &|i| i * 2);
        assert_eq!(one, many);
        assert_eq!(one, (0..100).map(|i| i * 2).collect::<Vec<_>>());
    }
    #[test]
    fn par_map_keeps_index_order_and_names_a_panicking_file() {
        let v = par_map(100, &|i| i.to_string(), &|i| i * 2);
        assert_eq!(v, (0..100).map(|i| i * 2).collect::<Vec<_>>());
        let r = std::panic::catch_unwind(|| par_map(10, &|i| format!("f{i}"), &|i| if i == 7 { panic!("boom") } else { i }));
        assert!(r.is_err());
        assert_eq!(super::super::CURRENT_FILE.with(|c| c.borrow().clone()), "f7");
    }
    #[test]
    fn java_import_resolves_under_any_source_root() {
        let files = |l: &[&str]| FileSet::from_paths(l.iter().copied());
        let set = files(&["src/main/java/a/b/C.java", "x/Other.java"]);
        assert_eq!(resolve_import_target("src/main/java/z/D.java", "a.b.C", &set).as_deref(), Some("src/main/java/a/b/C.java"));
        assert_eq!(resolve_import_target("z/D.java", "a.b.*", &set), None);
        assert_eq!(resolve_import_target("z/D.java", "java.util.List", &set), None);
        let a = pf("z/D.java", vec![sym("run", 1)], vec![call("run", "go", Some("C"))], vec![imp("z/D.java", "a.b.C", &["C"], false)]);
        let c = pf("src/main/java/a/b/C.java", vec![sym("go", 1)], vec![], vec![]);
        let e = resolve_edges(&[a, c]);
        assert!(has_call(&e, "z/D.java::run", "src/main/java/a/b/C.java::go", 1.0), "{e:?}");
    }
    fn lang(mut p: ParsedFile, l: &str) -> ParsedFile {
        p.language = l.into();
        p
    }
    fn calls_of(e: &[(EdgeKind, String, String, f64)]) -> Vec<&(EdgeKind, String, String, f64)> {
        e.iter().filter(|x| x.0 == EdgeKind::Calls).collect()
    }
    fn with_lines(mut p: ParsedFile, lines: &[&str]) -> ParsedFile {
        p.lines = lines.iter().map(|l| l.to_string()).collect();
        p
    }
    fn wrapper_file(l: &str, path: &str) -> ParsedFile {
        lang(pf(path, vec![span("Wrapper", SymbolKind::Class, 1, 4), span("run", SymbolKind::Method, 2, 3)], vec![], vec![]), l)
    }
    #[test]
    fn argument_next_to_a_constructor_is_not_a_declared_type() {
        let a = with_lines(
            lang(pf("a.ts", vec![span("go", SymbolKind::Function, 1, 3)], vec![call("go", "run", Some("inner"))], vec![]), "typescript"),
            &["function go(inner: any){", "  const w = new Wrapper(inner); inner.run();", "}"],
        );
        let e = resolve_edges(&[a, wrapper_file("typescript", "b.ts")]);
        assert!(calls_of(&e).is_empty(), "{e:?}");
    }
    #[test]
    fn real_declaration_shapes_still_type_the_receiver() {
        let cases: [(&str, &str, &str, &str); 5] = [
            ("typescript", "a.ts", "function go(w: Wrapper){", "  w.run();"),
            ("typescript", "a.ts", "function go(){", "  const w = new Wrapper(1); w.run();"),
            ("python", "a.py", "def go(w: Wrapper):", "    w.run()"),
            ("python", "a.py", "def go():", "    w = Wrapper(1); w.run()"),
            ("java", "A.java", "void go() {", "  Wrapper w = make(); w.run();"),
        ];
        for (l, path, l1, l2) in cases {
            let a = with_lines(
                lang(pf(path, vec![span("go", SymbolKind::Function, 1, 3)], vec![call("go", "run", Some("w"))], vec![]), l),
                &[l1, l2, "}"],
            );
            let b = wrapper_file(l, &format!("w/{path}"));
            let e = resolve_edges(&[a, b]);
            assert!(has_call(&e, &format!("{path}::go"), &format!("w/{path}::run"), 1.0), "{l1} {l2}: {e:?}");
        }
    }
    #[test]
    fn repo_wide_bare_name_bind_is_half_confidence_and_never_a_method() {
        let a = lang(pf("a.ts", vec![sym("f", 1)], vec![call("f", "helper", None), call("f", "area", None)], vec![]), "typescript");
        let b = lang(pf("b.ts", vec![sym("helper", 1)], vec![], vec![]), "typescript");
        let c = lang(pf("c.ts", vec![span("Sq", SymbolKind::Class, 1, 4), span("area", SymbolKind::Method, 2, 3)], vec![], vec![]), "typescript");
        let e = resolve_edges(&[a, b, c]);
        assert!(has_call(&e, "a.ts::f", "b.ts::helper", 0.5), "{e:?}");
        assert_eq!(calls_of(&e).len(), 1, "{e:?}");
    }
    #[test]
    fn barrel_reexport_resolves_an_imported_name_at_full_confidence() {
        let test = lang(pf("src/dom/index.test.tsx", vec![sym("t", 1)], vec![call("t", "render", None)], vec![imp("src/dom/index.test.tsx", ".", &["render"], false)]), "typescript");
        let idx = with_lines(lang(pf("src/dom/index.ts", vec![], vec![], vec![]), "typescript"), &["export { render } from './render'"]);
        let real = lang(pf("src/dom/render.ts", vec![sym("render", 1)], vec![], vec![]), "typescript");
        let bench = lang(pf("benchmarks/jsx/hono.ts", vec![sym("render", 1)], vec![], vec![]), "typescript");
        let e = resolve_edges(&[test, idx, real, bench]);
        assert!(has_call(&e, "src/dom/index.test.tsx::t", "src/dom/render.ts::render", 1.0), "{e:?}");
        assert_eq!(calls_of(&e).len(), 1, "{e:?}");
    }
    #[test]
    fn same_name_in_several_files_without_an_import_binds_nothing() {
        let a = lang(pf("src/a.ts", vec![sym("f", 1)], vec![call("f", "helper", None)], vec![]), "typescript");
        let b = lang(pf("src/b.ts", vec![sym("helper", 1)], vec![], vec![]), "typescript");
        let c = lang(pf("src/c.ts", vec![sym("helper", 1)], vec![], vec![]), "typescript");
        assert!(calls_of(&resolve_edges(&[a, b, c])).is_empty());
    }
    #[test]
    fn name_guess_ignores_benchmark_copies() {
        let mk = || lang(pf("src/a.ts", vec![sym("f", 1)], vec![call("f", "helper", None)], vec![]), "typescript");
        let real = lang(pf("src/b.ts", vec![sym("helper", 1)], vec![], vec![]), "typescript");
        let copy = || lang(pf("benchmarks/x/b.ts", vec![sym("helper", 1)], vec![], vec![]), "typescript");
        let e = resolve_edges(&[mk(), copy(), real]);
        assert!(has_call(&e, "src/a.ts::f", "src/b.ts::helper", 0.5), "{e:?}");
        assert_eq!(calls_of(&e).len(), 1, "{e:?}");
        let e = resolve_edges(&[mk(), copy()]);
        assert!(calls_of(&e).is_empty(), "{e:?}");
    }
    #[test]
    fn new_binds_only_to_a_class_never_a_type_alias_or_interface() {
        let a = || {
            let mut site = call("f", "Response", None);
            site.kind = "instantiates".into();
            lang(pf("src/a.ts", vec![sym("f", 1)], vec![site], vec![]), "typescript")
        };
        let other = |path: &str, k: SymbolKind| lang(pf(path, vec![span("Response", k, 1, 2)], vec![], vec![]), "typescript");
        assert!(calls_of(&resolve_edges(&[a(), other("src/types.ts", SymbolKind::Type)])).is_empty());
        assert!(calls_of(&resolve_edges(&[a(), other("src/i.ts", SymbolKind::Interface)])).is_empty());
        assert_eq!(calls_of(&resolve_edges(&[a(), other("src/c.ts", SymbolKind::Class)])).len(), 1);
    }
    #[test]
    fn conflicting_star_reexports_exclude_the_name() {
        let use_ = lang(pf("use.ts", vec![sym("u", 1)], vec![call("u", "x", None)], vec![imp("use.ts", "./star", &["x"], false)]), "typescript");
        let star = with_lines(lang(pf("star.ts", vec![], vec![], vec![]), "typescript"), &["export * from './x1'", "export * from './x2'"]);
        let x1 = lang(pf("x1.ts", vec![sym("x", 1)], vec![], vec![]), "typescript");
        let x2 = lang(pf("x2.ts", vec![sym("x", 1)], vec![], vec![]), "typescript");
        assert!(calls_of(&resolve_edges(&[use_, star, x1, x2])).is_empty());
    }
    #[test]
    fn an_unimported_arrow_binding_keeps_the_name_guess() {
        let c = || lang(pf("c.ts", vec![sym("f", 1)], vec![call("f", "handler", None), call("f", "later", None)], vec![]), "typescript");
        let a = with_lines(lang(pf("a.ts", vec![sym("handler", 1), sym("later", 3)], vec![], vec![]), "typescript"), &["export const handler = () => 1", "", "let later = () => 2"]);
        let e = resolve_edges(&[c(), a]);
        assert!(has_call(&e, "c.ts::f", "a.ts::handler", 0.5), "{e:?}");
        assert!(has_call(&e, "c.ts::f", "a.ts::later", 0.5), "{e:?}");
        assert_eq!(calls_of(&e).len(), 2, "{e:?}");
    }
    #[test]
    fn a_bare_name_never_binds_to_another_files_variable() {
        let a = lang(pf("src/a.ts", vec![sym("f", 1)], vec![call("f", "limit", None), call("f", "real", None)], vec![]), "typescript");
        let b = with_lines(lang(pf("src/b.ts", vec![sym("real", 3)], vec![], vec![]), "typescript"), &["export const limit = 10", "", "function real() {}"]);
        let e = resolve_edges(&[a, b]);
        assert!(has_call(&e, "src/a.ts::f", "src/b.ts::real", 0.5), "{e:?}");
        assert_eq!(calls_of(&e).len(), 1, "{e:?}");
    }
    #[test]
    fn unknown_receiver_method_call_writes_no_edge() {
        let a = lang(pf("a.py", vec![sym("run", 1)], vec![call("run", "get", Some("x"))], vec![]), "python");
        let b = lang(pf("b.py", vec![sym("get", 1)], vec![], vec![]), "python");
        let e = resolve_edges(&[a, b]);
        assert!(calls_of(&e).is_empty(), "{e:?}");
    }
    #[test]
    fn builtin_bare_call_does_not_bind_across_files() {
        let a = lang(
            pf("a.py", vec![sym("run", 1)], vec![call("run", "len", None), call("run", "print", None), call("run", "helper", None)], vec![]),
            "python",
        );
        let b = lang(pf("b.py", vec![sym("len", 1), sym("print", 5), sym("helper", 9)], vec![], vec![]), "python");
        let e = resolve_edges(&[a, b]);
        assert_eq!(calls_of(&e).len(), 1, "{e:?}");
        assert!(has_call(&e, "a.py::run", "b.py::helper", 0.5));
    }
    #[test]
    fn rust_std_method_names_do_not_bind_across_files() {
        let a = lang(pf("a.rs", vec![sym("run", 1)], vec![call("run", "is_empty", None), call("run", "is_empty", Some("v"))], vec![]), "rust");
        let b = lang(pf("b.rs", vec![sym("is_empty", 1)], vec![], vec![]), "rust");
        assert!(calls_of(&resolve_edges(&[a, b])).is_empty());
    }
    #[test]
    fn builtin_name_still_binds_in_the_same_file() {
        let a = lang(pf("a.py", vec![sym("run", 1), sym("len", 5)], vec![call("run", "len", None)], vec![]), "python");
        assert!(has_call(&resolve_edges(&[a]), "a.py::run", "a.py::len", 1.0));
    }
    #[test]
    fn other_object_same_name_call_is_not_a_self_edge() {
        let a = lang(
            pf("server.py", vec![span("search_symbols", SymbolKind::Function, 1, 5)], vec![call("search_symbols", "search_symbols", Some("other"))], vec![]),
            "python",
        );
        let e = resolve_edges(&[a]);
        assert!(calls_of(&e).is_empty(), "{e:?}");
    }
    #[test]
    fn duplicate_edges_collapse_to_max_confidence() {
        let mut v = vec![
            (EdgeKind::Tests, "a".to_string(), "b".to_string(), 0.5),
            (EdgeKind::Tests, "a".to_string(), "b".to_string(), 1.0),
            (EdgeKind::Tests, "a".to_string(), "b".to_string(), 0.5),
        ];
        dedup_max_confidence(&mut v);
        assert_eq!(v, vec![(EdgeKind::Tests, "a".to_string(), "b".to_string(), 1.0)]);
    }
    #[test]
    fn static_class_receiver_binds_the_class_method() {
        let a = lang(pf("a.ts", vec![sym("run", 1)], vec![call("run", "make", Some("Factory"))], vec![]), "typescript");
        let b = lang(pf("b.ts", vec![span("Factory", SymbolKind::Class, 1, 6), span("make", SymbolKind::Method, 2, 3)], vec![], vec![]), "typescript");
        assert!(has_call(&resolve_edges(&[a, b]), "a.ts::run", "b.ts::make", 1.0));
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
        let a = pf("src/a.ts", vec![sym("f", 1)], vec![call("f", "fetchRows", None)], vec![]);
        let mut b = pf("assets/d3.js", vec![sym("fetchRows", 1)], vec![], vec![]);
        b.language = "javascript".into();
        let mut c = pf("lib/x.py", vec![sym("fetchRows", 1)], vec![], vec![]);
        c.language = "python".into();
        let e = resolve_edges(&[a, b, c]);
        assert!(has_call(&e, "src/a.ts::f", "assets/d3.js::fetchRows", 0.5), "{e:?}");
        assert!(!e.iter().any(|x| x.0 == EdgeKind::Calls && x.2 == "lib/x.py::fetchRows"));
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
