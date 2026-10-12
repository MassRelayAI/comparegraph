//! Rust context in our own layer (the vendored Rust extractor keeps `use` paths as raw text and ignores `impl` blocks):
//! `use` items with aliases, `mod foo;` declarations, `impl X` spans and the declared type of local variables, read with
//! tree-sitter-rust; plus the module-path → file mapping (`crate::`, `self::`, `super::`, child modules).
use std::collections::HashMap;
use tree_sitter::Node;

#[derive(Debug, Default, PartialEq)]
pub struct FnVars {
    pub start: u32,
    pub end: u32,
    /// variable → type name (`p: &Parser`, `let p = Parser::new()`, `let p: Parser`)
    pub vars: HashMap<String, String>,
}

#[derive(Debug, Default, PartialEq)]
pub struct RustInfo {
    /// alias → full path segments (`use crate::util::Parser as P` → "P" → [crate, util, Parser])
    pub uses: Vec<(String, Vec<String>)>,
    /// `mod foo;`
    pub mods: Vec<String>,
    /// (type name, first line, last line) of every `impl X` / `impl Trait for X`
    pub impls: Vec<(String, u32, u32)>,
    pub fns: Vec<FnVars>,
}

fn text<'a>(n: Node, src: &'a str) -> &'a str {
    &src[n.byte_range()]
}

fn segs(t: &str) -> Vec<String> {
    t.split("::").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// `&'a mut a::Parser<T>` → `Parser`
fn type_name(t: &str) -> Option<String> {
    let t = t.trim().trim_start_matches('&').trim();
    let t = if t.starts_with('\'') { t.split_once(' ')?.1.trim() } else { t };
    let t = t.strip_prefix("mut ").unwrap_or(t).trim();
    let t = t.split('<').next()?.trim();
    let last = t.rsplit("::").next()?.trim();
    (!last.is_empty() && last.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| last.to_string())
}

fn flatten(n: Node, src: &str, prefix: &[String], out: &mut Vec<(String, Vec<String>)>) {
    match n.kind() {
        "identifier" | "scoped_identifier" | "crate" | "super" => {
            let mut p = prefix.to_vec();
            p.extend(segs(text(n, src)));
            if let Some(a) = p.last().cloned() {
                out.push((a, p));
            }
        }
        "self" => {
            if let Some(a) = prefix.last().cloned() {
                out.push((a, prefix.to_vec()));
            }
        }
        "use_as_clause" => {
            let (Some(path), Some(alias)) = (n.child_by_field_name("path"), n.child_by_field_name("alias")) else {
                return;
            };
            let mut p = prefix.to_vec();
            p.extend(segs(text(path, src)));
            out.push((text(alias, src).to_string(), p));
        }
        "scoped_use_list" => {
            let mut p = prefix.to_vec();
            if let Some(path) = n.child_by_field_name("path") {
                p.extend(segs(text(path, src)));
            }
            if let Some(list) = n.child_by_field_name("list") {
                flatten(list, src, &p, out);
            }
        }
        "use_list" => {
            let mut c = n.walk();
            for ch in n.named_children(&mut c) {
                flatten(ch, src, prefix, out);
            }
        }
        _ => {}
    }
}

fn collect_vars(n: Node, src: &str, vars: &mut HashMap<String, String>) {
    match n.kind() {
        "parameter" => {
            if let (Some(p), Some(t)) = (n.child_by_field_name("pattern"), n.child_by_field_name("type")) {
                if p.kind() == "identifier" {
                    if let Some(tn) = type_name(text(t, src)) {
                        vars.insert(text(p, src).to_string(), tn);
                    }
                }
            }
        }
        "let_declaration" => {
            if let Some(p) = n.child_by_field_name("pattern").filter(|p| p.kind() == "identifier") {
                let name = text(p, src).to_string();
                let ty = n.child_by_field_name("type").and_then(|t| type_name(text(t, src))).or_else(|| {
                    let v = n.child_by_field_name("value")?;
                    match v.kind() {
                        "struct_expression" => type_name(text(v.child_by_field_name("name")?, src)),
                        "call_expression" => {
                            let f = v.child_by_field_name("function")?;
                            if f.kind() != "scoped_identifier" {
                                return None;
                            }
                            let path = text(f.child_by_field_name("path")?, src);
                            type_name(path).filter(|t| t.chars().next().is_some_and(char::is_uppercase))
                        }
                        _ => None,
                    }
                });
                if let Some(t) = ty {
                    vars.insert(name, t);
                }
            }
        }
        _ => {}
    }
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        collect_vars(ch, src, vars);
    }
}

