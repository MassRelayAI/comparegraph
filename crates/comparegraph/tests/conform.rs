use comparegraph::application::conform::conform;
#[test]
fn reference_provider_passes_the_two_fixtures() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let rep = conform("reference", &root).unwrap();
    for f in &rep.fixtures {
        eprintln!(
            "{} nodes p={:.2} r={:.2} edges p={:.2} r={:.2}",
            f.name, f.nodes.precision, f.nodes.recall, f.edges.precision, f.edges.recall
        );
    }
    assert!(rep.passed, "{:?}", rep);
}
