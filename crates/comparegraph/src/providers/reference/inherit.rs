//! Inheritance edges (`extends`, `implements`): our own tree-sitter pass over each file, resolved like calls
//! (same file → imported name → unique name across the repo at 0.5 → else dropped; an external base gets no edge).
use super::resolve::{qualified_names, resolve_import_target};
use super::ParsedFile;
use crate::domain::EdgeKind;
use crate::providers::reference::vendor::SymbolKind;
use std::collections::HashMap;
use tree_sitter::Node as TsNode;

/// One raw relation found in a file: `sub` inherits from / implements `base` (bare names).
#[derive(Debug, PartialEq)]
pub struct Raw {
    pub kind: EdgeKind,
    pub sub: String,
    pub base: String,
    /// C#: a `base_list` does not say class from interface; the first entry is an `extends` unless it resolves to an interface.
    pub ambiguous: bool,
}

fn language_of(lang: &str) -> Option<tree_sitter::Language> {
    Some(match lang {
        "typescript" | "tsx" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "javascript" | "jsx" | "mjs" => tree_sitter_javascript::LANGUAGE.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "csharp" => tree_sitter_c_sharp::LANGUAGE.into(),
        _ => return None,
    })
}

/// `a.b.Base<T>` / `mod::Trait` / `Base[T]` → `Base`; None when no plain identifier remains.
fn bare(text: &str) -> Option<String> {
    let t = text.split(['<', '[', '(']).next()?.trim();
    let last = t.rsplit(['.', ':']).next()?.trim();
    (!last.is_empty() && last.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$'))
        .then(|| last.to_string())
}

fn text<'a>(n: TsNode, src: &'a str) -> &'a str {
    &src[n.byte_range()]
}

fn named_types(n: TsNode, src: &str, out: &mut Vec<String>) {
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        match ch.kind() {
            "type_list" => named_types(ch, src, out),
            "type_arguments" | "comment" | "type_parameters" => {}
            _ => {
                if let Some(b) = bare(text(ch, src)) {
                    out.push(b);
                }
            }
        }
    }
}

pub fn find(language: &str, src: &str) -> Vec<Raw> {
    let Some(lang) = language_of(language) else {
        return vec![];
    };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return vec![];
    }
    let Some(tree) = parser.parse(src, None) else {
        return vec![];
    };
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        let mut c = n.walk();
        stack.extend(n.children(&mut c));
        let name_of = |n: TsNode| n.child_by_field_name("name").and_then(|x| bare(text(x, src)));
        let mut push = |kind: EdgeKind, sub: &str, bases: Vec<String>, ambiguous: bool| {
            for base in bases {
                out.push(Raw {
                    kind,
                    sub: sub.to_string(),
                    base,
                    ambiguous,
                });
            }
        };
        match (language, n.kind()) {
            ("python", "class_definition") => {
                let (Some(sub), Some(sc)) = (name_of(n), n.child_by_field_name("superclasses"))
                else {
                    continue;
                };
                let mut c = sc.walk();
                let bases = sc
                    .named_children(&mut c)
                    .filter(|x| matches!(x.kind(), "identifier" | "attribute"))
                    .filter_map(|x| bare(text(x, src)))
                    .collect();
                push(EdgeKind::Extends, &sub, bases, false);
            }
            ("typescript" | "tsx" | "javascript" | "jsx" | "mjs", "class_declaration")
            | ("typescript" | "tsx", "abstract_class_declaration") => {
                let Some(sub) = name_of(n) else { continue };
                let mut c = n.walk();
                for h in n.children(&mut c).filter(|x| x.kind() == "class_heritage") {
                    let mut hc = h.walk();
                    for part in h.named_children(&mut hc) {
                        let mut v = Vec::new();
                        match part.kind() {
                            "extends_clause" => {
                                // value field(s): the base expression; skip type_arguments
                                let mut pc = part.walk();
                                for x in part.children_by_field_name("value", &mut pc) {
                                    if let Some(b) = bare(text(x, src)) {
                                        v.push(b);
                                    }
                                }
                                push(EdgeKind::Extends, &sub, v, false);
                            }
                            "implements_clause" => {
                                named_types(part, src, &mut v);
                                push(EdgeKind::Implements, &sub, v, false);
                            }
                            // javascript: the heritage holds the base expression directly
                            _ => {
                                if let Some(b) = bare(text(part, src)) {
                                    push(EdgeKind::Extends, &sub, vec![b], false);
                                }
                            }
                        }
                    }
                }
            }
            ("java", "class_declaration") => {
                let Some(sub) = name_of(n) else { continue };
                if let Some(sc) = n.child_by_field_name("superclass") {
                    let mut v = Vec::new();
                    named_types(sc, src, &mut v);
                    push(EdgeKind::Extends, &sub, v, false);
                }
                if let Some(si) = n.child_by_field_name("interfaces") {
                    let mut v = Vec::new();
                    named_types(si, src, &mut v);
                    push(EdgeKind::Implements, &sub, v, false);
                }
            }
            ("csharp", "class_declaration" | "struct_declaration" | "interface_declaration") => {
                let Some(sub) = name_of(n) else { continue };
                let mut c = n.walk();
                let Some(bl) = n.children(&mut c).find(|x| x.kind() == "base_list") else {
                    continue;
                };
                let mut v = Vec::new();
                named_types(bl, src, &mut v);
                for (i, b) in v.into_iter().enumerate() {
                    let (k, amb) = if i == 0 && n.kind() == "class_declaration" {
                        (EdgeKind::Extends, true)
                    } else {
                        (EdgeKind::Implements, false)
                    };
                    push(k, &sub, vec![b], amb);
                }
            }
            ("rust", "impl_item") => {
                let (Some(tr), Some(ty)) = (
                    n.child_by_field_name("trait"),
                    n.child_by_field_name("type"),
                ) else {
                    continue;
                };
                if let (Some(sub), Some(base)) = (bare(text(ty, src)), bare(text(tr, src))) {
                    push(EdgeKind::Implements, &sub, vec![base], false);
                }
            }
            _ => {}
        }
    }
    out
}

