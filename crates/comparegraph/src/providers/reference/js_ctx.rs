//! JavaScript and TypeScript test cases in our own layer. The vendored extractor records a call only inside a named function, so the
//! calls made in the body of `it('…', () => { … })`, `test(…)` and the `beforeEach(…)` family (arrow functions with no name) are lost.
//! This pass finds those bodies and returns their call sites, attributed to the file (`CALLER_FILE`); the resolver turns the edges
//! into `tests` edges from the test file. A variable declared with `new Foo(…)` or `: Foo` anywhere in the file names its type, so
//! `service.getUser()` reaches `Foo`'s method.
use super::vendor::{extractor::extract_receiver_name, CallSite};
use std::collections::HashMap;
use tree_sitter::Node;

/// The `caller` of a call site found here: the enclosing test file, not a symbol.
pub const CALLER_FILE: &str = "\u{0}test-file";

const CASES: &[&str] = &["it", "test"];
const HOOKS: &[&str] = &["beforeEach", "beforeAll", "afterEach", "afterAll", "before", "after"];
/// test-runner and assertion functions: never a call into the code under test
const RUNNER: &[&str] = &["it", "test", "describe", "expect", "beforeEach", "beforeAll", "afterEach", "afterAll", "before", "after", "suite", "context", "assert"];

fn text<'a>(n: Node, src: &'a str) -> &'a str {
    &src[n.byte_range()]
}

fn is_callback(n: Node) -> bool {
    matches!(n.kind(), "arrow_function" | "function_expression" | "function")
}

#[derive(PartialEq)]
enum Kind {
    Case,
    Hook,
    Describe,
    Other,
}

const CASE_MODIFIERS: &[&str] = &["skip", "only", "todo", "concurrent", "fails"];

/// `it`/`test` (optionally `.skip/.only/.todo/.concurrent/.fails`) is a case; `beforeEach` etc. and `test.beforeEach` a hook;
/// `describe`, `suite`, `context` and `test.describe` a group; anything else (`test.step`, `expect.…`) is none of them.
fn classify(call: Node, src: &str) -> Kind {
    let Some(f) = call.child_by_field_name("function") else { return Kind::Other };
    let (base, member) = match f.kind() {
        "identifier" => (text(f, src), None),
        "member_expression" => match (f.child_by_field_name("object"), f.child_by_field_name("property")) {
            (Some(o), Some(p)) if o.kind() == "identifier" => (text(o, src), Some(text(p, src))),
            _ => return Kind::Other,
        },
        _ => return Kind::Other,
    };
    match (base, member) {
        (b, None) if CASES.contains(&b) => Kind::Case,
        (b, Some(m)) if CASES.contains(&b) && CASE_MODIFIERS.contains(&m) => Kind::Case,
        (b, None) if HOOKS.contains(&b) => Kind::Hook,
        ("test", Some(m)) if HOOKS.contains(&m) => Kind::Hook,
        (b, _) if HOOKS.contains(&b) => Kind::Hook,
        ("describe" | "suite" | "context", None) => Kind::Describe,
        // `describe.each([…])('t', cb)` / `.for` are not blocks of ours: their body's calls stay unattributed (a known limit)
        ("describe" | "suite" | "context", Some(m)) if ["skip", "only", "todo", "serial", "parallel", "concurrent", "sequential"].contains(&m) => Kind::Describe,
        ("test", Some("describe")) => Kind::Describe,
        _ => Kind::Other,
    }
}

fn type_of_annotation(n: Node, src: &str) -> Option<String> {
    let t = text(n, src).trim_start_matches(':').trim();
    let name = t.split('|').map(str::trim).find(|p| *p != "undefined" && *p != "null")?;
    let name = name.split('<').next()?.rsplit('.').next()?.trim();
    (name.chars().next().is_some_and(char::is_uppercase) && name.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| name.to_string())
}

pub(super) fn new_type(v: Node, src: &str) -> Option<String> {
    if v.kind() != "new_expression" {
        return None;
    }
    let c = v.child_by_field_name("constructor")?;
    let last = text(c, src).rsplit('.').next()?;
    last.chars().next().is_some_and(char::is_uppercase).then(|| last.to_string())
}

