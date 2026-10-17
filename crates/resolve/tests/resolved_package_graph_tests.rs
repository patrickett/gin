use resolve::ResolvedPackageGraph;
use test_fixtures::TempPackage;

#[test]
fn minimum_manifests_share_local_packages_and_allow_requirement_cycles() {
    let package = TempPackage::new("minimum_local_cycle");
    package.write(
        "mod.flask",
        "#settings\nmodule: 'app'\nversion: 0.1.0\n#dependencies\na: './a'\nb: './b'\n",
    );
    package.write(
        "a/mod.flask",
        "#settings\nmodule: 'first'\nversion: 0.1.0\n#dependencies\nshared: '../shared'\n",
    );
    package.write(
        "b/mod.flask",
        "#settings\nmodule: 'second'\nversion: 0.1.0\n#dependencies\nshared: '../shared'\n",
    );
    package.write(
        "shared/mod.flask",
        "#settings\nmodule: 'shared'\nversion: 0.1.0\n#dependencies\napp: '../'\n",
    );
    let graph = ResolvedPackageGraph::discover(package.path());
    assert!(graph.diagnostics.is_empty(), "{:?}", graph.diagnostics);
    assert_eq!(graph.nodes.len(), 4);
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.config.name() == "shared")
            .count(),
        1
    );
}