fn class_like(k: &SymbolKind) -> bool {
    !matches!(k, SymbolKind::Function | SymbolKind::Method)
}

/// Resolved inheritance edges for all files: (kind, subclass_qn, base_qn, confidence). Sorted by the caller.
pub fn inherit_edges(files: &[ParsedFile]) -> Vec<(EdgeKind, String, String, f64)> {
    let file_set: HashMap<String, ()> = files.iter().map(|f| (f.rel_path.clone(), ())).collect();
    // per file: name → (first qn, kind)
    let mut syms: HashMap<&str, HashMap<&str, (String, &SymbolKind)>> = HashMap::new();
    let mut by_name: HashMap<&str, Vec<(String, &SymbolKind)>> = HashMap::new();
    for f in files {
        let m = syms.entry(f.rel_path.as_str()).or_default();
        for (s, qn) in f.symbols.iter().zip(qualified_names(f)) {
            if !m.contains_key(s.name.as_str()) {
                m.insert(s.name.as_str(), (qn.clone(), &s.kind));
                if class_like(&s.kind) {
                    by_name
                        .entry(s.name.as_str())
                        .or_default()
                        .push((qn, &s.kind));
                }
            }
        }
    }
    let mut out = Vec::new();
    for f in files {
        let raws = find(&f.language, &f.lines.join("\n"));
        if raws.is_empty() {
            continue;
        }
        let local = &syms[f.rel_path.as_str()];
        for r in raws {
            let Some((sub_qn, _)) = local.get(r.sub.as_str()) else {
                continue;
            };
            let mut hit: Option<(String, &SymbolKind, f64)> = None;
            if let Some((qn, k)) = local.get(r.base.as_str()) {
                if class_like(k) {
                    hit = Some((qn.clone(), k, 1.0));
                }
            }
            if hit.is_none() {
                for imp in f.imports.iter().filter(|i| i.specifiers.contains(&r.base)) {
                    if let Some(t) = resolve_import_target(&f.rel_path, &imp.source, &file_set) {
                        if let Some((qn, k)) = syms.get(t.as_str()).and_then(|m| m.get(r.base.as_str())) {
                            hit = Some((qn.clone(), k, 1.0));
                            break;
                        }
                    }
                }
            }
            if hit.is_none() {
                if let Some([(qn, k)]) = by_name.get(r.base.as_str()).map(Vec::as_slice) {
                    hit = Some((qn.clone(), k, 0.5));
                }
            }
            let Some((qn, k, conf)) = hit else { continue };
            if qn == *sub_qn {
                continue;
            }
            let kind = if r.ambiguous && matches!(k, SymbolKind::Interface) {
                EdgeKind::Implements
            } else {
                r.kind
            };
            out.push((kind, sub_qn.clone(), qn, conf));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::ReferenceProvider;
    use crate::domain::EdgeKind;
    use crate::providers::{BuildOptions, GraphProvider};

    fn edges(files: &[(&str, &str)]) -> Vec<(EdgeKind, String, String, f64)> {
        let d = tempfile::tempdir().unwrap();
        for (p, c) in files {
            let full = d.path().join(p);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, c).unwrap();
        }
        let out = ReferenceProvider
            .build(d.path(), &BuildOptions::default())
            .unwrap();
        out.edges_by_qn
            .into_iter()
            .filter(|e| matches!(e.0, EdgeKind::Extends | EdgeKind::Implements))
            .collect()
    }
    fn e(k: EdgeKind, a: &str, b: &str, c: f64) -> (EdgeKind, String, String, f64) {
        (k, a.into(), b.into(), c)
    }

    #[test]
    fn python_imported_base_is_extends_and_external_base_is_dropped() {
        let got = edges(&[
            ("pkg/base.py", "class Base:\n    pass\n"),
            (
                "pkg/sub.py",
                "from .base import Base\nimport abc\n\nclass Sub(Base):\n    pass\n\nclass Ext(abc.ABC):\n    pass\n",
            ),
        ]);
        assert_eq!(got, vec![e(EdgeKind::Extends, "pkg/sub.py::Sub", "pkg/base.py::Base", 1.0)]);
    }
    #[test]
    fn typescript_extends_and_implements() {
        let got = edges(&[
            ("src/base.ts", "export class Base {}\nexport interface Shape { a(): void }\n"),
            (
                "src/sub.ts",
                "import { Base, Shape } from './base';\nimport { Ext } from 'lib';\nexport class Sub extends Base implements Shape {}\nexport class E2 extends Ext {}\n",
            ),
        ]);
        assert_eq!(
            got,
            vec![
                e(EdgeKind::Extends, "src/sub.ts::Sub", "src/base.ts::Base", 1.0),
                e(EdgeKind::Implements, "src/sub.ts::Sub", "src/base.ts::Shape", 1.0),
            ]
        );
    }
    #[test]
    fn javascript_extends() {
        let got = edges(&[
            ("a.js", "export class A {}\n"),
            ("b.js", "import { A } from './a';\nexport class B extends A {}\n"),
        ]);
        assert_eq!(got, vec![e(EdgeKind::Extends, "b.js::B", "a.js::A", 1.0)]);
    }
    #[test]
    fn java_extends_and_implements_by_unique_name_at_half() {
        let got = edges(&[
            ("p/Base.java", "package p;\npublic class Base {}\n"),
            ("p/Iface.java", "package p;\npublic interface Iface {}\n"),
            (
                "p/Sub.java",
                "package p;\npublic class Sub extends Base implements Iface, java.io.Serializable {}\n",
            ),
        ]);
        assert_eq!(
            got,
            vec![
                e(EdgeKind::Extends, "p/Sub.java::Sub", "p/Base.java::Base", 0.5),
                e(EdgeKind::Implements, "p/Sub.java::Sub", "p/Iface.java::Iface", 0.5),
            ]
        );
    }
    #[test]
    fn csharp_first_base_extends_rest_implement() {
        let got = edges(&[(
            "A.cs",
            "class Base {}\ninterface IFoo {}\nclass Sub : Base, IFoo {}\nclass Sub2 : IFoo {}\n",
        )]);
        assert_eq!(
            got,
            vec![
                e(EdgeKind::Extends, "A.cs::Sub", "A.cs::Base", 1.0),
                e(EdgeKind::Implements, "A.cs::Sub", "A.cs::IFoo", 1.0),
                e(EdgeKind::Implements, "A.cs::Sub2", "A.cs::IFoo", 1.0),
            ]
        );
    }
    #[test]
    fn rust_trait_impl_is_implements_and_std_trait_is_dropped() {
        let got = edges(&[(
            "src/lib.rs",
            "pub trait Shape { fn a(&self); }\npub struct Sq;\nimpl Shape for Sq { fn a(&self) {} }\nimpl std::fmt::Display for Sq { fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { Ok(()) } }\n",
        )]);
        assert_eq!(got, vec![e(EdgeKind::Implements, "src/lib.rs::Sq", "src/lib.rs::Shape", 1.0)]);
    }
}
