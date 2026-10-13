use comparegraph::application::{add::add_repo, build::build_repo};
use comparegraph::providers::BuildOptions;
use std::path::Path;

fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let full = d.path().join(p);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, c).unwrap();
    }
    d
}
fn ws(repo: &Path) -> tempfile::TempDir {
    let w = tempfile::tempdir().unwrap();
    add_repo(w.path(), repo, Some("r"), false).unwrap();
    w
}
const A: &str = "from b import helper\n\ndef main():\n    return helper()\n";
const B: &str = "def helper():\n    return 1\n";
const B2: &str = "def helper():\n    return 2\n\ndef extra():\n    return helper()\n";

#[test]
fn incremental_after_one_edit_equals_a_full_build_of_the_same_tree() {
    let d = tree(&[("a.py", A), ("b.py", B), ("c.py", "def c():\n    pass\n")]);
    let w = ws(d.path());
    let first = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    assert_eq!((first.cache_hits, first.cache_misses), (0, 3));
    std::fs::write(d.path().join("b.py"), B2).unwrap();
    let inc = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    assert_eq!((inc.cache_hits, inc.cache_misses), (2, 1));
    let fresh = ws(d.path());
    let full = build_repo(fresh.path(), "r", "reference", &BuildOptions::default()).unwrap();
    assert_eq!(inc.store_hash, full.store_hash);
    assert_ne!(first.store_hash, inc.store_hash);
}

#[test]
fn full_bypasses_the_cache_and_a_removed_file_disappears() {
    let d = tree(&[("a.py", A), ("b.py", B)]);
    let w = ws(d.path());
    build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    let full = build_repo(
        w.path(),
        "r",
        "reference",
        &BuildOptions {
            full: true,
            ..BuildOptions::default()
        },
    )
    .unwrap();
    assert_eq!((full.cache_hits, full.cache_misses), (0, 0));
    std::fs::remove_file(d.path().join("b.py")).unwrap();
    let after = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    assert_eq!((after.cache_hits, after.cache_misses), (1, 0));
    let fresh = ws(d.path());
    let f = build_repo(fresh.path(), "r", "reference", &BuildOptions::default()).unwrap();
    assert_eq!(after.store_hash, f.store_hash);
    assert_eq!(after.nodes, f.nodes);
    let store = comparegraph::store::Store::open(w.path()).unwrap();
    let nodes = store.nodes_of_run(after.run).unwrap();
    assert!(nodes.iter().all(|n| !n.path.starts_with("b.py")));
}

fn run_rows(w: &Path) -> i64 {
    let c = rusqlite::Connection::open(w.join(".comparegraph").join("comparegraph.db"))
        .or_else(|_| rusqlite::Connection::open(w.join("comparegraph.db")))
        .unwrap();
    c.query_row("SELECT count(*) FROM run", [], |r| r.get(0)).unwrap()
}

#[test]
fn a_no_op_build_reuses_the_latest_run() {
    let d = tree(&[("a.py", A), ("b.py", B)]);
    let w = ws(d.path());
    let first = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    assert!(!first.reused);
    let rows = run_rows(w.path());
    let again = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    assert!(again.reused);
    assert_eq!(again.run, first.run);
    assert_eq!(again.store_hash, first.store_hash);
    assert_eq!((again.nodes, again.edges), (first.nodes, first.edges));
    assert_eq!(run_rows(w.path()), rows);
}

#[test]
fn a_changed_or_removed_file_or_full_makes_a_new_run() {
    let d = tree(&[("a.py", A), ("b.py", B)]);
    let w = ws(d.path());
    let first = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    std::fs::write(d.path().join("b.py"), B2).unwrap();
    let changed = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    assert!(!changed.reused);
    assert_ne!(changed.run, first.run);
    let same = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    assert!(same.reused);
    assert_eq!(same.run, changed.run);
    std::fs::write(d.path().join("c.py"), "def c():\n    pass\n").unwrap();
    let added = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    assert!(!added.reused);
    std::fs::remove_file(d.path().join("c.py")).unwrap();
    let removed = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    assert!(!removed.reused);
    assert_ne!(removed.run, added.run);
    let full = BuildOptions {
        full: true,
        ..BuildOptions::default()
    };
    let f1 = build_repo(w.path(), "r", "reference", &full).unwrap();
    let f2 = build_repo(w.path(), "r", "reference", &full).unwrap();
    assert!(!f1.reused && !f2.reused);
    assert_ne!(f1.run, f2.run);
}

