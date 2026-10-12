//! Python context in our own layer (the vendored Python extractor drops `import a as b`, `from m import f as g` aliases and keeps no
//! variable types): imports with aliases, and the DECLARED or CONSTRUCTED type of variables, read with tree-sitter-python.
//! Declared means an annotation (`x: Foo`, `def f(x: Foo)`, `self.x: Foo`), a constructor call (`x = Foo(...)`, `self.x = Foo(...)`)
//! or `with Foo(...) as y`. Nothing is inferred beyond that.
use std::collections::HashMap;
use tree_sitter::Node;

#[derive(Debug, PartialEq, Clone)]
pub struct PyImport {
    /// the name this import binds in the file
    pub alias: String,
    /// dotted module path, leading dots kept for relative imports (`.core`, `..a.b`)
    pub module: String,
    /// `from module import name` → Some(name); `import module` → None
    pub name: Option<String>,
}

#[derive(Debug, Default, PartialEq)]
pub struct PyScope {
    pub start: u32,
    pub end: u32,
    /// variable → declared class name
    pub vars: HashMap<String, String>,
    /// names declared `global` / `nonlocal` here: they are not locals and shadow nothing
    pub globals: Vec<String>,
}

#[derive(Debug, Default, PartialEq)]
pub struct PyClassCtx {
    pub start: u32,
    pub end: u32,
    /// attribute → declared class name (`self.x = Foo()`, class-level `x: Foo`)
    pub attrs: HashMap<String, String>,
}

#[derive(Debug, Default, PartialEq)]
pub struct PyInfo {
    pub imports: Vec<PyImport>,
    pub scopes: Vec<PyScope>,
    pub classes: Vec<PyClassCtx>,
    pub module_vars: HashMap<String, String>,
    /// function name → class named by its `-> T` annotation
    pub returns: HashMap<String, String>,
    /// function name → the class of each element of its `-> tuple[A, B]` / `Tuple[A, B]` annotation ("" = not a class)
    pub tuples: HashMap<String, Vec<String>>,
    /// every function name seen so far, and those seen twice (a name defined twice types nothing)
    seen_fns: Vec<String>,
    dup_fns: Vec<String>,
}

fn text<'a>(n: Node, src: &'a str) -> &'a str {
    &src[n.byte_range()]
}

/// A class by naming convention: a capital letter after any leading underscores (`Foo`, `_ProgressLine`).
pub fn class_like(name: &str) -> bool {
    name.trim_start_matches('_').chars().next().is_some_and(char::is_uppercase)
}

/// `Optional[Foo]`, `Foo | None`, `"Foo"`, `pkg.Foo[int]` → `Foo`; only a capitalised class-like name qualifies.
pub fn annotation_type(t: &str) -> Option<String> {
    let t: String = t.trim().trim_matches(|c| c == '"' || c == '\'').chars().filter(|c| !c.is_whitespace()).collect();
    let parts: Vec<&str> = t.split('|').filter(|p| *p != "None").collect();
    let [one] = parts.as_slice() else { return None };
    let mut one = *one;
    if let Some(inner) = one.strip_prefix("Optional[").and_then(|r| r.strip_suffix(']')) {
        return annotation_type(inner);
    }
    one = one.split('[').next()?;
    let last = one.rsplit('.').next()?;
    (class_like(last) && last.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| last.to_string())
}