/// variable → type, only when every declaration of that name in the file agrees.
fn var_types(root: Node, src: &str) -> HashMap<String, String> {
    let mut found: HashMap<String, Option<String>> = HashMap::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        let mut add = |name: &str, ty: Option<String>| {
            let e = found.entry(name.to_string()).or_insert_with(|| ty.clone());
            if *e != ty {
                *e = None;
            }
        };
        match n.kind() {
            "variable_declarator" => {
                if let Some(name) = n.child_by_field_name("name").filter(|x| x.kind() == "identifier") {
                    let ty = n
                        .child_by_field_name("type")
                        .and_then(|t| type_of_annotation(t, src))
                        .or_else(|| n.child_by_field_name("value").and_then(|v| new_type(v, src)));
                    add(text(name, src), ty);
                }
            }
            "assignment_expression" => {
                if let (Some(l), Some(r)) = (n.child_by_field_name("left").filter(|x| x.kind() == "identifier"), n.child_by_field_name("right")) {
                    add(text(l, src), new_type(r, src));
                }
            }
            _ => {}
        }
        let mut c = n.walk();
        stack.extend(n.named_children(&mut c));
    }
    found.into_iter().filter_map(|(k, v)| v.map(|t| (k, t))).collect()
}

/// A test case found in a file: `it("…")` / `test("…")` (describe titles prefix it), spanning the call.
#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub name: String,
    pub line_start: u32,
    pub line_end: u32,
}

/// The `caller` of a call site made inside case number `i` (see `test_cases`).
pub const CASE_PREFIX: &str = "\u{0}case:";

/// The caller key of a call made in a case: the case's symbol name and the line its span starts on.
pub fn case_key(c: &Case) -> String {
    format!("{CASE_PREFIX}{}:{}", c.line_start, c.name)
}

/// Split a caller key back into `(line_start, symbol name)`.
pub fn parse_case_key(caller: &str) -> Option<(u32, &str)> {
    let (l, n) = caller.strip_prefix(CASE_PREFIX)?.split_once(':')?;
    Some((l.parse().ok()?, n))
}

#[derive(Clone, Copy, PartialEq)]
enum Who {
    Nobody,
    File,
    Case(usize),
}

const TITLE_MAX: usize = 80;

/// The first argument as a plain string (quotes, or a template without substitutions).
fn title_arg(call: Node, src: &str) -> Option<String> {
    let args = call.child_by_field_name("arguments")?;
    let a = args.named_child(0)?;
    let raw = match a.kind() {
        "string" => text(a, src),
        "template_string" if !text(a, src).contains("${") => text(a, src),
        _ => return None,
    };
    let t = raw.trim_matches(|c| c == '"' || c == '\'' || c == '`');
    Some(t.split_whitespace().collect::<Vec<_>>().join(" "))
}

struct Acc {
    calls: Vec<CallSite>,
    cases: Vec<Case>,
    titles: Vec<String>,
}

