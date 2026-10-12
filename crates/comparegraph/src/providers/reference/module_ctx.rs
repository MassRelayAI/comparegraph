//! Calls made outside any function body, attributed to the file (`js_ctx::CALLER_FILE`): a Python `if __name__ == "__main__": main()`,
//! a decorator, `app.use(router())` at the top of a TypeScript file, a Go package-level `var x = load()`, a Rust `static`/`const`
//! (and `lazy_static!`) initialiser, a Java static initialiser or field initialiser. The vendored extractor records a call only
//! inside a named function, so these are lost without this pass.
//!
//! A module table (`HANDLERS = {"x": handle_x}`, `{ data: onData }`, `[a, b]`) names a function by a bare identifier. That is a
//! reference, not a call: it is returned with kind `refs` and the resolver binds it only to an imported or same-file function.
use super::js_ctx::{self, CALLER_FILE};
use super::vendor::{extractor::extract_receiver_name, CallSite};
use tree_sitter::Node;

fn text<'a>(n: Node, src: &'a str) -> &'a str {
    &src[n.byte_range()]
}

fn push(out: &mut Vec<CallSite>, n: Node, callee: &str, receiver: Option<String>, kind: &str, line_offset: u32) {
    out.push(CallSite {
        caller: CALLER_FILE.into(),
        callee: callee.to_string(),
        receiver,
        line: n.start_position().row as u32 + 1 + line_offset,
        kind: kind.into(),
    });
}

fn parse(src: &str, lang: tree_sitter::Language) -> Option<tree_sitter::Tree> {
    let mut p = tree_sitter::Parser::new();
    p.set_language(&lang).ok()?;
    p.parse(src, None)
}

/// Module-level call sites and references of one file.
pub fn module_calls(src: &str, language: &str) -> Vec<CallSite> {
    let mut out = Vec::new();
    match language {
        "typescript" | "tsx" | "javascript" | "jsx" | "mjs" => return js_ctx::module_calls(src, language),
        "python" => {
            if let Some(t) = parse(src, tree_sitter_python::LANGUAGE.into()) {
                python(t.root_node(), src, &mut out);
            }
        }
        "go" => {
            if let Some(t) = parse(src, tree_sitter_go::LANGUAGE.into()) {
                go(t.root_node(), src, &mut out);
            }
        }
        "rust" => {
            if let Some(t) = parse(src, tree_sitter_rust::LANGUAGE.into()) {
                rust(t.root_node(), src, false, 0, &mut out);
            }
        }
        "java" => {
            if let Some(t) = parse(src, tree_sitter_java::LANGUAGE.into()) {
                java(t.root_node(), src, &mut out);
            }
        }
        _ => {}
    }
    out
}

fn children(n: Node) -> Vec<Node> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}

// ---- Python ----

fn py_call(f: Node, at: Node, src: &str, out: &mut Vec<CallSite>) {
    match f.kind() {
        "identifier" => push(out, at, text(f, src), None, "calls", 0),
        "attribute" => {
            if let (Some(a), Some(o)) = (f.child_by_field_name("attribute"), f.child_by_field_name("object")) {
                push(out, at, text(a, src), Some(extract_receiver_name(o, src)), "calls", 0);
            }
        }
        _ => {}
    }
}

fn python(n: Node, src: &str, out: &mut Vec<CallSite>) {
    match n.kind() {
        // a function body is the vendored extractor's business; a lambda is no named caller
        "function_definition" | "lambda" | "import_statement" | "import_from_statement" => return,
        "decorator" => {
            if let Some(e) = n.named_child(0) {
                match e.kind() {
                    "call" => {
                        if let Some(f) = e.child_by_field_name("function") {
                            py_call(f, e, src, out);
                        }
                    }
                    _ => py_call(e, e, src, out),
                }
            }
        }
        "call" => {
            if let Some(f) = n.child_by_field_name("function") {
                py_call(f, n, src, out);
            }
        }
        "dictionary" => {
            for p in children(n) {
                if let Some(v) = p.child_by_field_name("value").filter(|v| v.kind() == "identifier") {
                    push(out, v, text(v, src), None, "refs", 0);
                }
            }
        }
        "list" | "tuple" | "set" => {
            for v in children(n).into_iter().filter(|v| v.kind() == "identifier") {
                push(out, v, text(v, src), None, "refs", 0);
            }
        }
        "assignment" => {
            if let Some(v) = n.child_by_field_name("right").filter(|v| v.kind() == "identifier") {
                push(out, v, text(v, src), None, "refs", 0);
            }
        }
        _ => {}
    }
    for ch in children(n) {
        python(ch, src, out);
    }
}