/// `tuple[A, B]`, `Tuple[A, B]`, `typing.Tuple[A, B]` → one class name per element ("" for an element that is no class). A `...` makes none.
pub fn tuple_annotation(t: &str) -> Option<Vec<String>> {
    let t: String = t.trim().trim_matches(|c| c == '"' || c == '\'').chars().filter(|c| !c.is_whitespace()).collect();
    let inner = ["tuple[", "Tuple[", "typing.Tuple["].iter().find_map(|p| t.strip_prefix(p))?.strip_suffix(']')?;
    let (mut parts, mut depth, mut cur) = (vec![], 0i32, String::new());
    for ch in inner.chars() {
        match ch {
            '[' => depth += 1,
            ']' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(ch);
    }
    parts.push(cur);
    if depth != 0 || parts.iter().any(|p| p == "..." || p.is_empty()) || parts.len() < 2 {
        return None;
    }
    Some(parts.iter().map(|p| annotation_type(p).unwrap_or_default()).collect())
}

/// `Foo(...)`, `pkg.Foo(...)` → `Foo` (a capitalised callee is a class by convention; `await` and parentheses are looked through).
/// `make(...)` → `()make`: the type is whatever `make` is annotated to return, settled by the resolver.
fn constructed_type(v: Node, src: &str) -> Option<String> {
    let v = match v.kind() {
        "await" | "parenthesized_expression" => v.named_child(0)?,
        _ => v,
    };
    if v.kind() != "call" {
        return None;
    }
    let f = v.child_by_field_name("function")?;
    let name = match f.kind() {
        "identifier" => text(f, src),
        "attribute" => text(f.child_by_field_name("attribute")?, src),
        _ => return None,
    };
    if class_like(name) {
        Some(name.to_string())
    } else if f.kind() == "identifier" {
        Some(format!("(){name}"))
    } else {
        None
    }
}

struct Cx {
    scope: Option<usize>,
    class: Option<usize>,
}

fn bind(info: &mut PyInfo, cx: &Cx, name: &str, ty: String) {
    match (cx.scope, cx.class) {
        (Some(s), _) if info.scopes[s].globals.iter().any(|g| g == name) => {
            if !ty.is_empty() {
                info.module_vars.insert(name.to_string(), ty);
            }
        }
        (Some(s), _) => {
            info.scopes[s].vars.insert(name.to_string(), ty);
        }
        (None, Some(c)) => {
            info.classes[c].attrs.insert(name.to_string(), ty);
        }
        (None, None) => {
            info.module_vars.insert(name.to_string(), ty);
        }
    }
}

fn walk(n: Node, src: &str, info: &mut PyInfo, cx: &Cx) {
    let (s, e) = (n.start_position().row as u32 + 1, n.end_position().row as u32 + 1);
    match n.kind() {
        "import_statement" => {
            let mut c = n.walk();
            for ch in n.children_by_field_name("name", &mut c) {
                match ch.kind() {
                    "aliased_import" => {
                        if let (Some(m), Some(a)) = (ch.child_by_field_name("name"), ch.child_by_field_name("alias")) {
                            info.imports.push(PyImport { alias: text(a, src).into(), module: text(m, src).into(), name: None });
                        }
                    }
                    "dotted_name" => {
                        let m = text(ch, src);
                        // `import a.b` binds `a`; the vendored receiver of `a.b.f()` is the last attribute `b`
                        for seg in [m.split('.').next(), m.rsplit('.').next()].into_iter().flatten() {
                            let module = if seg == m.split('.').next().unwrap_or(m) { seg.to_string() } else { m.to_string() };
                            info.imports.push(PyImport { alias: seg.into(), module, name: None });
                        }
                    }
                    _ => {}
                }
            }
            return;
        }
        "import_from_statement" => {
            let Some(module) = n.child_by_field_name("module_name").map(|m| text(m, src).to_string()) else { return };
            let mut c = n.walk();
            for ch in n.children_by_field_name("name", &mut c) {
                match ch.kind() {
                    "aliased_import" => {
                        if let (Some(nm), Some(a)) = (ch.child_by_field_name("name"), ch.child_by_field_name("alias")) {
                            info.imports.push(PyImport { alias: text(a, src).into(), module: module.clone(), name: Some(text(nm, src).into()) });
                        }
                    }
                    "dotted_name" | "identifier" => {
                        let nm = text(ch, src);
                        info.imports.push(PyImport { alias: nm.into(), module: module.clone(), name: Some(nm.into()) });
                    }
                    _ => {}
                }
            }
            return;
        }
        "global_statement" | "nonlocal_statement" => {
            if let Some(sc) = cx.scope {
                let mut c = n.walk();
                let names: Vec<String> = n.named_children(&mut c).filter(|x| x.kind() == "identifier").map(|x| text(x, src).to_string()).collect();
                info.scopes[sc].globals.extend(names);
            }
            return;
        }
        "class_definition" => {
            info.classes.push(PyClassCtx { start: s, end: e, ..Default::default() });
            let inner = Cx { scope: None, class: Some(info.classes.len() - 1) };
            let mut c = n.walk();
            for ch in n.named_children(&mut c) {
                walk(ch, src, info, &inner);
            }
            return;
        }
        "function_definition" => {
            info.scopes.push(PyScope { start: s, end: e, ..Default::default() });
            if let Some(name) = n.child_by_field_name("name") {
                let nm = text(name, src).to_string();
                if info.seen_fns.contains(&nm) {
                    info.returns.remove(&nm);
                    info.tuples.remove(&nm);
                    info.dup_fns.push(nm.clone());
                } else {
                    info.seen_fns.push(nm.clone());
                }
                if !info.dup_fns.contains(&nm) {
                    if let Some(ty) = n.child_by_field_name("return_type").and_then(|t| annotation_type(text(t, src))) {
                        info.returns.insert(nm.clone(), ty);
                    }
                    if let Some(els) = n.child_by_field_name("return_type").and_then(|t| tuple_annotation(text(t, src))) {
                        info.tuples.insert(nm, els);
                    }
                }
            }
            let inner = Cx { scope: Some(info.scopes.len() - 1), class: cx.class };
            if let Some(ps) = n.child_by_field_name("parameters") {
                let mut c = ps.walk();
                for p in ps.named_children(&mut c) {
                    // an untyped parameter still shadows a module variable of the same name ("" = shadowed, no type)
                    let plain = match p.kind() {
                        "identifier" => Some(p),
                        "default_parameter" => p.child_by_field_name("name"),
                        _ => None,
                    };
                    if let Some(x) = plain {
                        bind(info, &inner, text(x, src), String::new());
                    }
                    if matches!(p.kind(), "typed_parameter" | "typed_default_parameter") {
                        let name = p.child_by_field_name("name").or_else(|| p.named_child(0));
                        let ty = p.child_by_field_name("type").and_then(|t| annotation_type(text(t, src)));
                        if let (Some(name), Some(ty)) = (name.filter(|x| x.kind() == "identifier"), ty) {
                            bind(info, &inner, text(name, src), ty);
                        }
                    }
                }
            }
            let mut c = n.walk();
            for ch in n.named_children(&mut c) {
                walk(ch, src, info, &inner);
            }
            return;
        }
        "assignment" => {
            if let Some(left) = n.child_by_field_name("left") {
                let ty = n
                    .child_by_field_name("type")
                    .and_then(|t| annotation_type(text(t, src)))
                    .or_else(|| n.child_by_field_name("right").and_then(|v| constructed_type(v, src)));
                if let (None, "identifier", Some(sc)) = (&ty, left.kind(), cx.scope) {
                    if !info.scopes[sc].globals.iter().any(|g| g == text(left, src)) {
                        info.scopes[sc].vars.entry(text(left, src).to_string()).or_default();
                    }
                }
                // `a, b = pair()`: element i of what `pair` is annotated to return (`()pair#i/n`, settled by the resolver)
                if matches!(left.kind(), "pattern_list" | "tuple_pattern") {
                    let mut c = left.walk();
                    let names: Vec<Node> = left.named_children(&mut c).collect();
                    let func = ty.as_deref().and_then(|t| t.strip_prefix("()")).filter(|f| !f.contains('#'));
                    if let (Some(func), true) = (func, names.iter().all(|x| x.kind() == "identifier")) {
                        for (i, x) in names.iter().enumerate() {
                            bind(info, cx, text(*x, src), format!("(){func}#{i}/{}", names.len()));
                        }
                    }
                }
                if let Some(ty) = ty {
                    match left.kind() {
                        "identifier" => bind(info, cx, text(left, src), ty),
                        "attribute" => {
                            let obj = left.child_by_field_name("object").map(|o| text(o, src));
                            if let (Some("self" | "cls"), Some(a), Some(c)) = (obj, left.child_by_field_name("attribute"), cx.class) {
                                info.classes[c].attrs.insert(text(a, src).to_string(), ty);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        "as_pattern" => {
            // `with Foo(...) as y`
            let target = n.child_by_field_name("alias").and_then(|a| a.named_child(0));
            if let (Some(v), Some(t)) = (n.named_child(0), target.filter(|t| t.kind() == "identifier")) {
                if let Some(ty) = constructed_type(v, src) {
                    bind(info, cx, text(t, src), ty);
                }
            }
        }
        _ => {}
    }
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        walk(ch, src, info, cx);
    }
}

pub fn analyze(src: &str) -> PyInfo {
    let mut info = PyInfo::default();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&tree_sitter_python::LANGUAGE.into()).is_err() {
        return info;
    }
    let Some(tree) = parser.parse(src, None) else { return info };
    walk(tree.root_node(), src, &mut info, &Cx { scope: None, class: None });
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imports_aliases_and_declared_types() {
        let i = analyze(
            "import a.b as c\nimport x.y\nfrom .m import (f as g, h)\n_db: Database | None = None\nclass K:\n    attr: Foo\n    def m(self, p: Bar, q: Baz = None, r=3):\n        self.s = Qux(1)\n        self.t: Optional[Zed] = None\n        with Foo(1) as w, open(x) as u:\n            pass\n        y = pkg.Thing()\n        z = other()\n",
        );
        let imp = |a: &str, m: &str, n: Option<&str>| PyImport { alias: a.into(), module: m.into(), name: n.map(Into::into) };
        for want in [imp("c", "a.b", None), imp("x", "x", None), imp("y", "x.y", None), imp("g", ".m", Some("f")), imp("h", ".m", Some("h"))] {
            assert!(i.imports.contains(&want), "{:?}", i.imports);
        }
        assert_eq!(i.module_vars["_db"], "Database");
        let g = analyze("_db: Database | None = None\ndef configure():\n    global _db\n    _db = None\n    _db.close()\n");
        assert_eq!(g.module_vars["_db"], "Database");
        assert!(!g.scopes[0].vars.contains_key("_db"));
        assert!(class_like("_ProgressLine") && !class_like("_x") && !class_like("make"));
        assert_eq!(i.classes[0].attrs["attr"], "Foo");
        assert_eq!((i.classes[0].attrs["s"].as_str(), i.classes[0].attrs["t"].as_str()), ("Qux", "Zed"));
        let v = &i.scopes[0].vars;
        assert_eq!((v["p"].as_str(), v["q"].as_str(), v["w"].as_str(), v["y"].as_str()), ("Bar", "Baz", "Foo", "Thing"));
        assert!(v["u"] == "()open" && v["z"] == "()other" && v["r"].is_empty());
    }
    #[test]
    fn annotation_forms() {
        for (t, want) in [("Foo", Some("Foo")), ("Optional[Foo]", Some("Foo")), ("Foo | None", Some("Foo")), ("\"a.Foo\"", Some("Foo")), ("int", None), ("A | B", None), ("list[Foo]", None)] {
            assert_eq!(annotation_type(t).as_deref(), want, "{t}");
        }
    }
}
