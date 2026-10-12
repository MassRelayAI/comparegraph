//! JavaScript and TypeScript DECLARED types in our own layer (Plan 5 A2): the class named by a function's `: T` return annotation,
//! the class of a field (`private reader: Reader;`, `reader = new Reader()`, `this.reader = new Reader()`, a constructor parameter
//! property `constructor(private reader: Reader)`), and the local `const x = f()` that calls a function by bare name.
//! A type comes only from a declaration; a function without a return annotation, or a field without a type, types nothing.
use super::js_ctx::new_type;
use std::collections::HashMap;
use tree_sitter::Node;

#[derive(Debug, Default, PartialEq)]
pub struct ClassFields {
    pub start: u32,
    pub end: u32,
    pub fields: HashMap<String, String>,
}

#[derive(Debug, Default, PartialEq)]
pub struct TsDecls {
    /// function name → declared return class (a name declared twice with different types is dropped)
    pub returns: HashMap<String, String>,
    pub classes: Vec<ClassFields>,
    /// (line, variable, function) for `const x = f()` / `const x = await f()` with no annotation on `x`
    pub locals: Vec<(u32, String, String)>,
    /// (line, field, method) for every call `this.<field>.<method>(...)` (read from the syntax tree, not from text)
    pub member_calls: Vec<(u32, String, String)>,
    /// every function or variable name this file declares WITHOUT a typed return: it hides an imported function of the same name
    pub untyped: Vec<(u32, String, bool)>,
}

fn text<'a>(n: Node, src: &'a str) -> &'a str {
    &src[n.byte_range()]
}

/// `: Reader`, `: Reader<T>`, `: Reader | null` → `Reader`. A union of two classes, `ns.Reader`, `Reader[]` and `any` are none.
pub fn declared_class(t: &str) -> Option<String> {
    let t = t.trim().trim_start_matches(':').trim();
    let mut parts = t.split('|').map(str::trim).filter(|p| !matches!(*p, "undefined" | "null" | ""));
    let one = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let name = one.split('<').next()?.trim();
    (name.chars().next().is_some_and(char::is_uppercase) && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$')).then(|| name.to_string())
}

#[derive(Default)]
struct Acc {
    returns: HashMap<String, Option<String>>,
    classes: Vec<(u32, u32, HashMap<String, Option<String>>)>,
    locals: Vec<(u32, String, String)>,
    member_calls: Vec<(u32, String, String)>,
    untyped: Vec<(u32, String, bool)>,
}

fn put(m: &mut HashMap<String, Option<String>>, k: &str, v: String) {
    let e = m.entry(k.to_string()).or_insert_with(|| Some(v.clone()));
    if e.as_deref() != Some(v.as_str()) {
        *e = None;
    }
}

fn add_field(a: &mut Acc, cls: Option<usize>, name: &str, ty: Option<String>) {
    if let (Some(c), Some(t)) = (cls, ty) {
        put(&mut a.classes[c].2, name, t);
    }
}

fn walk(n: Node, src: &str, a: &mut Acc, cls: Option<usize>) {
    let line = n.start_position().row as u32 + 1;
    match n.kind() {
        "class_declaration" | "abstract_class_declaration" | "class" => {
            a.classes.push((line, n.end_position().row as u32 + 1, HashMap::new()));
            let inner = Some(a.classes.len() - 1);
            let mut c = n.walk();
            for ch in n.named_children(&mut c) {
                walk(ch, src, a, inner);
            }
            return;
        }
        "function_declaration" | "generator_function_declaration" => {
            if let (Some(name), Some(ty)) = (n.child_by_field_name("name"), n.child_by_field_name("return_type").and_then(|t| declared_class(text(t, src)))) {
                put(&mut a.returns, text(name, src), ty);
            } else if let Some(name) = n.child_by_field_name("name") {
                a.untyped.push((line, text(name, src).to_string(), true));
            }
        }
        "variable_declarator" => {
            if let Some(name) = n.child_by_field_name("name").filter(|x| x.kind() == "identifier") {
                // any declarator named like a function hides it; a typed function value is settled just below
                if !n.child_by_field_name("value").is_some_and(|v| matches!(v.kind(), "arrow_function" | "function_expression" | "function") && v.child_by_field_name("return_type").and_then(|t| declared_class(text(t, src))).is_some()) {
                    a.untyped.push((line, text(name, src).to_string(), false));
                }
            }
            if let (Some(name), Some(v)) = (n.child_by_field_name("name").filter(|x| x.kind() == "identifier"), n.child_by_field_name("value")) {
                let name = text(name, src);
                if matches!(v.kind(), "arrow_function" | "function_expression" | "function") {
                    if let Some(ty) = v.child_by_field_name("return_type").and_then(|t| declared_class(text(t, src))) {
                        put(&mut a.returns, name, ty);
                    }
                } else if n.child_by_field_name("type").is_none() {
                    let v = if v.kind() == "await_expression" { v.named_child(0).unwrap_or(v) } else { v };
                    if let Some(f) = (v.kind() == "call_expression").then(|| v.child_by_field_name("function")).flatten().filter(|f| f.kind() == "identifier") {
                        a.locals.push((line, name.to_string(), text(f, src).to_string()));
                    }
                }
            }
        }
        "public_field_definition" | "field_definition" => {
            if let Some(name) = n.child_by_field_name("name").or_else(|| n.child_by_field_name("property")).filter(|x| matches!(x.kind(), "property_identifier" | "private_property_identifier")) {
                let ty = n.child_by_field_name("type").map(|t| declared_class(text(t, src))).unwrap_or_else(|| n.child_by_field_name("value").and_then(|v| new_type(v, src)));
                add_field(a, cls, text(name, src).trim_start_matches('#'), ty);
            }
        }
        // `constructor(private reader: Reader)`: a parameter property is a field
        "required_parameter" | "optional_parameter" => {
            let mut c = n.walk();
            let is_prop = n.children(&mut c).any(|x| x.kind() == "accessibility_modifier" || text(x, src) == "readonly");
            if let (true, Some(p), Some(t)) = (is_prop, n.child_by_field_name("pattern").filter(|p| p.kind() == "identifier"), n.child_by_field_name("type")) {
                add_field(a, cls, text(p, src), declared_class(text(t, src)));
            }
        }
        "call_expression" => {
            let m = n.child_by_field_name("function").filter(|f| f.kind() == "member_expression");
            let obj = m.and_then(|m| m.child_by_field_name("object")).filter(|o| o.kind() == "member_expression");
            if let (Some(m), Some(obj)) = (m, obj) {
                if let (Some(this), Some(field), Some(method)) = (obj.child_by_field_name("object"), obj.child_by_field_name("property"), m.child_by_field_name("property")) {
                    if this.kind() == "this" {
                        a.member_calls.push((line, text(field, src).to_string(), text(method, src).to_string()));
                    }
                }
            }
        }
        // `this.reader = new Reader()` in any method of the class
        "assignment_expression" => {
            if let (Some(l), Some(r)) = (n.child_by_field_name("left").filter(|l| l.kind() == "member_expression"), n.child_by_field_name("right")) {
                if let (Some(o), Some(p)) = (l.child_by_field_name("object"), l.child_by_field_name("property")) {
                    if o.kind() == "this" {
                        add_field(a, cls, text(p, src), new_type(r, src));
                    }
                }
            }
        }
        _ => {}
    }
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        walk(ch, src, a, cls);
    }
}