fn collect(n: Node, src: &str, vars: &HashMap<String, String>, who: Who, acc: &mut Acc) {
    match n.kind() {
        // a named function is the vendored extractor's business
        "function_declaration" | "method_definition" | "generator_function_declaration" => return,
        "call_expression" => {
            let line = n.start_position().row as u32 + 1;
            if let Some(f) = n.child_by_field_name("function") {
                if who != Who::Nobody {
                    let caller = match who {
                        Who::Case(i) => case_key(&acc.cases[i]),
                        _ => CALLER_FILE.to_string(),
                    };
                    let (callee, receiver) = if f.kind() == "member_expression" {
                        let m = f.child_by_field_name("property").map(|p| text(p, src).to_string());
                        let r = f.child_by_field_name("object").map(|o| extract_receiver_name(o, src));
                        (m, r)
                    } else {
                        (Some(text(f, src).to_string()), None)
                    };
                    let on_runner = receiver.as_deref().is_some_and(|r| RUNNER.contains(&r));
                    if let Some(callee) = callee.filter(|c| !on_runner && !RUNNER.contains(&c.as_str()) && c.chars().all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '$')) {
                        // a receiver variable of known type is replaced by its type, which the resolver reads as `Type.method()`
                        let receiver = receiver.map(|r| vars.get(&r).cloned().unwrap_or(r));
                        acc.calls.push(CallSite { caller, callee, receiver, line, kind: "calls".into() });
                    }
                }
                let kind = classify(n, src);
                if kind == Kind::Case || kind == Kind::Hook {
                    let is_case = kind == Kind::Case;
                    // the callback arguments are test bodies
                    let inner = if is_case {
                        let own = title_arg(n, src).unwrap_or_else(|| "case".into());
                        let mut full = acc.titles.iter().cloned().chain([own]).collect::<Vec<_>>().join(" > ");
                        if full.chars().count() > TITLE_MAX {
                            full = full.chars().take(TITLE_MAX).collect::<String>().trim_end().to_string(); // by chars: UTF-8 safe
                        }
                        acc.cases.push(Case { name: format!("it: {full}"), line_start: line, line_end: n.end_position().row as u32 + 1 });
                        Who::Case(acc.cases.len() - 1)
                    } else if matches!(who, Who::Case(_)) {
                        who // a hook in a describe body belongs to that describe
                    } else {
                        Who::File
                    };
                    if let Some(args) = n.child_by_field_name("arguments") {
                        let mut c = args.walk();
                        for a in args.named_children(&mut c) {
                            if is_callback(a) {
                                collect(a, src, vars, inner, acc);
                            } else {
                                collect(a, src, vars, who, acc);
                            }
                        }
                    }
                    collect(f, src, vars, who, acc);
                    return;
                }
                // `describe("title", () => { … })` is a node of its own, prefixes the titles of what is inside it, and owns the calls
                // made in its body outside any case
                if kind == Kind::Describe {
                    acc.titles.push(title_arg(n, src).unwrap_or_default());
                    let mut full = acc.titles.join(" > ");
                    if full.chars().count() > TITLE_MAX {
                        full = full.chars().take(TITLE_MAX).collect::<String>().trim_end().to_string(); // by chars: UTF-8 safe
                    }
                    if full.trim_matches(|c: char| c == '>' || c.is_whitespace()).is_empty() {
                        full = "describe".into();
                    }
                    acc.cases.push(Case { name: format!("describe: {full}"), line_start: line, line_end: n.end_position().row as u32 + 1 });
                    let me = Who::Case(acc.cases.len() - 1);
                    let mut c = n.walk();
                    for ch in n.named_children(&mut c) {
                        collect(ch, src, vars, me, acc);
                    }
                    acc.titles.pop();
                    return;
                }
            }
        }
        "new_expression" if who != Who::Nobody => {
            if let Some(c) = n.child_by_field_name("constructor") {
                let name = text(c, src).rsplit('.').next().unwrap_or("");
                if name.chars().next().is_some_and(char::is_uppercase) {
                    let caller = match who {
                        Who::Case(i) => case_key(&acc.cases[i]),
                        _ => CALLER_FILE.to_string(),
                    };
                    acc.calls.push(CallSite { caller, callee: name.to_string(), receiver: None, line: n.start_position().row as u32 + 1, kind: "instantiates".into() });
                }
            }
        }
        _ => {}
    }
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        collect(ch, src, vars, who, acc);
    }
}

fn parse_js(src: &str, language: &str) -> Option<tree_sitter::Tree> {
    let lang: tree_sitter::Language = match language {
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "javascript" | "jsx" | "mjs" => tree_sitter_javascript::LANGUAGE.into(),
        _ => return None,
    };
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).ok()?;
    parser.parse(src, None)
}

/// The `it`/`test` cases of a test file, and the call sites in their bodies (caller `CASE_PREFIX + index`) and in hook bodies
/// (caller `CALLER_FILE`). Case names are `it: <describe > … > title>`, at most 80 characters of title.
pub fn test_cases(src: &str, language: &str) -> (Vec<Case>, Vec<CallSite>) {
    let Some(tree) = parse_js(src, language) else { return (vec![], vec![]) };
    let root = tree.root_node();
    let vars = var_types(root, src);
    let mut acc = Acc { calls: vec![], cases: vec![], titles: vec![] };
    collect(root, src, &vars, Who::Nobody, &mut acc);
    (acc.cases, acc.calls)
}

