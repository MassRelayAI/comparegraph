//! Go context in our own layer (Plan 6 A2). The vendored Go extractor records a call as (callee, last field of the receiver) and keeps
//! no types, so an unqualified call, a method on a declared receiver and a package-qualified call were all lost or guessed by name.
//!
//! What this reads, per file: the package clause; every function and method declaration (with the receiver type); the DECLARED type
//! of each local (`var x T`, `x := T{}`, `x := &T{}`, `x := NewT()`, `x := new(T)`, `x := y.(T)`, a parameter `x T` / `x *T`, the method
//! receiver itself); the struct fields and embedded types of each type declaration; and the operand of every call (`a.b.m()` keeps
//! the whole path `a.b`, not just `b`). Nothing is inferred beyond a declaration: a name declared twice with different types, a
//! range variable and a function result we cannot read are `Unknown`, and an unknown receiver binds nothing.
//!
//! The index joins the files of one directory and package clause (Go's package is the unit): unqualified calls bind to a function of
//! the same package; `pkg.F()` binds through the import path (resolved with the `go.mod` module path) to the imported package; `x.M()`
//! binds to method M of x's declared type, or of a type it embeds. An interface-typed receiver binds NOTHING (interface vs
//! implementation ambiguity), a type conversion `T(x)` is not a call, and a name defined twice in a package (build-tag variants)
//! binds nothing.
use super::vendor::{CallSite, SymbolKind};
use super::ParsedFile;
use std::collections::HashMap;
use tree_sitter::Node;

/// The module path a `go.mod` text declares.
pub fn parse_go_mod(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let l = l.trim();
        let rest = l.strip_prefix("module")?;
        if !rest.starts_with([' ', '\t']) {
            return None;
        }
        let m = rest.trim().trim_matches('"');
        let m = m.split("//").next().unwrap_or(m).trim().trim_matches('"');
        (!m.is_empty()).then(|| m.to_string())
    })
}

/// The `go.mod` module paths of a repo: (directory of the `go.mod` relative to the repo root, module path). Plain data, built once per
/// build and carried by reference (the resolver's workers run on other threads).
#[derive(Debug, Default, Clone)]
pub struct GoModules(Vec<(String, String)>);

impl GoModules {
    pub fn new(mut m: Vec<(String, String)>) -> GoModules {
        m.sort();
        GoModules(m)
    }
}

pub enum ModuleHit {
    /// the repo has no `go.mod` we read: the caller falls back to its path heuristics
    NoModules,
    /// the import path belongs to no module of this repo
    External,
    Dir(String),
}

impl GoModules {
    /// The repo directory an import path names, from the longest module path that is a prefix of it.
    pub fn dir_of(&self, source: &str) -> ModuleHit {
        let mods = &self.0;
        if mods.is_empty() {
            return ModuleHit::NoModules;
        }
        let best = mods
            .iter()
            .filter(|(_, m)| source == m || source.strip_prefix(m.as_str()).is_some_and(|r| r.starts_with('/')))
            .max_by_key(|(_, m)| m.len());
        match best {
            None => ModuleHit::External,
            Some((dir, m)) => {
                let rest = source[m.len()..].trim_start_matches('/');
                let d = match (dir.is_empty(), rest.is_empty()) {
                    (true, _) => rest.to_string(),
                    (false, true) => dir.clone(),
                    (false, false) => format!("{dir}/{rest}"),
                };
                ModuleHit::Dir(d)
            }
        }
    }
}

/// A type as written: `T`, `*T`, `pkg.T`, `T[int]` → (package qualifier, name).
#[derive(Debug, Clone, PartialEq)]
pub struct TypeRef {
    pub qual: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Known(TypeRef),
    /// the first result type of the function `qual.name(...)` (or `name(...)`), settled by the index
    Result(Option<String>, String),
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    /// `f()`
    Bare,
    /// `a.m()` → ["a"], `a.b.m()` → ["a", "b"]
    Path(Vec<String>),
    /// anything else (`f().m()`, `x[i].m()`, `(*p).m()`, …) or two different operands for one (line, callee)
    Complex,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Struct,
    Interface,
    Other,
}