pub fn analyze(src: &str) -> RustInfo {
    let mut info = RustInfo::default();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&tree_sitter_rust::LANGUAGE.into()).is_err() {
        return info;
    }
    let Some(tree) = parser.parse(src, None) else {
        return info;
    };
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        let (s, e) = (n.start_position().row as u32 + 1, n.end_position().row as u32 + 1);
        match n.kind() {
            "use_declaration" => {
                if let Some(a) = n.child_by_field_name("argument") {
                    flatten(a, src, &[], &mut info.uses);
                }
                continue;
            }
            "mod_item" => {
                if n.child_by_field_name("body").is_none() {
                    if let Some(name) = n.child_by_field_name("name") {
                        info.mods.push(text(name, src).to_string());
                    }
                }
            }
            "impl_item" => {
                if let Some(t) = n.child_by_field_name("type").and_then(|t| type_name(text(t, src))) {
                    info.impls.push((t, s, e));
                }
            }
            "function_item" => {
                let mut f = FnVars { start: s, end: e, ..Default::default() };
                collect_vars(n, src, &mut f.vars);
                info.fns.push(f);
            }
            _ => {}
        }
        let mut c = n.walk();
        stack.extend(n.named_children(&mut c));
    }
    info.uses.sort();
    info.uses.dedup();
    info.mods.sort();
    info.impls.sort();
    info.fns.sort_by_key(|f| (f.start, f.end));
    info
}

/// The directory holding a crate's modules and root file: the prefix up to the rightmost `src` component.
pub fn crate_src(from: &str) -> Option<String> {
    let parts: Vec<&str> = from.split('/').collect();
    let i = parts.iter().rposition(|p| *p == "src")?;
    if i + 1 >= parts.len() {
        return None;
    }
    Some(parts[..=i].join("/"))
}

fn join(dir: &str, rest: &str) -> String {
    if dir.is_empty() {
        rest.to_string()
    } else {
        format!("{dir}/{rest}")
    }
}

/// A module: the directory its children live in and its file.
#[derive(Clone)]
struct Module {
    dir: String,
    file: String,
}

fn module_of_file(from: &str) -> Module {
    let (dir, name) = from.rsplit_once('/').unwrap_or(("", from));
    let stem = name.trim_end_matches(".rs");
    let child_dir = if matches!(stem, "lib" | "main" | "mod") { dir.to_string() } else { join(dir, stem) };
    Module { dir: child_dir, file: from.to_string() }
}

fn child(m: &Module, seg: &str, files: &HashMap<String, ()>) -> Option<Module> {
    [join(&m.dir, &format!("{seg}.rs")), join(&m.dir, &format!("{seg}/mod.rs"))]
        .into_iter()
        .find(|c| files.contains_key(c))
        .map(|file| Module { dir: join(&m.dir, seg), file })
}

fn parent(m: &Module, files: &HashMap<String, ()>) -> Option<Module> {
    let (pdir, _) = m.dir.rsplit_once('/')?;
    let name = pdir.rsplit('/').next().unwrap_or(pdir);
    let up = pdir.rsplit_once('/').map_or("", |d| d.0);
    [join(pdir, "mod.rs"), join(up, &format!("{name}.rs")), join(pdir, "lib.rs"), join(pdir, "main.rs")]
        .into_iter()
        .find(|c| files.contains_key(c))
        .map(|file| Module { dir: pdir.to_string(), file })
}

fn crate_root(from: &str, files: &HashMap<String, ()>) -> Option<Module> {
    let src = crate_src(from)?;
    ["lib.rs", "main.rs"]
        .iter()
        .map(|r| join(&src, r))
        .find(|c| files.contains_key(c))
        .map(|file| Module { dir: src, file })
}