/// Call sites inside the `it`/`test`/hook callbacks of a test file.
pub fn test_case_calls(src: &str, language: &str) -> Vec<CallSite> {
    test_cases(src, language).1
}

/// JSX elements naming a component: `<Foo />` and `<Foo>` (an upper-case identifier; `<div>` and `<A.B>` are not), with the line.
/// A JSX element is a rendering call; the resolver binds it only to an imported or same-file component.
pub fn jsx_elements(src: &str, language: &str) -> Vec<(String, u32)> {
    if !matches!(language, "tsx" | "jsx" | "javascript" | "mjs") {
        return vec![];
    }
    let Some(tree) = parse_js(src, language) else { return vec![] };
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        if matches!(n.kind(), "jsx_self_closing_element" | "jsx_opening_element") {
            if let Some(name) = n.child_by_field_name("name").filter(|x| x.kind() == "identifier") {
                let t = text(name, src);
                if t.chars().next().is_some_and(char::is_uppercase) {
                    out.push((t.to_string(), n.start_position().row as u32 + 1));
                }
            }
        }
        let mut c = n.walk();
        stack.extend(n.named_children(&mut c));
    }
    out.sort_by(|a, b| (a.1, &a.0).cmp(&(b.1, &b.0)));
    out
}

/// Class fields bound to a function (`request = (…) => { … }`, `fetch: T = function () {}`): methods the vendored extractor does not
/// see. Returns (name, first line, last line), 1-based, in source order.
pub fn class_field_functions(src: &str, language: &str) -> Vec<(String, u32, u32)> {
    let Some(tree) = parse_js(src, language) else { return vec![] };
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        if matches!(n.kind(), "public_field_definition" | "field_definition") && n.parent().is_some_and(|p| p.kind() == "class_body") {
            let name = n.child_by_field_name("name").or_else(|| n.child_by_field_name("property")).filter(|x| x.kind() == "property_identifier"); // a `#private` field is never a method others can bind
            let value = n.child_by_field_name("value").filter(|v| is_callback(*v));
            if let (Some(name), Some(_)) = (name, value) {
                out.push((text(name, src).to_string(), n.start_position().row as u32 + 1, n.end_position().row as u32 + 1));
            }
        }
        let mut c = n.walk();
        stack.extend(n.named_children(&mut c));
    }
    out.sort_by_key(|o| (o.1, o.0.clone()));
    out
}

fn pattern_names(n: Node, src: &str, out: &mut Vec<String>) {
    match n.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => out.push(text(n, src).to_string()),
        "type_annotation" => {}
        "pair_pattern" => {
            if let Some(v) = n.child_by_field_name("value") {
                pattern_names(v, src, out);
            }
        }
        "assignment_pattern" | "object_assignment_pattern" => {
            if let Some(l) = n.child_by_field_name("left") {
                pattern_names(l, src, out);
            }
        }
        _ => {
            let mut c = n.walk();
            for ch in n.named_children(&mut c) {
                pattern_names(ch, src, out);
            }
        }
    }
}