#[derive(Debug)]
pub struct GoType {
    pub name: String,
    pub kind: Kind,
    pub fields: HashMap<String, Option<TypeRef>>,
    /// None = an embedded type we cannot read (a type parameter, a shape we do not parse)
    pub embeds: Vec<Option<TypeRef>>,
}

#[derive(Debug)]
pub struct GoFn {
    pub name: String,
    /// the receiver's base type for a method
    pub recv: Option<TypeRef>,
    pub start: u32,
    pub end: u32,
    pub locals: HashMap<String, Ty>,
}

#[derive(Debug, Default)]
pub struct GoInfo {
    pub package: String,
    pub fns: Vec<GoFn>,
    pub types: Vec<GoType>,
    /// package-level `var x T` / `var x = T{}` of this file
    pub vars: HashMap<String, Ty>,
    /// top-level function → first result type (a name declared twice is dropped to None)
    pub returns: HashMap<String, Option<TypeRef>>,
    /// (line, callee) → operand
    pub sites: HashMap<(u32, String), Operand>,
}

fn text<'a>(n: Node, src: &'a str) -> &'a str {
    &src[n.byte_range()]
}

fn type_ref(n: Node, src: &str) -> Option<TypeRef> {
    match n.kind() {
        "type_identifier" => Some(TypeRef { qual: None, name: text(n, src).to_string() }),
        "qualified_type" => Some(TypeRef {
            qual: Some(text(n.child_by_field_name("package")?, src).to_string()),
            name: text(n.child_by_field_name("name")?, src).to_string(),
        }),
        "pointer_type" | "parenthesized_type" => type_ref(n.named_child(0)?, src),
        "generic_type" => type_ref(n.child_by_field_name("type")?, src),
        _ => None,
    }
}

fn put(m: &mut HashMap<String, Ty>, name: &str, t: Ty) {
    if name == "_" {
        return;
    }
    match m.get(name) {
        Some(old) if *old != t => {
            m.insert(name.to_string(), Ty::Unknown);
        }
        Some(_) => {}
        None => {
            m.insert(name.to_string(), t);
        }
    }
}

/// The type a value expression declares by its shape.
fn value_ty(v: Node, src: &str) -> Ty {
    match v.kind() {
        "composite_literal" => v.child_by_field_name("type").and_then(|t| type_ref(t, src)).map_or(Ty::Unknown, Ty::Known),
        "unary_expression" => match (v.child_by_field_name("operator").map(|o| text(o, src)), v.child_by_field_name("operand")) {
            (Some("&"), Some(o)) if o.kind() == "composite_literal" => value_ty(o, src),
            _ => Ty::Unknown,
        },
        "parenthesized_expression" => v.named_child(0).map_or(Ty::Unknown, |i| value_ty(i, src)),
        "type_assertion_expression" => v.child_by_field_name("type").and_then(|t| type_ref(t, src)).map_or(Ty::Unknown, Ty::Known),
        "call_expression" => {
            let Some(f) = v.child_by_field_name("function") else { return Ty::Unknown };
            match f.kind() {
                "identifier" if text(f, src) == "new" => v
                    .child_by_field_name("arguments")
                    .and_then(|a| a.named_child(0))
                    .and_then(|t| type_ref(t, src))
                    .map_or(Ty::Unknown, Ty::Known),
                "identifier" => Ty::Result(None, text(f, src).to_string()),
                "selector_expression" => match (f.child_by_field_name("operand"), f.child_by_field_name("field")) {
                    (Some(o), Some(n)) if o.kind() == "identifier" => Ty::Result(Some(text(o, src).to_string()), text(n, src).to_string()),
                    _ => Ty::Unknown,
                },
                _ => Ty::Unknown,
            }
        }
        _ => Ty::Unknown,
    }
}

fn idents<'a>(list: Node<'a>, src: &str) -> Vec<(String, Node<'a>)> {
    let mut c = list.walk();
    if list.kind() == "identifier" {
        return vec![(text(list, src).to_string(), list)];
    }
    list.named_children(&mut c).map(|n| (if n.kind() == "identifier" { text(n, src).to_string() } else { String::new() }, n)).collect()
}

