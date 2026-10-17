use resolve::resolve_dep_hover;
use test_fixtures::TempPackage;

#[test]
fn hover_dep_root_shows_metadata() {
    let pkg = TempPackage::new("hover_dep_root");
    pkg.write(
        "mod.flask",
        r#"#settings
module: "root"
version: 0.0.0

#dependencies
core: "./core"
"#,
    );
    pkg.write(
        "core/mod.flask",
        r#"#settings
module: "core"
version: 1.0.0
description: "Core types and utilities"

#dependencies
"#,
    );
    let base_path = pkg.write("main.gin", "main:\n    return 0\n");

    let text = resolve_dep_hover(&base_path, "core").expect("dep root hover");
    assert_eq!(
        text,
        "```gin\ncore\n```\n\n---\n\nCore types and utilities\n\n---\n\nversion = 1.0.0"
    );
}

#[test]
fn hover_dep_root_unknown_dep_returns_none() {
    let pkg = TempPackage::new("hover_dep_unknown");
    pkg.write(
        "mod.flask",
        r#"#settings
module: "root"
version: 0.0.0

#dependencies
"#,
    );
    let base_path = pkg.write("main.gin", "main:\n    return 0\n");

    let hover = resolve_dep_hover(&base_path, "nonexistent_dep_xyz");
    assert!(hover.is_none(), "expected None for undeclared dependency");
}