// ---- Plan 6 B2: incremental edges and communities --------------------------------------------------------------------------

type CommRow = (String, Vec<String>, Option<f64>, Vec<String>);
/// Every community of a run by value: label, keywords, cohesion, member qualified names (ids are per run).
fn communities_of(w: &Path, run: comparegraph::domain::RunId) -> Vec<CommRow> {
    let store = comparegraph::store::Store::open(w).unwrap();
    let qn: std::collections::HashMap<_, _> = store
        .nodes_of_run(run)
        .unwrap()
        .into_iter()
        .map(|n| (n.id.unwrap(), n.qualified_name))
        .collect();
    let mut v: Vec<CommRow> = store
        .communities_of_run(run)
        .unwrap()
        .into_iter()
        .map(|c| {
            let mut m: Vec<String> = c.members.iter().map(|i| qn[i].clone()).collect();
            m.sort();
            (c.label, c.keywords, c.cohesion, m)
        })
        .collect();
    v.sort_by(|a, b| a.3.cmp(&b.3));
    v
}

fn base_files() -> Vec<(&'static str, String)> {
    let mut v: Vec<(&'static str, String)> = vec![
        ("a.py", A.to_string()),
        ("b.py", B.to_string()),
        ("f.py", "def user():\n    return extra()\n".to_string()),
    ];
    for (i, n) in ["u1.py", "u2.py", "u3.py", "u4.py", "u5.py", "u6.py", "u7.py", "u8.py"].into_iter().enumerate() {
        v.push((n, format!("def leaf{i}():\n    return {i}\n\ndef top{i}():\n    return leaf{i}()\n")));
    }
    v
}

/// Build the tree, apply `edit`, build again incrementally, and build the same tree fresh with a full resolve: the hashes and the
/// communities must be equal. Returns the incremental report.
fn edit_and_compare(edit: impl Fn(&Path)) -> comparegraph::application::build::BuildReport {
    let files = base_files();
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, c)| (*p, c.as_str())).collect();
    let d = tree(&refs);
    let w = ws(d.path());
    build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    edit(d.path());
    let inc = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    let fresh = ws(d.path());
    let full = build_repo(
        fresh.path(),
        "r",
        "reference",
        &BuildOptions {
            full: true,
            ..BuildOptions::default()
        },
    )
    .unwrap();
    assert_eq!(inc.store_hash, full.store_hash, "incremental differs from a full build");
    assert_eq!((inc.nodes, inc.edges), (full.nodes, full.edges));
    assert_eq!(communities_of(w.path(), inc.run), communities_of(fresh.path(), full.run));
    inc
}

#[test]
fn an_unrelated_edit_copies_the_other_files_edges_and_equals_a_full_build() {
    let r = edit_and_compare(|d| std::fs::write(d.join("u3.py"), "def leaf2():\n    return 22\n\ndef top2():\n    return leaf2()\n# edited\n").unwrap());
    assert!(r.edges_copied > 0, "no edge was copied");
    assert!(!r.limitation_items.iter().any(|l| l.code == "incremental_fallback"));
}

#[test]
fn an_edit_that_changes_an_importers_edge_still_equals_a_full_build() {
    // b.py gains `extra`: a.py's import of b and f.py's name-only call `extra()` (a file that imports nothing of b) both change
    let r = edit_and_compare(|d| std::fs::write(d.join("b.py"), B2).unwrap());
    assert!(r.edges_copied > 0);
    assert!(!r.limitation_items.iter().any(|l| l.code == "incremental_fallback"));
}

#[test]
fn a_removed_file_equals_a_full_build() {
    let r = edit_and_compare(|d| std::fs::remove_file(d.join("b.py")).unwrap());
    assert!(r.edges_copied > 0);
}

#[test]
fn an_added_file_that_an_import_now_resolves_to_equals_a_full_build() {
    edit_and_compare(|d| {
        std::fs::write(d.join("a.py"), "from newmod import thing\nfrom b import helper\n\ndef main():\n    return helper() + thing()\n").unwrap();
        std::fs::write(d.join("newmod.py"), "def thing():\n    return 3\n").unwrap();
    });
}

#[test]
fn a_changed_package_file_falls_back_to_a_full_resolve_and_says_so() {
    let r = edit_and_compare(|d| {
        std::fs::create_dir_all(d.join("pkg")).unwrap();
        std::fs::write(d.join("pkg/__init__.py"), "from .impl import run\n").unwrap();
        std::fs::write(d.join("pkg/impl.py"), "def run():\n    return 1\n").unwrap();
    });
    let l = r.limitation_items.iter().find(|l| l.code == "incremental_fallback").expect("no incremental_fallback limitation");
    assert!(l.detail.contains("__init__.py"), "{}", l.detail);
    assert_eq!(r.edges_copied, 0);
}

