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

/// The vendored SQL extractor yields no symbol (its node names do not match the grammar it is given: the language fixture's
/// CREATE TABLE and VIEW give none, fixtures/languages/DIGESTS.json `sql` nodes 1 = files 1). A file of such a language cannot
/// hide a symbol, so its parse errors do not count as unread (`NO_SYMBOL_LANGUAGES`). If SQL ever yields one, this fails and the
/// language leaves the list.
#[test]
fn sql_yields_no_symbol_so_it_is_a_no_symbol_language() {
    use comparegraph::providers::reference::{parse_file, NO_SYMBOL_LANGUAGES};
    let sql = "CREATE TABLE users (id int, name text);\n\nCREATE VIEW v AS SELECT * FROM users;\n\nCREATE INDEX i ON users (name);\n\nCREATE FUNCTION add_one(x int) RETURNS int AS $$ SELECT x + 1 $$ LANGUAGE sql;\n";
    assert!(parse_file("db/s.sql", sql, "sql").unwrap().symbols.is_empty());
    assert_eq!(NO_SYMBOL_LANGUAGES, ["sql"]);
}

/// A base `.sql` file with parse errors no longer blocks R4: the branch deletes g, base deletes it too (and changes a line
/// elsewhere, so the merge leaves base's file as it is) and adds a migration the SQL grammar cannot parse.
#[test]
fn a_sql_parse_error_on_base_does_not_block_a_removal_yes() {
    let g = "def g(items):\n    total = 0\n    for x in items:\n        total = total + x * 3 - len(items)\n    return total\n";
    let foo = "def foo():\n    return 1\n";
    let (ws, base) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let d = base.path().join("r");
    std::fs::create_dir_all(&d).unwrap();
    clone_with(&d, &[("src/a.py", &format!("{foo}\n{g}"))], MIT);
    common::git(&d, &["branch", "-M", "main"]);
    ok(ws.path(), &["add", d.to_str().unwrap(), "--name", "r"]);
    common::git(&d, &["checkout", "-q", "-b", "feat"]);
    std::fs::write(d.join("src/a.py"), foo).unwrap();
    common::commit_all(&d);
    common::git(&d, &["checkout", "-q", "main"]);
    std::fs::write(d.join("src/a.py"), format!("# kept by base\n{foo}")).unwrap();
    std::fs::create_dir_all(d.join("db")).unwrap();
    std::fs::write(d.join("db/m.sql"), "CREATE TABLE ((( broken;\n").unwrap();
    common::commit_all(&d);
    assert_eq!(has_parse_error("CREATE TABLE ((( broken;\n", "sql"), Some(true));
    let v = ok(ws.path(), &["delta", "r", "--base", "main", "--head", "feat", "--refresh"]);
    let row = v["data"]["branch"]["removed"].as_array().unwrap().iter().find(|r| r["qualified_name"] == "src/a.py::g").unwrap().clone();
    assert_eq!((row["on_base"].as_str(), row["via"].as_str()), (Some("yes"), Some("absent_on_base")), "{row} {}", v["evidence"]["limitations"]);
}

/// Round 6: the SQL pin, at the grammar: none of the four node kinds the vendored SQL extractor matches exists in the SQL
/// grammar it is given (`id_for_node_kind` 0), so it can never yield a symbol.
#[test]
fn the_sql_grammar_has_none_of_the_kinds_the_sql_extractor_matches() {
    let lang: tree_sitter::Language = tree_sitter_sql::LANGUAGE.into();
    for k in ["create_table_statement", "create_function_statement", "create_view_statement", "create_index_statement"] {
        assert_eq!(lang.id_for_node_kind(k, true), 0, "{k} is a node kind of the SQL grammar now");
    }
}
