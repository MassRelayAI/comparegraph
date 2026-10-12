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

/// `it`, `test`, `it.only`, `test.skip` → true for a case; `beforeEach` etc. → true for a hook.
fn runner_kind(call: Node, src: &str) -> bool {
    let Some(f) = call.child_by_field_name("function") else { return false };
    let base = match f.kind() {
        "identifier" => text(f, src),
        "member_expression" => match f.child_by_field_name("object") {
            Some(o) if o.kind() == "identifier" => text(o, src),
            _ => return false,
        },
        _ => return false,
    };
    CASES.contains(&base) || HOOKS.contains(&base)
}

fn type_of_annotation(n: Node, src: &str) -> Option<String> {
    let t = text(n, src).trim_start_matches(':').trim();
    let name = t.split('|').map(str::trim).find(|p| *p != "undefined" && *p != "null")?;
    let name = name.split('<').next()?.rsplit('.').next()?.trim();
    (name.chars().next().is_some_and(char::is_uppercase) && name.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| name.to_string())
}

fn new_type(v: Node, src: &str) -> Option<String> {
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

fn collect(n: Node, src: &str, vars: &HashMap<String, String>, in_case: bool, out: &mut Vec<CallSite>) {
    match n.kind() {
        // a named function is the vendored extractor's business
        "function_declaration" | "method_definition" | "generator_function_declaration" => return,
        "call_expression" => {
            let line = n.start_position().row as u32 + 1;
            if let Some(f) = n.child_by_field_name("function") {
                if in_case {
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
                        out.push(CallSite { caller: CALLER_FILE.into(), callee, receiver, line, kind: "calls".into() });
                    }
                }
                if runner_kind(n, src) {
                    // the callback arguments are test bodies
                    if let Some(args) = n.child_by_field_name("arguments") {
                        let mut c = args.walk();
                        for a in args.named_children(&mut c) {
                            if is_callback(a) {
                                collect(a, src, vars, true, out);
                            } else {
                                collect(a, src, vars, in_case, out);
                            }
                        }
                    }
                    collect(f, src, vars, in_case, out);
                    return;
                }
            }
        }
        "new_expression" if in_case => {
            if let Some(c) = n.child_by_field_name("constructor") {
                let name = text(c, src).rsplit('.').next().unwrap_or("");
                if name.chars().next().is_some_and(char::is_uppercase) {
                    out.push(CallSite {
                        caller: CALLER_FILE.into(),
                        callee: name.to_string(),
                        receiver: None,
                        line: n.start_position().row as u32 + 1,
                        kind: "instantiates".into(),
                    });
                }
            }
        }
        _ => {}
    }
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        collect(ch, src, vars, in_case, out);
    }
}

/// Call sites inside the `it`/`test`/hook callbacks of a test file.
pub fn test_case_calls(src: &str, language: &str) -> Vec<CallSite> {
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
    collect(root, src, &vars, false, &mut out);
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
        assert!(c.iter().all(|s| s.caller == CALLER_FILE));
        assert!(!got.iter().any(|(n, _)| n == "helper" || n == "inner" || n == "expect" || n == "toBe"), "{got:?}");
    }
}