#[test]
fn a_change_touching_most_files_falls_back() {
    let r = edit_and_compare(|d| {
        for n in ["u1.py", "u2.py", "u3.py", "u4.py", "u5.py", "u6.py", "u7.py"] {
            std::fs::write(d.join(n), "def changed():\n    return 0\n").unwrap();
        }
    });
    assert!(r.limitation_items.iter().any(|l| l.code == "incremental_fallback"));
}

// ---- review round: relative imports with no identifier, and the base run ---------------------------------------------------

fn compare_after(v1: &[(&str, &str)], v2: &[(&str, &str)]) -> comparegraph::application::build::BuildReport {
    let d = tree(v1);
    let w = ws(d.path());
    build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    for (p, c) in v2 {
        let full = d.path().join(p);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, c).unwrap();
    }
    let inc = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    let fresh = ws(d.path());
    let full = build_repo(fresh.path(), "r", "reference", &BuildOptions { full: true, ..BuildOptions::default() }).unwrap();
    assert_eq!(inc.store_hash, full.store_hash, "incremental differs from a full build");
    inc
}
const PAD: [(&str, &str); 6] = [
    ("p1.ts", "export function p1() { return 1; }\n"),
    ("p2.ts", "export function p2() { return 2; }\n"),
    ("p3.ts", "export function p3() { return 3; }\n"),
    ("p4.ts", "export function p4() { return 4; }\n"),
    ("p5.ts", "export function p5() { return 5; }\n"),
    ("p6.ts", "export function p6() { return 6; }\n"),
];
fn with_pad<'a>(mut v: Vec<(&'a str, &'a str)>) -> Vec<(&'a str, &'a str)> {
    v.extend(PAD);
    v
}

#[test]
fn an_added_index_file_that_a_dot_only_import_now_resolves_to_equals_a_full_build() {
    // w1: `import * as ns from '.'` in src/u.ts, then src/index.ts appears
    compare_after(
        &with_pad(vec![("src/u.ts", "import * as ns from '.';\nexport const k = ns;\n")]),
        &[("src/index.ts", "export function zzz(): number { return 1; }\n")],
    );
}

#[test]
fn an_added_index_file_that_a_parent_dot_import_now_resolves_to_equals_a_full_build() {
    // w2: `import '..'` in src/sub/u.ts, then src/index.ts appears
    compare_after(
        &with_pad(vec![("src/sub/u.ts", "import '..';\nexport const k = 1;\n")]),
        &[("src/index.ts", "export function zzz(): number { return 1; }\n")],
    );
}

#[test]
fn a_removed_index_file_that_a_dot_only_import_resolved_to_equals_a_full_build() {
    let d = tree(&with_pad(vec![
        ("src/u.ts", "import * as ns from '.';\nexport const k = ns;\n"),
        ("src/index.ts", "export function zzz(): number { return 1; }\n"),
    ]));
    let w = ws(d.path());
    build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    std::fs::remove_file(d.path().join("src/index.ts")).unwrap();
    let inc = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    let fresh = ws(d.path());
    let full = build_repo(fresh.path(), "r", "reference", &BuildOptions { full: true, ..BuildOptions::default() }).unwrap();
    assert_eq!(inc.store_hash, full.store_hash);
}

#[test]
fn a_run_of_another_provider_between_does_not_disable_the_saving_or_the_reuse() {
    use comparegraph::domain::{Run, RunStatus};
    let files = base_files();
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, c)| (*p, c.as_str())).collect();
    let d = tree(&refs);
    let w = ws(d.path());
    let first = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    {
        let store = comparegraph::store::Store::open(w.path()).unwrap();
        let repo = store.repo_by_name("r").unwrap().unwrap();
        let id = store
            .insert_run(&Run {
                id: None,
                repo: repo.id.unwrap(),
                provider: "external".into(),
                provider_version: "9".into(),
                started_at: comparegraph::store::now_iso(),
                seconds: 0.0,
                status: RunStatus::Running,
                limitations: vec![],
                store_hash: None,
            })
            .unwrap();
        store.finish_run(id, RunStatus::Done, 0.1, &[], Some("x")).unwrap();
    }
    let again = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    assert!(again.reused && again.run == first.run, "an interleaved external run hid the reference run");
    std::fs::write(d.path().join("u3.py"), "def leaf2():\n    return 22\n\ndef top2():\n    return leaf2()\n# edited\n").unwrap();
    let inc = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    assert!(inc.edges_copied > 0, "no edge was copied with an external run in between");
}