// ---- Go ----

fn go(n: Node, src: &str, out: &mut Vec<CallSite>) {
    match n.kind() {
        "function_declaration" | "method_declaration" | "func_literal" | "import_declaration" => return,
        "call_expression" => {
            if let Some(f) = n.child_by_field_name("function") {
                match f.kind() {
                    "identifier" => push(out, n, text(f, src), None, "calls", 0),
                    "selector_expression" => {
                        if let (Some(fl), Some(o)) = (f.child_by_field_name("field"), f.child_by_field_name("operand")) {
                            push(out, n, text(fl, src), Some(extract_receiver_name(o, src)), "calls", 0);
                        }
                    }
                    _ => {}
                }
            }
        }
        // `map[string]func(){"a": handleA}` / `[]func(){a, b}`: the element (the last one of a keyed pair) is a reference
        "literal_element" => {
            let last = n.parent().is_none_or(|p| p.kind() != "keyed_element" || p.named_child(p.named_child_count().saturating_sub(1)) == Some(n));
            if last {
                if let Some(v) = n.named_child(0).filter(|v| v.kind() == "identifier" && n.named_child_count() == 1) {
                    push(out, v, text(v, src), None, "refs", 0);
                }
            }
        }
        _ => {}
    }
    for ch in children(n) {
        go(ch, src, out);
    }
}

// ---- Rust ----

/// `in_val`: inside the value of a `static` / `const` item, where a call runs at load time.
fn rust(n: Node, src: &str, in_val: bool, line_offset: u32, out: &mut Vec<CallSite>) {
    match n.kind() {
        "function_item" | "closure_expression" | "use_declaration" | "attribute_item" => return,
        "static_item" | "const_item" => {
            for ch in children(n) {
                rust(ch, src, true, line_offset, out);
            }
            return;
        }
        "macro_invocation" => {
            // lazy_static! { static ref X: T = init(); } — not parsed as code by the grammar: parse the body as items
            if n.child_by_field_name("macro").is_some_and(|m| text(m, src).rsplit("::").next() == Some("lazy_static")) {
                if let Some(tt) = children(n).into_iter().find(|c| c.kind() == "token_tree") {
                    let body = text(tt, src);
                    let inner = body.strip_prefix('{').and_then(|b| b.strip_suffix('}')).unwrap_or(body).replace("static ref ", "static ");
                    // the stripped `{` is on the token tree's own first line; the replacement keeps line count
                    if let Some(t) = parse(&inner, tree_sitter_rust::LANGUAGE.into()) {
                        rust(t.root_node(), &inner, false, line_offset + tt.start_position().row as u32, out);
                    }
                }
            }
            return;
        }
        "call_expression" if in_val => {
            if let Some(f) = n.child_by_field_name("function") {
                match f.kind() {
                    "identifier" => push(out, n, text(f, src), None, "calls", line_offset),
                    "scoped_identifier" => {
                        if let Some(name) = f.child_by_field_name("name") {
                            let recv = f.child_by_field_name("path").map(|p| text(p, src).to_string());
                            push(out, n, text(name, src), recv, "calls", line_offset);
                        }
                    }
                    "field_expression" => {
                        if let (Some(fl), Some(v)) = (f.child_by_field_name("field"), f.child_by_field_name("value")) {
                            push(out, n, text(fl, src), Some(extract_receiver_name(v, src)), "calls", line_offset);
                        }
                    }
                    _ => {}
                }
            }
        }
        "array_expression" | "tuple_expression" if in_val => {
            for v in children(n).into_iter().filter(|v| v.kind() == "identifier") {
                push(out, v, text(v, src), None, "refs", line_offset);
            }
        }
        _ => {}
    }
    for ch in children(n) {
        rust(ch, src, in_val, line_offset, out);
    }
}

// ---- Java ----

fn java(n: Node, src: &str, out: &mut Vec<CallSite>) {
    match n.kind() {
        "method_declaration" | "constructor_declaration" | "lambda_expression" | "import_declaration" | "package_declaration" => return,
        "method_invocation" => {
            if let Some(name) = n.child_by_field_name("name") {
                let recv = n.child_by_field_name("object").map(|o| extract_receiver_name(o, src));
                push(out, n, text(name, src), recv, "calls", 0);
            }
        }
        _ => {}
    }
    for ch in children(n) {
        java(ch, src, out);
    }
}

/// `if __name__ == "__main__":` at the top level of a Python file.
pub fn has_main_guard(lines: &[String]) -> bool {
    lines.iter().any(|l| {
        let t = l.trim_end();
        t.starts_with("if __name__") && t.contains("__main__") && t.ends_with(':')
    })
}
