//! Multibyte source text must never panic a build: a char search that yields a byte index is advanced by `len_utf8`, not by 1.
//! A panic in the resolution passes fails the run with the file named instead of crashing the process.
use super::{ReferenceProvider, PANIC_AT};
use crate::domain::EdgeKind;
use crate::providers::{BuildOptions, GraphProvider, ProviderError};

fn repo(files: &[(String, String)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let full = d.path().join(p);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, c).unwrap();
    }
    d
}

#[test]
fn java_declaration_after_a_multibyte_char_resolves_the_typed_receiver() {
    for lead in ["×", "─", "→", "é", "日本", "😀"] {
        let java = format!(
            "class Widget {{\n  void go() {{}}\n}}\nclass App {{\n  void run() {{\n    // {lead} note\n    {lead}Widget w;\n    w = null;\n    w.go();\n  }}\n}}\n"
        );
        let d = repo(&[("src/App.java".into(), java)]);
        let o = ReferenceProvider.build(d.path(), &BuildOptions::default()).unwrap_or_else(|e| panic!("{lead}: {e}"));
        assert!(o.files_parsed >= 1);
    }
}

#[test]
fn declared_type_survives_multibyte_before_the_type_and_still_reads_ascii() {
    use super::resolve::declared_type;
    for lead in ["×", "─", "→", "😀"] {
        let t = format!("{lead}Widget w;\nw.go();");
        // the type is glued to a multibyte char: the identifier run starts after the char, so the type is `Widget`
        assert_eq!(declared_type(&t, "w"), Some("Widget"), "{lead}");
    }
    for lead in ["×", "─", "→", "日本", "😀"] {
        let t = format!("{lead} Widget w;");
        assert_eq!(declared_type(&t, "w"), Some("Widget"), "{lead} spaced");
    }
    assert_eq!(declared_type("Widget w;", "w"), Some("Widget"));
    assert_eq!(declared_type("x = 1; Widget w;", "w"), Some("Widget"));
}

#[test]
fn rust_and_ts_lines_with_an_arrow_before_an_identifier_do_not_panic() {
    let rs = "pub struct Store;\nimpl Store { pub fn new() -> Store { Store } }\npub fn run() {\n    // → Store here ─ ×\n    let s = → Store::new();\n    let t = Store::new();\n}\n".to_string();
    let ts = "class Box2 { go() {} }\nfunction run() {\n  // → Box2\n  const b: →Box2 = new Box2();\n  b.go();\n}\n".to_string();
    let d = repo(&[("Cargo.toml".into(), "[package]\nname = \"d\"\n".into()), ("src/lib.rs".into(), rs), ("src/a.ts".into(), ts)]);
    let o = ReferenceProvider.build(d.path(), &BuildOptions::default()).unwrap();
    // the ASCII call still resolves
    assert!(o.edges_by_qn.iter().any(|e| e.0 == EdgeKind::Calls && e.1.ends_with("::run") && e.2.contains("new")), "{:?}", o.edges_by_qn);
}

const MB: &str = "× ─ → é ß 日本 😀";