#[test]
fn a_test_files_module_level_call_is_copied_as_a_tests_edge() {
    // the `tests` keep predicate: a converted module-level call (source = the file node, target a symbol) is copied
    let v1 = [
        ("lib.py", "def helper():\n    return 1\n"),
        ("test_lib.py", "from lib import helper\n\nhelper()\n"),
        ("o1.py", "def o1():\n    return 1\n"),
        ("o2.py", "def o2():\n    return 2\n"),
        ("o3.py", "def o3():\n    return 3\n"),
    ];
    let r = compare_after(&v1, &[("o2.py", "def o2():\n    return 22\n")]);
    assert!(r.edges_copied > 0);
}

// ---- a changed re-export barrel -------------------------------------------------------------------------------------------

const BARREL_PAD: [(&str, &str); 4] = [
    ("m1.ts", "export function m1() { return 1; }\n"),
    ("m2.ts", "export function m2() { return 2; }\n"),
    ("m3.ts", "export function m3() { return 3; }\n"),
    ("m4.ts", "export function m4() { return 4; }\n"),
];
fn barrel_tree() -> Vec<(&'static str, &'static str)> {
    let mut v = vec![
        ("src/index.ts", "export { f } from './x';\n"),
        ("src/x.ts", "export function f() { return 1; }\n"),
        ("src/y.ts", "export function f() { return 2; }\n"),
        ("src/use.ts", "import { f } from '.';\nexport function g() { return f(); }\n"),
    ];
    v.extend(BARREL_PAD);
    v
}

#[test]
fn a_changed_re_export_barrel_falls_back_to_a_full_resolve() {
    let r = compare_after(&barrel_tree(), &[("src/index.ts", "export { f } from './y';\n")]);
    let l = r.limitation_items.iter().find(|l| l.code == "incremental_fallback").expect("no incremental_fallback");
    assert!(l.detail.contains("src/index.ts"), "{}", l.detail);
    assert_eq!(r.edges_copied, 0);
}

#[test]
fn a_removed_index_file_falls_back() {
    let d = tree(&barrel_tree());
    let w = ws(d.path());
    build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    std::fs::remove_file(d.path().join("src/index.ts")).unwrap();
    let inc = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    assert!(inc.limitation_items.iter().any(|l| l.code == "incremental_fallback" && l.detail.contains("src/index.ts")));
}

#[test]
fn a_changed_non_barrel_file_still_takes_the_partial_path() {
    let r = compare_after(&barrel_tree(), &[("m2.ts", "export function m2() { return 22; }\n")]);
    assert!(r.edges_copied > 0);
    assert!(!r.limitation_items.iter().any(|l| l.code == "incremental_fallback"));
}

// ---- Go: the incremental build passes the same go.mod module table as a full build ---------------------------------------

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), t).unwrap();
        }
    }
}

#[test]
fn a_go_edit_in_a_module_equals_a_full_build() {
    let d = tempfile::tempdir().unwrap();
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/go"), d.path());
    assert!(d.path().join("go.mod").exists());
    let w = ws(d.path());
    build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    let util = d.path().join("util.go");
    let mut text = std::fs::read_to_string(&util).unwrap();
    text.push_str("\nfunc ZzAdded() int {\n\treturn 1\n}\n");
    std::fs::write(&util, text).unwrap();
    let inc = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    let fresh = ws(d.path());
    let full = build_repo(fresh.path(), "r", "reference", &BuildOptions { full: true, ..BuildOptions::default() }).unwrap();
    assert_eq!(inc.store_hash, full.store_hash, "go: incremental differs from a full build");
    eprintln!("go hashes inc={} full={} copied={} fallback={:?}", inc.store_hash, full.store_hash, inc.edges_copied, inc.limitation_items.iter().filter(|l| l.code == "incremental_fallback").collect::<Vec<_>>());
}

// ---- manifests that resolution reads but that are not source: go.mod, Cargo.toml -----------------------------------------