/// Names bound inside a function, block or catch clause (parameters, `const`/`let`/`var`, destructuring, catch bindings) with the
/// first and last line of the scope they hide an outer name in. File-level bindings are left out (an import cannot share one).
pub fn local_bindings(src: &str, language: &str) -> Vec<(String, u32, u32)> {
    let Some(tree) = parse_js(src, language) else { return vec![] };
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        let mut names = Vec::new();
        let mut scope = None;
        match n.kind() {
            k if FUNCTION_LIKE.contains(&k) => {
                scope = Some(n);
                if let Some(p) = n.child_by_field_name("parameters") {
                    let mut c = p.walk();
                    for prm in p.named_children(&mut c) {
                        match prm.child_by_field_name("pattern") {
                            Some(pt) => pattern_names(pt, src, &mut names),
                            None => pattern_names(prm, src, &mut names),
                        }
                    }
                } else if let Some(p) = n.child_by_field_name("parameter") {
                    pattern_names(p, src, &mut names);
                }
            }
            "catch_clause" => {
                scope = Some(n);
                if let Some(p) = n.child_by_field_name("parameter") {
                    pattern_names(p, src, &mut names);
                }
            }
            "variable_declarator" => {
                // `var` is hoisted to the enclosing function (or the module: no scope); `let`/`const` stay in the nearest block or loop
                let hoisted = n.parent().is_some_and(|d| d.kind() == "variable_declaration");
                let mut up = n.parent();
                while let Some(u) = up {
                    let block = matches!(u.kind(), "statement_block" | "for_statement" | "for_in_statement" | "class_body");
                    if (block && !hoisted) || FUNCTION_LIKE.contains(&u.kind()) {
                        scope = Some(u);
                        break;
                    }
                    if u.kind() == "program" {
                        break;
                    }
                    up = u.parent();
                }
                if let Some(nm) = n.child_by_field_name("name") {
                    pattern_names(nm, src, &mut names);
                }
            }
            // `for (const x of xs)` / `for (let k in o)` bind `left` for the whole loop, a braceless body included (a bare
            // `for (x of xs)` assigns: it has no declaration keyword); `var` is hoisted to the function
            "for_in_statement" if n.child_by_field_name("kind").is_some() => {
                let var = n.child_by_field_name("kind").is_some_and(|k| text(k, src) == "var");
                if var {
                    let mut up = n.parent();
                    while let Some(u) = up {
                        if FUNCTION_LIKE.contains(&u.kind()) {
                            scope = Some(u);
                            break;
                        }
                        up = u.parent();
                    }
                } else {
                    scope = Some(n);
                }
                if let Some(l) = n.child_by_field_name("left") {
                    pattern_names(l, src, &mut names);
                }
            }
            _ => {}
        }
        if let Some(sc) = scope {
            for nm in names {
                out.push((nm, sc.start_position().row as u32 + 1, sc.end_position().row as u32 + 1));
            }
        }
        let mut c = n.walk();
        stack.extend(n.named_children(&mut c));
    }
    out
}

fn has_type_keyword(n: Node) -> bool {
    let mut c = n.walk();
    let r = n.children(&mut c).any(|ch| ch.kind() == "type");
    r
}

/// Local names of imports that are type-only (`import type { A }`, `import { type A as B }`).
pub fn type_only_imports(src: &str, language: &str) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    let Some(tree) = parse_js(src, language) else { return out };
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        if n.kind() == "import_statement" {
            let whole = has_type_keyword(n);
            let mut s2 = vec![n];
            while let Some(m) = s2.pop() {
                if m.kind() == "import_specifier" {
                    let own = has_type_keyword(m);
                    if whole || own {
                        if let Some(nm) = m.child_by_field_name("alias").or_else(|| m.child_by_field_name("name")) {
                            out.insert(text(nm, src).to_string());
                        }
                    }
                } else if m.kind() == "identifier" && whole && m.parent().is_some_and(|p| p.kind() == "import_clause") {
                    out.insert(text(m, src).to_string());
                }
                let mut c = m.walk();
                s2.extend(m.named_children(&mut c));
            }
            continue;
        }
        let mut c = n.walk();
        stack.extend(n.named_children(&mut c));
    }
    out
}

const FUNCTION_LIKE: &[&str] = &["function_declaration", "generator_function_declaration", "method_definition", "function_expression", "function", "generator_function", "arrow_function"];

