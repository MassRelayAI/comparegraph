//! One small fixture per parsed language under `fixtures/languages/<language>/`, built as ONE repo, with a
//! committed digest of each language's graph in `fixtures/languages/DIGESTS.json`. A grammar bump that changes
//! a digest fails here; the maintainer re-golds on purpose (`UPDATE_LANGUAGE_DIGESTS=1`).
use comparegraph::application::{add::add_repo, build::build_repo, export::export_json};
use comparegraph::providers::reference::walker::language_for;
use comparegraph::providers::BuildOptions;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// One extension (or file name) per language id the walker knows, and the id it must map to.
/// 29 languages; `tsx`, `jsx` and `mjs` are their own ids, so 32 ids in all.
const PROBES: &[(&str, &str)] = &[
    ("a.ts", "typescript"), ("a.tsx", "tsx"), ("a.js", "javascript"), ("a.cjs", "javascript"), ("a.mjs", "mjs"),
    ("a.jsx", "jsx"), ("a.py", "python"), ("a.go", "go"), ("a.rs", "rust"), ("a.rb", "ruby"), ("a.tf", "hcl"),
    ("a.hcl", "hcl"), ("a.yml", "yaml"), ("a.yaml", "yaml"), ("a.c", "c"), ("a.h", "c"), ("a.m", "objc"),
    ("a.java", "java"), ("a.php", "php"), ("a.swift", "swift"), ("a.sh", "bash"), ("a.bash", "bash"),
    ("a.pl", "perl"), ("a.pm", "perl"), ("a.css", "css"), ("a.scss", "scss"), ("a.html", "html"),
    ("a.htm", "html"), ("a.toml", "toml"), ("a.sql", "sql"), ("a.cpp", "cpp"), ("a.cc", "cpp"),
    ("a.cxx", "cpp"), ("a.hpp", "cpp"), ("a.cs", "csharp"), ("a.kt", "kotlin"), ("a.kts", "kotlin"),
    ("a.dart", "dart"), ("a.scala", "scala"), ("a.r", "r"), ("a.R", "r"), ("a.graphql", "graphql"),
    ("a.gql", "graphql"), ("a.ps1", "powershell"), ("Dockerfile", "dockerfile"),
];

fn language_ids() -> BTreeSet<&'static str> {
    let mut ids = BTreeSet::new();
    for (file, want) in PROBES {
        assert_eq!(language_for(Path::new(file)), Some(*want), "walker table drifted for {file}");
        ids.insert(*want);
    }
    ids
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/languages")
}

fn copy_tree(from: &Path, to: &Path) {
    for e in std::fs::read_dir(from).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap();
        if p.is_dir() {
            std::fs::create_dir_all(to.join(name)).unwrap();
            copy_tree(&p, &to.join(name));
        } else if name != "DIGESTS.json" && name != "README.md" {
            std::fs::copy(&p, to.join(name)).unwrap();
        }
    }
}

