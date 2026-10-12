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