#[test]
fn a_go_mod_only_edit_is_a_real_run_that_equals_a_full_build() {
    let d = tempfile::tempdir().unwrap();
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/go"), d.path());
    let w = ws(d.path());
    let first = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    std::fs::write(d.path().join("go.mod"), "module other\n\ngo 1.21\n").unwrap();
    let second = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    assert!(!second.reused, "a go.mod edit was treated as a no-op");
    assert_ne!(second.run, first.run);
    let fresh = ws(d.path());
    let full = build_repo(fresh.path(), "r", "reference", &BuildOptions { full: true, ..BuildOptions::default() }).unwrap();
    assert_eq!(second.store_hash, full.store_hash);
    eprintln!("go.mod edit: first={} second={} full={}", first.store_hash, second.store_hash, full.store_hash);
    assert_ne!(first.store_hash, full.store_hash, "the edit did not change resolution; pick one that does");
}

#[test]
fn a_go_mod_edit_together_with_a_source_edit_falls_back_and_equals_a_full_build() {
    let d = tempfile::tempdir().unwrap();
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/go"), d.path());
    let w = ws(d.path());
    build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    std::fs::write(d.path().join("go.mod"), "module other\n\ngo 1.21\n").unwrap();
    let util = d.path().join("util.go");
    let mut text = std::fs::read_to_string(&util).unwrap();
    text.push_str("\nfunc ZzAdded2() int {\n\treturn 1\n}\n");
    std::fs::write(&util, text).unwrap();
    let inc = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    let l = inc.limitation_items.iter().find(|l| l.code == "incremental_fallback").expect("no incremental_fallback");
    assert!(l.detail.contains("go.mod"), "{}", l.detail);
    assert_eq!(inc.edges_copied, 0);
    let fresh = ws(d.path());
    let full = build_repo(fresh.path(), "r", "reference", &BuildOptions { full: true, ..BuildOptions::default() }).unwrap();
    assert_eq!(inc.store_hash, full.store_hash);
}

#[test]
fn a_cargo_toml_only_edit_is_a_real_run_that_equals_a_full_build() {
    let t = tree(&[
        ("Cargo.toml", "[package]\nname = \"a\"\nversion = \"0.1.0\"\n"),
        ("src/lib.rs", "pub fn f() -> i32 { 1 }\n"),
        ("src/main.rs", "fn main() { let _ = a::f(); }\n"),
    ]);
    let w = ws(t.path());
    let first = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    std::fs::write(t.path().join("Cargo.toml"), "[package]\nname = \"b\"\nversion = \"0.1.0\"\n").unwrap();
    let second = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    assert!(!second.reused, "a Cargo.toml edit was treated as a no-op");
    assert_ne!(second.run, first.run);
    let fresh = ws(t.path());
    let full = build_repo(fresh.path(), "r", "reference", &BuildOptions { full: true, ..BuildOptions::default() }).unwrap();
    assert_eq!(second.store_hash, full.store_hash);
}

/// A4: 50 toml files each overshoot by one line (`clamped_line_end`); the run keeps ONE limitation with the count and the first
/// five paths, the envelope's limitations stay small, and the store hash does not depend on limitations.
#[test]
fn repeated_limitations_collapse_into_one_with_a_count() {
    let files: Vec<(String, String)> = (0..50).map(|i| (format!("conf/f{i:02}.toml"), "[plugins]\na = 1\nb = 2\n".to_string())).collect();
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    let d = tree(&refs);
    let w = ws(d.path());
    let r = build_repo(w.path(), "r", "reference", &BuildOptions::default()).unwrap();
    let clamped: Vec<_> = r.limitation_items.iter().filter(|l| l.code == "clamped_line_end").collect();
    assert_eq!(clamped.len(), 1, "{:?}", r.limitation_items);
    assert!(clamped[0].detail.starts_with("50 occurrences; first: "), "{}", clamped[0].detail);
    assert_eq!(clamped[0].detail.matches(".toml").count(), 5, "{}", clamped[0].detail);
    assert!(clamped[0].detail.ends_with(", …"), "{}", clamped[0].detail);
    assert_eq!(r.limitations, r.limitation_items.len());
    assert!(serde_json::to_string(&r.limitation_items).unwrap().len() < 8 * 1024);
    // the stored run carries the collapsed list too
    let store = comparegraph::store::Store::open(w.path()).unwrap();
    let run = store.run(r.run).unwrap().unwrap();
    assert_eq!(run.limitations.iter().filter(|l| l.code == "clamped_line_end").count(), 1);
    // limitations are not part of store_hash
    let full = build_repo(w.path(), "r", "reference", &BuildOptions { full: true, ..BuildOptions::default() }).unwrap();
    assert_eq!(full.store_hash, r.store_hash);
}