fn computed() -> BTreeMap<String, Value> {
    let ids = language_ids();
    assert_eq!(ids.len(), 32, "29 languages, 32 ids (tsx, jsx, mjs are their own ids)");
    let fx = fixtures_dir();
    for id in &ids {
        let dir = fx.join(id);
        let has_file = dir.is_dir() && std::fs::read_dir(&dir).unwrap().next().is_some();
        assert!(has_file, "no fixture for {id}: add fixtures/languages/{id}/ with a source file");
    }

    let repo = tempfile::tempdir().unwrap();
    copy_tree(&fx, repo.path());
    std::fs::write(repo.path().join("LICENSE"), "MIT License\n\nCopyright (c) 2026 S\n").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "."],
        vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "x"],
    ] {
        assert!(Command::new("git").arg("-C").arg(repo.path()).args(&args).status().unwrap().success());
    }
    let ws = tempfile::tempdir().unwrap();
    add_repo(ws.path(), repo.path(), Some("languages"), false).unwrap();
    build_repo(ws.path(), "languages", "reference", &BuildOptions::default()).unwrap();
    let j = export_json(ws.path(), Some("languages")).unwrap();
    let r = &j["repos"][0];

    let lang_of = |path: &str| language_for(Path::new(path)).map(str::to_string);
    let mut node_lines: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut edge_lines: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut files: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut kinds: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    let mut edge_kinds: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    let mut qn_path: BTreeMap<String, String> = BTreeMap::new();
    for n in r["nodes"].as_array().unwrap() {
        let path = n["path"].as_str().unwrap();
        let (kind, qn) = (n["kind"].as_str().unwrap(), n["qualified_name"].as_str().unwrap());
        qn_path.insert(qn.to_string(), path.to_string());
        let Some(l) = lang_of(path) else { continue };
        node_lines.entry(l.clone()).or_default().push(format!(
            "N|{kind}|{qn}|{path}|{}-{}",
            n["line_start"], n["line_end"]
        ));
        files.entry(l.clone()).or_default().insert(path.to_string());
        *kinds.entry(l).or_default().entry(kind.to_string()).or_default() += 1;
    }
    for e in r["edges"].as_array().unwrap() {
        let (kind, s, t) = (
            e["kind"].as_str().unwrap(),
            e["source_qn"].as_str().unwrap(),
            e["target_qn"].as_str().unwrap(),
        );
        let Some(l) = qn_path.get(s).and_then(|p| lang_of(p)) else { continue };
        edge_lines.entry(l.clone()).or_default().push(format!("E|{kind}|{s}|{t}|{}", e["confidence"]));
        *edge_kinds.entry(l).or_default().entry(kind.to_string()).or_default() += 1;
    }

    let mut out = BTreeMap::new();
    for id in ids {
        let mut lines: Vec<String> = node_lines.remove(id).unwrap_or_default();
        let nodes = lines.len();
        let mut el = edge_lines.remove(id).unwrap_or_default();
        let edges = el.len();
        lines.append(&mut el);
        lines.sort();
        let mut h = Sha256::new();
        for l in &lines {
            h.update(l.as_bytes());
            h.update(b"\n");
        }
        let obj = |m: Option<&BTreeMap<String, u64>>| -> Value {
            let mut o = Map::new();
            for (k, v) in m.into_iter().flatten() {
                o.insert(k.clone(), json!(v));
            }
            Value::Object(o)
        };
        out.insert(
            id.to_string(),
            json!({
                "files": files.get(id).map(|f| f.len()).unwrap_or(0),
                "nodes": nodes,
                "edges": edges,
                "kinds": obj(kinds.get(id)),
                "edge_kinds": obj(edge_kinds.get(id)),
                "digest": hex::encode(h.finalize()),
            }),
        );
    }
    out
}

/// nodes other than the per-file node
fn symbols(v: &Value) -> u64 {
    v["nodes"].as_u64().unwrap() - v["kinds"]["file"].as_u64().unwrap_or(0)
}

fn describe(c: &Value, g: &Value) -> String {
    let mut s = String::new();
    for k in ["files", "nodes", "edges", "kinds", "edge_kinds", "digest"] {
        if c[k] != g[k] {
            s.push_str(&format!("    {k}: committed {} -> now {}\n", g[k], c[k]));
        }
    }
    s
}

#[test]
fn every_language_has_a_fixture_and_its_graph_digest_matches() {
    let now = computed();
    let path = fixtures_dir().join("DIGESTS.json");
    if std::env::var("UPDATE_LANGUAGE_DIGESTS").as_deref() == Ok("1") {
        let mut m = Map::new();
        let old: Value = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        for (k, mut v) in now {
            if symbols(&v) == 0 {
                v["note"] = old[&k]["note"].clone().as_str().map(|s| json!(s)).unwrap_or(json!("the extractor yields only the file node for this language (no symbols)"));
            }
            m.insert(k, v);
        }
        std::fs::write(&path, serde_json::to_string_pretty(&Value::Object(m)).unwrap() + "\n").unwrap();
        return;
    }
    let committed: Value = serde_json::from_str(
        &std::fs::read_to_string(&path).expect("fixtures/languages/DIGESTS.json missing: UPDATE_LANGUAGE_DIGESTS=1"),
    )
    .unwrap();
    let mut report = String::new();
    for (lang, c) in &now {
        let g = &committed[lang];
        if g.is_null() {
            report.push_str(&format!("{lang}: no committed digest\n"));
            continue;
        }
        assert!(c["nodes"].as_u64().unwrap() > 0, "{lang}: the fixture parsed to nothing, not even a file node");
        let mut g2 = g.clone();
        if let Some(o) = g2.as_object_mut() {
            o.remove("note");
        }
        if &g2 != c {
            report.push_str(&format!("{lang}: graph changed\n{}", describe(c, &g2)));
        }
        if symbols(c) == 0 && g["note"].is_null() {
            report.push_str(&format!("{lang}: yields no symbols and DIGESTS.json has no note saying why\n"));
        }
    }
    for k in committed.as_object().unwrap().keys() {
        if !now.contains_key(k) {
            report.push_str(&format!("{k}: committed digest for a language the walker no longer has\n"));
        }
    }
    assert!(
        report.is_empty(),
        "language graph digests differ (re-gold on purpose: UPDATE_LANGUAGE_DIGESTS=1 cargo test --test languages):\n{report}"
    );
}
