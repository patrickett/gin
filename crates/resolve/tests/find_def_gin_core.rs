//! Package resolution against canonical `modules/gin_core` layout on disk.

use resolve::{find_import_suggestions, find_public_def};
use test_fixtures::TempPackage;
use test_fixtures::gin_core::{BOOL_GIN, INT_GIN, STRING_GIN};

#[test]
fn find_int_and_to_string_in_package_fixture() {
    let pkg = TempPackage::new("find_def_gin_core");
    pkg.write_flask("core");
    pkg.write("primitive/int.gin", INT_GIN);
    pkg.write("string/string.gin", STRING_GIN);
    pkg.write("primitive/bool.gin", BOOL_GIN);

    let int = find_public_def(pkg.path(), "Int", true);
    assert!(int.is_some(), "Int should be found in package fixture");
    let to_string = find_public_def(pkg.path(), "ToString", true);
    assert!(
        to_string.is_some(),
        "ToString should be found in package fixture"
    );
}

#[test]
fn bool_fixture_suggests_import_for_to_string() {
    let pkg = TempPackage::new("bool_suggest");
    pkg.write_flask("core");
    pkg.write("string/string.gin", STRING_GIN);
    const BOOL_WITHOUT_IMPORT: &str = "\
Bool is True or False\n\
Bool has ToString\n\
    ToString.to_string: when self then 'true' else 'false'\n\
";
    let bool_gin = pkg.write("primitive/bool.gin", BOOL_WITHOUT_IMPORT);
    let suggestions = find_import_suggestions(&bool_gin, "ToString", BOOL_WITHOUT_IMPORT);
    assert!(
        suggestions.iter().any(|s| {
            s.use_line.contains("ToString")
                && (s.use_line.contains("string") || s.use_line.contains("String"))
        }),
        "expected local import for ToString, got {:?}",
        suggestions
    );
}
