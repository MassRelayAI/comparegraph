// Run by hand: COMPAREGRAPH_CLONE=/abs/path/to/a/clone cargo test -p comparegraph --test real_clone -- --ignored --nocapture
use comparegraph::application::{add::add_repo, build::build_repo, check::check, export::export_json};
use comparegraph::providers::BuildOptions;
#[test]
#[ignore]
fn builds_a_real_clone_deterministically() {
    let Some(clone) = std::env::var_os("COMPAREGRAPH_CLONE") else {
        eprintln!("set COMPAREGRAPH_CLONE");
        return;
    };
    let ws = tempfile::tempdir().unwrap();
    add_repo(ws.path(), std::path::Path::new(&clone), Some("real"), false).unwrap();
    let a = build_repo(ws.path(), "real", "reference", &BuildOptions::default()).unwrap();
    let b = build_repo(ws.path(), "real", "reference", &BuildOptions::default()).unwrap();
    eprintln!(
        "files_parsed={} nodes={} edges={} limitations={} communities={} seconds={:.1}",
        a.files_parsed, a.nodes, a.edges, a.limitations, a.communities, a.seconds
    );
    assert_eq!(a.store_hash, b.store_hash);
    assert!(a.nodes > 100);
    assert!(check(ws.path()).unwrap().ok());
    let j = export_json(ws.path(), Some("real")).unwrap();
    assert_eq!(j["repos"][0]["nodes"].as_array().unwrap().len(), a.nodes);
}