/// `names = values` (a `:=` or a `var` spec): pairwise when the counts match; one call or assertion feeding several names
/// types the first only; anything else is Unknown.
fn bind_values(locals: &mut HashMap<String, Ty>, names: &[(String, Node)], values: Option<Node>, src: &str) {
    let vals: Vec<Node> = values.map(|v| if v.kind() == "expression_list" { let mut c = v.walk(); v.named_children(&mut c).collect() } else { vec![v] }).unwrap_or_default();
    for (i, (n, _)) in names.iter().enumerate() {
        if n.is_empty() {
            continue;
        }
        let t = if vals.len() == names.len() {
            value_ty(vals[i], src)
        } else if vals.len() == 1 && i == 0 {
            value_ty(vals[0], src)
        } else {
            Ty::Unknown
        };
        put(locals, n, t);
    }
}

fn collect_locals(n: Node, src: &str, locals: &mut HashMap<String, Ty>) {
    match n.kind() {
        "parameter_declaration" => {
            let t = n.child_by_field_name("type").and_then(|t| type_ref(t, src)).map_or(Ty::Unknown, Ty::Known);
            let mut c = n.walk();
            for name in n.children_by_field_name("name", &mut c) {
                put(locals, text(name, src), t.clone());
            }
        }
        "variadic_parameter_declaration" => {
            if let Some(name) = n.child_by_field_name("name") {
                put(locals, text(name, src), Ty::Unknown);
            }
        }
        "var_spec" => {
            let mut c = n.walk();
            let names: Vec<(String, Node)> = n.children_by_field_name("name", &mut c).map(|x| (text(x, src).to_string(), x)).collect();
            if let Some(t) = n.child_by_field_name("type") {
                let t = type_ref(t, src).map_or(Ty::Unknown, Ty::Known);
                for (nm, _) in &names {
                    put(locals, nm, t.clone());
                }
            } else {
                bind_values(locals, &names, n.child_by_field_name("value"), src);
            }
        }
        "short_var_declaration" => {
            if let Some(l) = n.child_by_field_name("left") {
                bind_values(locals, &idents(l, src), n.child_by_field_name("right"), src);
            }
        }
        "range_clause" => {
            if let Some(l) = n.child_by_field_name("left") {
                for (nm, _) in idents(l, src) {
                    if !nm.is_empty() {
                        put(locals, &nm, Ty::Unknown);
                    }
                }
            }
        }
        "type_switch_statement" => {
            if let Some(a) = n.child_by_field_name("alias") {
                for (nm, _) in idents(a, src) {
                    if !nm.is_empty() {
                        put(locals, &nm, Ty::Unknown);
                    }
                }
            }
        }
        _ => {}
    }
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        collect_locals(ch, src, locals);
    }
}

/// `a.b.c` made only of identifiers → ["a", "b", "c"].
fn ident_path(n: Node, src: &str) -> Option<Vec<String>> {
    match n.kind() {
        "identifier" => Some(vec![text(n, src).to_string()]),
        "selector_expression" => {
            let mut p = ident_path(n.child_by_field_name("operand")?, src)?;
            p.push(text(n.child_by_field_name("field")?, src).to_string());
            Some(p)
        }
        _ => None,
    }
}

fn collect_sites(n: Node, src: &str, sites: &mut HashMap<(u32, String), Operand>) {
    if n.kind() == "call_expression" {
        if let Some(f) = n.child_by_field_name("function") {
            let hit = match f.kind() {
                "identifier" => Some((text(f, src).to_string(), Operand::Bare)),
                "selector_expression" => f.child_by_field_name("field").map(|m| {
                    let op = f.child_by_field_name("operand").and_then(|o| ident_path(o, src)).map_or(Operand::Complex, Operand::Path);
                    (text(m, src).to_string(), op)
                }),
                _ => None,
            };
            if let Some((callee, op)) = hit {
                let key = (n.start_position().row as u32 + 1, callee);
                match sites.get(&key) {
                    Some(old) if *old != op => {
                        sites.insert(key, Operand::Complex);
                    }
                    Some(_) => {}
                    None => {
                        sites.insert(key, op);
                    }
                }
            }
        }
    }
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        collect_sites(ch, src, sites);
    }
}