/// Child module files of `mod foo;` declarations (for `imports` edges).
pub fn mod_files(from: &str, mods: &[String], files: &HashMap<String, ()>) -> Vec<String> {
    let m = module_of_file(from);
    mods.iter().filter_map(|n| child(&m, n, files)).map(|c| c.file).collect()
}

/// A `use`/path walked through modules: the file of the module it ends in, and the item name when the last segment is an item
/// (not a child module). External crates and unknown modules → None.
pub fn resolve_path(from: &str, path: &[String], files: &HashMap<String, ()>) -> Option<(String, Option<String>)> {
    let first = path.first()?;
    let (mut m, rest): (Module, &[String]) = match first.as_str() {
        "crate" => (crate_root(from, files)?, &path[1..]),
        "self" => (module_of_file(from), &path[1..]),
        "super" => {
            let mut m = parent(&module_of_file(from), files)?;
            let mut r = &path[1..];
            while r.first().is_some_and(|s| s == "super") {
                m = parent(&m, files)?;
                r = &r[1..];
            }
            (m, r)
        }
        _ => (module_of_file(from), path),
    };
    let Some((last, mods)) = rest.split_last() else {
        return Some((m.file, None));
    };
    for seg in mods {
        m = child(&m, seg, files)?;
    }
    match child(&m, last, files) {
        Some(c) => Some((c.file, None)),
        None if path.len() == rest.len() && rest.len() == 1 => None, // bare `use foo;` of something that is not a child module
        None => Some((m.file, Some(last.clone()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn set(l: &[&str]) -> HashMap<String, ()> {
        l.iter().map(|s| (s.to_string(), ())).collect()
    }
    #[test]
    fn use_groups_aliases_mods_impls_and_vars() {
        let i = analyze(
            "mod util;\nmod inline { }\nuse crate::a::{B, C as D, e::{self, F}};\nuse super::G;\nstruct S;\nimpl<T> Tr for S<T> {\n fn m(p: &mut Parser) { let q = Other::new(); let r: Third = x(); }\n}\n",
        );
        assert_eq!(i.mods, vec!["util"]);
        let u: Vec<(String, String)> = i.uses.iter().map(|(a, p)| (a.clone(), p.join("::"))).collect();
        for (a, p) in [("B", "crate::a::B"), ("D", "crate::a::C"), ("e", "crate::a::e"), ("F", "crate::a::e::F"), ("G", "super::G")] {
            assert!(u.contains(&(a.into(), p.into())), "{u:?}");
        }
        assert_eq!(i.impls, vec![("S".to_string(), 6, 8)]);
        let v = &i.fns[0].vars;
        assert_eq!((v["p"].as_str(), v["q"].as_str(), v["r"].as_str()), ("Parser", "Other", "Third"));
    }
    #[test]
    fn paths_map_to_files() {
        let f = set(&["c/src/lib.rs", "c/src/util.rs", "c/src/a/mod.rs", "c/src/a/b.rs", "c/src/a/c.rs"]);
        let p = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(resolve_path("c/src/lib.rs", &p(&["crate", "util", "Parser"]), &f), Some(("c/src/util.rs".into(), Some("Parser".into()))));
        assert_eq!(resolve_path("c/src/a/b.rs", &p(&["super", "c", "X"]), &f), Some(("c/src/a/c.rs".into(), Some("X".into()))));
        assert_eq!(resolve_path("c/src/a/b.rs", &p(&["super", "super", "util"]), &f), Some(("c/src/util.rs".into(), None)));
        assert_eq!(resolve_path("c/src/a/mod.rs", &p(&["self", "b", "Y"]), &f), Some(("c/src/a/b.rs".into(), Some("Y".into()))));
        assert_eq!(resolve_path("c/src/lib.rs", &p(&["std", "fmt"]), &f), None);
        assert_eq!(resolve_path("c/src/lib.rs", &p(&["crate", "Top"]), &f), Some(("c/src/lib.rs".into(), Some("Top".into()))));
        assert_eq!(mod_files("c/src/lib.rs", &p(&["util", "zzz"]), &f), vec!["c/src/util.rs".to_string()]);
    }
}
