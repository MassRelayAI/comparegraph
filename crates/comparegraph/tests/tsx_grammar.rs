//! The reference provider reads `.tsx` with tree-sitter-typescript's TSX grammar (round 5, option 3): the vendored parser maps
//! `tsx` to the TypeScript grammar, which cannot read JSX; the provider overrides that choice from outside `vendor/` (the
//! vendored files stay byte-identical, docs/THIRD_PARTY_NOTICES.md). `delta`'s parse-error check uses the same choice.
mod common;
use common::{clone_with, ok};
use comparegraph::providers::reference::has_parse_error;

const TSX: &str = "import React from 'react';\n\nexport function Card(props: { name: string }) {\n  const label = props.name.trim();\n  return <div className=\"card\"><span>{label}</span></div>;\n}\n\nexport function after(x: number): number {\n  const y = x * 2;\n  return y + 1;\n}\n\nexport const Row = ({ v }: { v: string }) => <li key={v}>{v}</li>;\n";
const JSX: &str = "export function Card(props) {\n  return <div className=\"card\">{props.name}</div>;\n}\n\nexport function after(x) {\n  return x + 1;\n}\n";
const MIT: &str = "MIT License\n\nCopyright (c) 2026 S\n";

/// A JSX expression inside a function: the file parses without error and every function is extracted.
#[test]
fn a_tsx_file_with_jsx_parses_cleanly_and_keeps_its_symbols() {
    assert_eq!(has_parse_error(TSX, "tsx"), Some(false), "JSX in .tsx read with an error");
    assert_eq!(has_parse_error(JSX, "jsx"), Some(false), "JSX in .jsx read with an error");
    let (ws, base) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let d = base.path().join("r");
    std::fs::create_dir_all(&d).unwrap();
    clone_with(&d, &[("src/Card.tsx", TSX), ("src/Card2.jsx", JSX)], MIT);
    ok(ws.path(), &["add", d.to_str().unwrap(), "--name", "r"]);
    ok(ws.path(), &["build", "r"]);
    let db = rusqlite::Connection::open(ws.path().join(".comparegraph/comparegraph.db")).unwrap();
    let names = |path: &str| -> Vec<String> {
        let mut st = db.prepare("SELECT name FROM node WHERE path=?1 AND kind IN ('function','method','class','type') ORDER BY name").unwrap();
        st.query_map([path], |r| r.get::<_, String>(0)).unwrap().map(|r| r.unwrap()).collect()
    };
    let tsx = names("src/Card.tsx");
    for n in ["Card", "Row", "after"] {
        assert!(tsx.contains(&n.to_string()), "{n} not extracted from the .tsx file: {tsx:?}");
    }
    let jsx = names("src/Card2.jsx");
    for n in ["Card", "after"] {
        assert!(jsx.contains(&n.to_string()), "{n} not extracted from the .jsx file: {jsx:?}");
    }
}