/// The type parameter names a declaration introduces: `func F[T any, U any]`, a method on `Stack[T]`, `type Box[T any] struct`.
fn type_params(n: Node, src: &str) -> Vec<String> {
    let mut out = vec![];
    if let Some(list) = n.child_by_field_name("type_parameters") {
        let mut c = list.walk();
        for d in list.named_children(&mut c) {
            let mut c2 = d.walk();
            for nm in d.children_by_field_name("name", &mut c2) {
                out.push(text(nm, src).to_string());
            }
        }
    }
    if n.kind() == "method_declaration" {
        // `func (s *Stack[T]) M()`: the receiver's type arguments name its type parameters
        let mut t = n.child_by_field_name("receiver").and_then(|r| r.named_child(0)).and_then(|d| d.child_by_field_name("type"));
        while let Some(x) = t {
            if x.kind() == "generic_type" {
                if let Some(args) = x.child_by_field_name("type_arguments") {
                    let mut c = args.walk();
                    for a in args.named_children(&mut c) {
                        let a = if a.kind() == "type_elem" { a.named_child(0).unwrap_or(a) } else { a };
                        out.push(text(a, src).to_string());
                    }
                }
                break;
            }
            t = if x.kind() == "pointer_type" { x.named_child(0) } else { None };
        }
    }
    out
}

fn first_result(n: Node, src: &str) -> Option<TypeRef> {
    let r = n.child_by_field_name("result")?;
    if r.kind() == "parameter_list" {
        let first = r.named_child(0)?;
        type_ref(first.child_by_field_name("type")?, src)
    } else {
        type_ref(r, src)
    }
}

fn type_decl(spec: Node, src: &str) -> Option<GoType> {
    let name = text(spec.child_by_field_name("name")?, src).to_string();
    let ty = spec.child_by_field_name("type");
    let kind = match ty.map(|t| t.kind()) {
        Some("struct_type") => Kind::Struct,
        Some("interface_type") => Kind::Interface,
        _ => Kind::Other,
    };
    let tp = type_params(spec, src);
    let mut g = GoType { name, kind, fields: HashMap::new(), embeds: vec![] };
    if let (Kind::Struct, Some(t)) = (kind, ty) {
        let mut c = t.walk();
        for list in t.named_children(&mut c).filter(|x| x.kind() == "field_declaration_list") {
            let mut c2 = list.walk();
            for fd in list.named_children(&mut c2).filter(|x| x.kind() == "field_declaration") {
                let ft = fd.child_by_field_name("type").and_then(|t| type_ref(t, src)).filter(|t| t.qual.is_some() || !tp.contains(&t.name));
                let mut c3 = fd.walk();
                let names: Vec<String> = fd.children_by_field_name("name", &mut c3).map(|x| text(x, src).to_string()).collect();
                if names.is_empty() {
                    g.embeds.push(ft);
                } else {
                    for nm in names {
                        g.fields.insert(nm, ft.clone());
                    }
                }
            }
        }
    }
    Some(g)
}