fn sample(ext: &str) -> String {
    let (c1, c2) = match ext {
        "py" | "rb" | "sh" | "pl" | "r" | "ps1" | "tf" | "hcl" | "yml" | "yaml" | "toml" | "Dockerfile" => ("#", "#"),
        "sql" => ("--", "--"),
        "css" | "scss" | "html" | "htm" | "graphql" | "gql" => ("/*", "*/"),
        _ => ("//", ""),
    };
    let c = |t: &str| format!("{c1} {t} {c2}\n");
    let body = match ext {
        "py" => String::from("import os\nfrom .é import ß\nclass Café:\n    def run(self):\n        x × y\n        日本 = Café()\n        日本.run()\n        os.path.join('─', '→')\n"),
        "ts" | "tsx" | "js" | "cjs" | "mjs" | "jsx" => String::from("import { ß } from './é';\nclass Café { run() { ß(); } }\nfunction f() { const 日本: →Café = new Café(); 日本.run(); const s = '× ─ →'; }\n"),
        "go" => String::from("package main\nimport \"fmt\"\ntype Café struct{}\nfunc (c Café) Run() {}\nfunc main() { var 日本 Café; 日本.Run(); fmt.Println(\"× ─ →\") }\n"),
        "rs" => String::from("use std::fmt;\nstruct Café;\nimpl Café { fn new() -> Café { Café } fn run(&self) {} }\nfn main() { let 日本 = Café::new(); 日本.run(); let s = \"× ─ →\"; }\n"),
        "java" | "cs" | "kt" | "kts" | "scala" | "dart" | "swift" => String::from("import a.é;\nclass Café { void run() {} }\nclass App { void go() { ×Café 日本; ─Café x = new Café(); →Café y; 日本.run(); String s = \"× ─ →\"; } }\n"),
        "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "m" => String::from("#include <stdio.h>\nint café(void) { return 1; }\nint main(void) { int 日本 = café(); printf(\"× ─ →\"); return 0; }\n"),
        "php" => String::from("<?php\nclass Café { function run() {} }\nfunction f() { $日本 = new Café(); $日本->run(); echo '× ─ →'; }\n"),
        "sh" | "bash" => String::from("f() { echo '× ─ →'; g; }\ng() { echo é; }\nf\n"),
        "rb" => String::from("class Café\n  def run; end\nend\ndef f\n  日本 = Café.new\n  日本.run\nend\n"),
        _ => String::new(),
    };
    format!("{}{}{}{}", c(MB), body, c(MB), c("日本 emoji 😀 × ─ → é ß"))
}

#[test]
fn every_walker_language_with_multibyte_text_builds_without_panicking() {
    let exts = [
        "ts", "tsx", "js", "cjs", "mjs", "jsx", "py", "go", "rs", "rb", "tf", "hcl", "yml", "yaml", "c", "h", "m", "java", "php", "swift", "sh", "bash", "pl", "pm", "css", "scss", "html",
        "htm", "toml", "sql", "cpp", "cc", "cxx", "hpp", "cs", "kt", "kts", "dart", "scala", "r", "graphql", "gql", "ps1",
    ];
    // every extension the walker maps is covered here
    for e in &exts {
        assert!(super::walker::language_for(std::path::Path::new(&format!("x.{e}"))).is_some(), "{e}");
    }
    assert!(super::walker::language_for(std::path::Path::new("Dockerfile")).is_some());
    let mut files: Vec<(String, String)> = exts.iter().map(|e| (format!("src/f_{e}.{e}"), sample(e))).collect();
    files.push(("Dockerfile".into(), format!("# {MB}\nFROM scratch\nRUN echo '× ─ →'\n")));
    files.push(("Cargo.toml".into(), format!("# {MB}\n[package]\nname = \"é\"\n")));
    for e in ["ts", "py", "rs", "js"] {
        files.push((format!("tests/t_{e}.test.{e}"), sample(e)));
    }
    files.push(("src/s.test.ts".into(), format!("describe('日本 ×', () => {{ it('─ →', () => {{ f(); }}); }});\n{}", sample("ts"))));
    let d = repo(&files);
    let r = ReferenceProvider.build(d.path(), &BuildOptions::default());
    let o = r.unwrap_or_else(|e| panic!("{e}"));
    assert!(o.files_seen >= exts.len());
}

#[test]
fn a_panic_in_resolution_fails_the_run_naming_the_file() {
    let d = repo(&[("src/a.rs".into(), "pub fn a() {}\n".into()), ("src/b.rs".into(), "pub fn b() { a(); }\n".into())]);
    PANIC_AT.with(|p| *p.borrow_mut() = Some("src/b.rs".into()));
    let r = ReferenceProvider.build(d.path(), &BuildOptions::default());
    PANIC_AT.with(|p| *p.borrow_mut() = None);
    match r {
        Err(ProviderError::Panicked(f, m)) => {
            assert_eq!(f, "src/b.rs");
            assert!(m.contains("injected panic"), "{m}");
        }
        other => panic!("expected Panicked, got {other:?}"),
    }
    // the hook is off again: the same repo builds
    assert!(ReferenceProvider.build(d.path(), &BuildOptions::default()).is_ok());
}