fn module_collect(n: Node, src: &str, vars: &HashMap<String, String>, out: &mut Vec<CallSite>) {
    let line = n.start_position().row as u32 + 1;
    match n.kind() {
        // a function body belongs to the function (or to nobody, when it is an anonymous callback)
        k if FUNCTION_LIKE.contains(&k) => return,
        "import_statement" | "type_alias_declaration" | "interface_declaration" => return,
        "call_expression" => {
            if let Some(f) = n.child_by_field_name("function") {
                let (callee, receiver) = match f.kind() {
                    "member_expression" => (
                        f.child_by_field_name("property").map(|p| text(p, src).to_string()),
                        f.child_by_field_name("object").map(|o| extract_receiver_name(o, src)),
                    ),
                    "identifier" => (Some(text(f, src).to_string()), None),
                    _ => (None, None),
                };
                let on_runner = receiver.as_deref().is_some_and(|r| RUNNER.contains(&r));
                if let Some(callee) = callee.filter(|c| !on_runner && !RUNNER.contains(&c.as_str()) && c.chars().all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '$')) {
                    let receiver = receiver.map(|r| vars.get(&r).cloned().unwrap_or(r));
                    out.push(CallSite { caller: CALLER_FILE.into(), callee, receiver, line, kind: "calls".into() });
                }
            }
        }
        "new_expression" => {
            if let Some(c) = n.child_by_field_name("constructor") {
                let name = text(c, src).rsplit('.').next().unwrap_or("");
                if name.chars().next().is_some_and(char::is_uppercase) && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '$') {
                    out.push(CallSite { caller: CALLER_FILE.into(), callee: name.to_string(), receiver: None, line, kind: "instantiates".into() });
                }
            }
        }
        // module tables: `{ data: onData }`, `{ onData }`, `[a, b]` name functions by a bare identifier
        "object" => {
            let mut c = n.walk();
            for p in n.named_children(&mut c) {
                let v = match p.kind() {
                    "pair" => p.child_by_field_name("value").filter(|v| v.kind() == "identifier"),
                    "shorthand_property_identifier" => Some(p),
                    _ => None,
                };
                if let Some(v) = v {
                    out.push(CallSite { caller: CALLER_FILE.into(), callee: text(v, src).to_string(), receiver: None, line: v.start_position().row as u32 + 1, kind: "refs".into() });
                }
            }
        }
        "array" => {
            let mut c = n.walk();
            for v in n.named_children(&mut c).filter(|v| v.kind() == "identifier") {
                out.push(CallSite { caller: CALLER_FILE.into(), callee: text(v, src).to_string(), receiver: None, line: v.start_position().row as u32 + 1, kind: "refs".into() });
            }
        }
        _ => {}
    }
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        module_collect(ch, src, vars, out);
    }
}

/// Call sites and table references outside any function body (see `module_ctx`), attributed to the file.
pub fn module_calls(src: &str, language: &str) -> Vec<CallSite> {
    let lang: tree_sitter::Language = match language {
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "javascript" | "jsx" | "mjs" => tree_sitter_javascript::LANGUAGE.into(),
        _ => return vec![],
    };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return vec![];
    }
    let Some(tree) = parser.parse(src, None) else { return vec![] };
    let root = tree.root_node();
    let vars = var_types(root, src);
    let mut out = Vec::new();
    module_collect(root, src, &vars, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calls_in_it_bodies_are_found_with_typed_receivers() {
        let src = "import { parse } from './p';\ndescribe('x', () => {\n  let svc: UserService;\n  beforeEach(() => { svc = new UserService(); });\n  it('a', () => {\n    const v = parse(1);\n    svc.getUser(2);\n    expect(v).toBe(1);\n    other.skip();\n  });\n  helper();\n});\nfunction named() { inner(); }\n";
        let c = test_case_calls(src, "typescript");
        let got: Vec<(String, Option<String>)> = c.iter().map(|s| (s.callee.clone(), s.receiver.clone())).collect();
        for want in [("parse", None), ("getUser", Some("UserService")), ("UserService", None), ("skip", Some("other"))] {
            assert!(got.contains(&(want.0.to_string(), want.1.map(String::from))), "{got:?}");
        }
        assert!(c.iter().all(|s| s.caller == CALLER_FILE || s.caller.starts_with(CASE_PREFIX)));
        // `helper()` in the describe body belongs to the describe (Plan 6 A1)
        assert!(c.iter().any(|s| s.callee == "helper" && s.caller.contains("describe: x")), "{c:?}");
        assert!(!got.iter().any(|(n, _)| n == "inner" || n == "expect" || n == "toBe"), "{got:?}");
    }
}