pub fn analyze(src: &str) -> GoInfo {
    let mut info = GoInfo::default();
    let mut p = tree_sitter::Parser::new();
    if p.set_language(&tree_sitter_go::LANGUAGE.into()).is_err() {
        return info;
    }
    let Some(tree) = p.parse(src, None) else { return info };
    let root = tree.root_node();
    collect_sites(root, src, &mut info.sites);
    let mut c = root.walk();
    for top in root.named_children(&mut c) {
        match top.kind() {
            "package_clause" => {
                if let Some(n) = top.named_child(0) {
                    info.package = text(n, src).to_string();
                }
            }
            "function_declaration" | "method_declaration" => {
                let Some(name) = top.child_by_field_name("name") else { continue };
                let name = text(name, src).to_string();
                let is_method = top.kind() == "method_declaration";
                let recv = if is_method {
                    top.child_by_field_name("receiver")
                        .and_then(|r| r.named_child(0))
                        .and_then(|d| d.child_by_field_name("type"))
                        .and_then(|t| type_ref(t, src))
                } else {
                    None
                };
                let mut locals = HashMap::new();
                collect_locals(top, src, &mut locals);
                // a type parameter is no package type, even when a package type has its name
                let tp = type_params(top, src);
                for t in locals.values_mut() {
                    if matches!(t, Ty::Known(r) if r.qual.is_none() && tp.contains(&r.name)) {
                        *t = Ty::Unknown;
                    }
                }
                if !is_method {
                    let r = first_result(top, src).filter(|r| r.qual.is_some() || !tp.contains(&r.name));
                    match info.returns.get(&name) {
                        Some(old) if *old != r => {
                            info.returns.insert(name.clone(), None);
                        }
                        Some(_) => {}
                        None => {
                            info.returns.insert(name.clone(), r);
                        }
                    }
                }
                info.fns.push(GoFn { name, recv, start: top.start_position().row as u32 + 1, end: top.end_position().row as u32 + 1, locals });
            }
            "type_declaration" => {
                let mut c2 = top.walk();
                for spec in top.named_children(&mut c2).filter(|s| matches!(s.kind(), "type_spec" | "type_alias")) {
                    if let Some(t) = type_decl(spec, src) {
                        info.types.push(t);
                    }
                }
            }
            "var_declaration" => collect_locals(top, src, &mut info.vars),
            _ => {}
        }
    }
    info
}

// ---- the index over a repo ----

struct Pkg {
    name: String,
    /// name → [(qn, declared in a _test.go file)]
    funcs: HashMap<String, Vec<(String, bool)>>,
    /// (type, method) → same
    methods: HashMap<(String, String), Vec<(String, bool)>>,
    /// type name → (file, index in that file's types); a name declared twice is dropped
    types: HashMap<String, Option<(usize, usize)>>,
    /// package-level vars: name → (file, Ty); two different declarations make Unknown
    vars: HashMap<String, (usize, Ty)>,
    /// top-level function → (file, first result)
    returns: HashMap<String, Option<(usize, TypeRef)>>,
}

/// The package and type name a type resolves to.
#[derive(Debug, Clone, PartialEq)]
struct Home {
    pkg: usize,
    name: String,
}

pub enum Outcome {
    Bound(String),
    Nothing,
    /// not a Go file, or not a case this layer decides
    Fallthrough,
}

pub struct GoIndex<'a> {
    files: &'a [ParsedFile],
    infos: Vec<Option<GoInfo>>,
    pkg_of: Vec<usize>,
    pkgs: Vec<Pkg>,
    /// per file: names an import binds → the imported package
    aliases: Vec<HashMap<String, usize>>,
    is_test: Vec<bool>,
}

fn is_test_file(p: &str) -> bool {
    p.ends_with("_test.go")
}

fn dir_of(p: &str) -> &str {
    p.rsplit_once('/').map_or("", |d| d.0)
}