pub fn analyze(src: &str, language: &str) -> Option<TsDecls> {
    let lang: tree_sitter::Language = match language {
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "javascript" | "jsx" | "mjs" => tree_sitter_javascript::LANGUAGE.into(),
        _ => return None,
    };
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).ok()?;
    let tree = parser.parse(src, None)?;
    let mut a = Acc::default();
    walk(tree.root_node(), src, &mut a, None);
    Some(TsDecls {
        returns: a.returns.into_iter().filter(|(k, _)| !a.untyped.iter().any(|u| &u.1 == k)).filter_map(|(k, v)| v.map(|t| (k, t))).collect(),
        classes: a
            .classes
            .into_iter()
            .map(|(start, end, f)| ClassFields { start, end, fields: f.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect() })
            .collect(),
        locals: a.locals,
        member_calls: a.member_calls,
        untyped: a.untyped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn declared_class_forms() {
        for (t, want) in [(": Reader", Some("Reader")), (": Reader | null", Some("Reader")), (": Reader<T>", Some("Reader")), (": A | B", None), (": ns.Reader", None), (": Reader[]", None), (": string", None)] {
            assert_eq!(declared_class(t).as_deref(), want, "{t}");
        }
    }
    #[test]
    fn declarations_only() {
        let d = analyze("function make(): Reader { return null as any; }\nfunction bare() { return new Reader(); }\nclass A {\n  private r: Reader;\n  o = new Other();\n  u;\n  constructor(private p: Pp, q: Qq) { this.z = new Zed(); }\n}\nfunction go() { const x = make(); const y: Reader = make(); }\n", "typescript").unwrap();
        assert_eq!(d.returns.get("make").map(String::as_str), Some("Reader"));
        assert!(!d.returns.contains_key("bare"));
        let f = &d.classes[0].fields;
        assert_eq!((f["r"].as_str(), f["o"].as_str(), f["p"].as_str(), f["z"].as_str()), ("Reader", "Other", "Pp", "Zed"));
        assert!(!f.contains_key("u") && !f.contains_key("q"));
        assert_eq!(d.locals.iter().map(|l| l.1.as_str()).collect::<Vec<_>>(), ["x"]);
    }
}