impl<'a> GoIndex<'a> {
    /// `import_dir` maps an import path to the repo directory it names.
    pub fn build(files: &'a [ParsedFile], qns: &[Vec<String>], import_dir: &dyn Fn(&str) -> Option<String>) -> Option<GoIndex<'a>> {
        if !files.iter().any(|f| f.language == "go") {
            return None;
        }
        let infos: Vec<Option<GoInfo>> = files.iter().map(|f| (f.language == "go").then(|| analyze(&f.lines.join("\n")))).collect();
        let is_test: Vec<bool> = files.iter().map(|f| is_test_file(&f.rel_path)).collect();
        let mut keys: HashMap<(String, String), usize> = HashMap::new();
        let mut pkgs: Vec<Pkg> = vec![];
        let mut pkg_of = vec![usize::MAX; files.len()];
        for (fi, f) in files.iter().enumerate() {
            let Some(info) = &infos[fi] else { continue };
            let key = (dir_of(&f.rel_path).to_string(), info.package.clone());
            let id = *keys.entry(key).or_insert_with(|| {
                pkgs.push(Pkg { name: info.package.clone(), funcs: HashMap::new(), methods: HashMap::new(), types: HashMap::new(), vars: HashMap::new(), returns: HashMap::new() });
                pkgs.len() - 1
            });
            pkg_of[fi] = id;
        }
        for (fi, f) in files.iter().enumerate() {
            let Some(info) = &infos[fi] else { continue };
            let pkg = &mut pkgs[pkg_of[fi]];
            for g in &info.fns {
                // the symbol this declaration is: same name, same first line
                let Some(qn) = f.symbols.iter().zip(&qns[fi]).find(|(s, _)| s.name == g.name && s.line_start == g.start && matches!(s.kind, SymbolKind::Function | SymbolKind::Method)).map(|(_, q)| q.clone()) else {
                    continue;
                };
                match &g.recv {
                    Some(r) => pkg.methods.entry((r.name.clone(), g.name.clone())).or_default().push((qn, is_test[fi])),
                    None => pkg.funcs.entry(g.name.clone()).or_default().push((qn, is_test[fi])),
                }
            }
            for (ti, t) in info.types.iter().enumerate() {
                pkg.types.entry(t.name.clone()).and_modify(|e| *e = None).or_insert(Some((fi, ti)));
            }
            for (n, t) in &info.vars {
                match pkg.vars.get(n) {
                    Some((_, old)) if old != t => {
                        pkg.vars.insert(n.clone(), (fi, Ty::Unknown));
                    }
                    Some(_) => {}
                    None => {
                        pkg.vars.insert(n.clone(), (fi, t.clone()));
                    }
                }
            }
            for (n, r) in &info.returns {
                let r = r.clone().map(|r| (fi, r));
                match pkg.returns.get(n) {
                    Some(_) => {
                        pkg.returns.insert(n.clone(), None);
                    }
                    None => {
                        pkg.returns.insert(n.clone(), r);
                    }
                }
            }
        }
        // imports: dir → its one non-test package
        let mut dir_pkgs: HashMap<String, Vec<usize>> = HashMap::new();
        for ((dir, name), id) in &keys {
            if !name.ends_with("_test") {
                dir_pkgs.entry(dir.clone()).or_default().push(*id);
            }
        }
        let mut memo: HashMap<String, Option<usize>> = HashMap::new();
        let mut aliases: Vec<HashMap<String, usize>> = vec![HashMap::new(); files.len()];
        for (fi, f) in files.iter().enumerate() {
            if infos[fi].is_none() {
                continue;
            }
            let mut dead: std::collections::HashSet<String> = std::collections::HashSet::new();
            for imp in &f.imports {
                let target = *memo.entry(imp.source.clone()).or_insert_with(|| {
                    let dir = import_dir(&imp.source)?;
                    match dir_pkgs.get(&dir).map(Vec::as_slice) {
                        Some([one]) => Some(*one),
                        _ => None,
                    }
                });
                let Some(t) = target else { continue };
                let last = imp.source.rsplit('/').next().unwrap_or(&imp.source);
                let spec = imp.specifiers.first().map(String::as_str).unwrap_or(last);
                if matches!(spec, "_" | ".") {
                    continue;
                }
                // the vendored import keeps one name: the explicit alias, else the last path segment. An explicit alias is honoured; a plain
                // import is also known by the package clause (`.../v2` is `caddy`). Two imports claiming one name bind it to neither.
                let mut names = vec![spec.to_string()];
                if spec == last && pkgs[t].name != spec {
                    names.push(pkgs[t].name.clone());
                }
                for name in names {
                    if dead.contains(&name) {
                        continue;
                    }
                    match aliases[fi].get(&name) {
                        Some(old) if *old != t => {
                            aliases[fi].remove(&name);
                            dead.insert(name);
                        }
                        _ => {
                            aliases[fi].insert(name, t);
                        }
                    }
                }
            }
        }
        Some(GoIndex { files, infos, pkg_of, pkgs, aliases, is_test })
    }

    fn info(&self, fi: usize) -> &GoInfo {
        self.infos[fi].as_ref().expect("go file")
    }

    /// Resolve a type as written in file `fi`.
    fn home(&self, fi: usize, t: &TypeRef) -> Option<Home> {
        let pkg = match &t.qual {
            None => self.pkg_of[fi],
            Some(q) => *self.aliases[fi].get(q)?,
        };
        Some(Home { pkg, name: t.name.clone() })
    }

    fn type_decl(&self, h: &Home) -> Option<(usize, &GoType)> {
        let (fi, ti) = (*self.pkgs[h.pkg].types.get(&h.name)?)?;
        Some((fi, &self.info(fi).types[ti]))
    }

    fn ty_home(&self, fi: usize, t: &Ty) -> Option<Home> {
        match t {
            Ty::Known(r) => self.home(fi, r),
            Ty::Result(q, n) => {
                let pkg = match q {
                    None => self.pkg_of[fi],
                    Some(q) => *self.aliases[fi].get(q)?,
                };
                let (rf, r) = self.pkgs[pkg].returns.get(n)?.as_ref()?;
                self.home(*rf, r)
            }
            Ty::Unknown => None,
        }
    }

    /// Is the first path segment a variable here? `Some(None)` = a variable of unknown type.
    fn var_home(&self, fi: usize, f: Option<&GoFn>, name: &str) -> Option<Option<Home>> {
        if let Some(t) = f.and_then(|f| f.locals.get(name)) {
            return Some(self.ty_home(fi, t));
        }
        let (vf, t) = self.pkgs[self.pkg_of[fi]].vars.get(name)?;
        Some(self.ty_home(*vf, t))
    }

    /// The declared type of field `name` of `h` (directly, or promoted through an embedded struct).
    fn field_home(&self, h: &Home, name: &str, depth: u8) -> Option<Home> {
        let (df, d) = self.type_decl(h)?;
        if let Some(t) = d.fields.get(name) {
            return self.home(df, t.as_ref()?);
        }
        if depth == 0 {
            return None;
        }
        // an embedded type we cannot resolve (`sync.Mutex`, a type parameter) might supply the field too: then nothing is known
        let mut homes = vec![];
        for e in &d.embeds {
            homes.push(self.home(df, e.as_ref()?)?);
        }
        homes.iter().find_map(|eh| self.field_home(eh, name, depth - 1))
    }

    /// Method `m` of type `h`, its own or promoted through an embedded type. `Err` = it is an interface or ambiguous: bind nothing.
    fn method(&self, h: &Home, m: &str, from_pkg: usize, depth: u8) -> Result<Option<String>, ()> {
        let (df, d) = self.type_decl(h).ok_or(())?;
        if d.kind == Kind::Interface {
            return Err(());
        }
        let ms = self.pkgs[h.pkg].methods.get(&(h.name.clone(), m.to_string()));
        if let Some(ms) = ms {
            let vis: Vec<&(String, bool)> = ms.iter().filter(|(_, t)| !t || h.pkg == from_pkg).collect();
            return match vis.as_slice() {
                [(q, _)] => Ok(Some(q.clone())),
                _ => Err(()),
            };
        }
        if depth == 0 {
            return Ok(None);
        }
        let mut found = None;
        for e in &d.embeds {
            // an embedded type we cannot resolve might supply the method too: ambiguous, bind nothing
            let Some(e) = e.as_ref().and_then(|e| self.home(df, e)) else { return Err(()) };
            match self.method(&e, m, from_pkg, depth - 1) {
                Ok(Some(q)) => {
                    if found.is_some() {
                        return Err(());
                    }
                    found = Some(q);
                }
                Ok(None) => {}
                // an embedded interface may also provide it: ambiguous, bind nothing
                Err(()) => return Err(()),
            }
        }
        Ok(found)
    }

    pub fn resolve(&self, fi: usize, c: &CallSite) -> Outcome {
        let Some(info) = self.infos[fi].as_ref() else { return Outcome::Fallthrough };
        if c.kind == "refs" {
            return Outcome::Fallthrough;
        }
        let Some(op) = info.sites.get(&(c.line, c.callee.clone())) else {
            // a `go` statement's or send's synthetic site, a function literal, a generic instantiation
            return Outcome::Nothing;
        };
        let pkg = self.pkg_of[fi];
        let enclosing = (c.caller != super::js_ctx::CALLER_FILE)
            .then(|| info.fns.iter().filter(|g| g.name == c.caller && g.start <= c.line && c.line <= g.end).min_by_key(|g| g.end - g.start))
            .flatten();
        let callee = c.callee.as_str();
        let bound = |v: &[(String, bool)], cross: bool| -> Outcome {
            let vis: Vec<&(String, bool)> = v.iter().filter(|(_, t)| !t || !cross).collect();
            match vis.as_slice() {
                [(q, _)] => Outcome::Bound(q.clone()),
                _ => Outcome::Nothing,
            }
        };
        match op {
            Operand::Complex => Outcome::Nothing,
            Operand::Bare => {
                // a closure or function-typed variable, a conversion `T(x)` and a builtin are no package function
                if self.var_home(fi, enclosing, callee).is_some() || self.pkgs[pkg].types.contains_key(callee) || super::builtins::is_builtin("go", callee) {
                    return Outcome::Nothing;
                }
                match self.pkgs[pkg].funcs.get(callee) {
                    Some(v) => bound(v, false),
                    None => Outcome::Nothing,
                }
            }
            Operand::Path(path) => {
                let first = path[0].as_str();
                match self.var_home(fi, enclosing, first) {
                    Some(h) => {
                        let Some(mut h) = h else { return Outcome::Nothing };
                        for field in &path[1..] {
                            match self.field_home(&h, field, 3) {
                                Some(n) => h = n,
                                None => return Outcome::Nothing,
                            }
                        }
                        match self.method(&h, callee, pkg, 3) {
                            Ok(Some(q)) => Outcome::Bound(q),
                            _ => Outcome::Nothing,
                        }
                    }
                    None => {
                        // `pkg.F()`: an imported package's top-level function
                        if path.len() != 1 {
                            return Outcome::Nothing;
                        }
                        let Some(&target) = self.aliases[fi].get(first) else { return Outcome::Nothing };
                        match self.pkgs[target].funcs.get(callee) {
                            Some(v) => bound(v, target != pkg),
                            None => Outcome::Nothing,
                        }
                    }
                }
            }
        }
    }

    pub fn is_test(&self, fi: usize) -> bool {
        self.is_test[fi]
    }

    pub fn files(&self) -> &[ParsedFile] {
        self.files
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_mod_module_line() {
        assert_eq!(parse_go_mod("// c\nmodule github.com/a/b/v2 // x\n\ngo 1.21\n").as_deref(), Some("github.com/a/b/v2"));
        assert_eq!(parse_go_mod("module \"x\"\n").as_deref(), Some("x"));
        assert_eq!(parse_go_mod("modulex foo\n"), None);
    }

    #[test]
    fn locals_come_from_declarations_only() {
        let src = "package p\n\nfunc (h adminHandler) m(d *Dispenser, e Other) {\n\tvar a T\n\tb := U{}\n\tc := &V{}\n\td2 := NewW()\n\te2, err := x.NewZ()\n\tfor i := range xs {\n\t}\n\tf := g()\n\tvar a T2\n}\n";
        let i = analyze(src);
        let l = &i.fns[0].locals;
        let k = |n: &str| Ty::Known(TypeRef { qual: None, name: n.into() });
        assert_eq!(l["h"], k("adminHandler"));
        assert_eq!(l["d"], k("Dispenser"));
        assert_eq!(l["b"], k("U"));
        assert_eq!(l["c"], k("V"));
        assert_eq!(l["d2"], Ty::Result(None, "NewW".into()));
        assert_eq!(l["e2"], Ty::Result(Some("x".into()), "NewZ".into()));
        assert_eq!(l["err"], Ty::Unknown);
        assert_eq!(l["i"], Ty::Unknown);
        assert_eq!(l["a"], Ty::Unknown, "declared twice with different types");
        assert_eq!(i.fns[0].recv, Some(TypeRef { qual: None, name: "adminHandler".into() }));
    }

    #[test]
    fn operands_keep_the_whole_path() {
        let i = analyze("package p\n\nfunc f() {\n\ta.b.m()\n\tg()\n\th().n()\n}\n");
        assert_eq!(i.sites[&(4, "m".into())], Operand::Path(vec!["a".into(), "b".into()]));
        assert_eq!(i.sites[&(5, "g".into())], Operand::Bare);
        assert_eq!(i.sites[&(6, "n".into())], Operand::Complex);
    }
}
