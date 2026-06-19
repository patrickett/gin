//! Package resolution against canonical `modules/gin_core` layout on disk.

use resolve::{find_import_suggestions, find_public_def};
use test_fixtures::TempPackage;

const INT_GIN: &str = r#"--- The 8-bit signed integer type.
SignedTinyInt is in -128...127
--- The 16-bit signed integer type.
SignedSmallInt is in -32768...32767
--- The 32-bit signed integer type.
SignedInt is in -2147483648...2147483647
--- The 64-bit signed integer type.
SignedBigInt is in -9223372036854775808...9223372036854775807
--- The 128-bit signed integer type.
SignedLargeInt is in -170141183460469231731687303715884105728...170141183460469231731687303715884105727
--- The 8-bit unsigned integer type.
TinyInt is in 0...255
--- The 16-bit unsigned integer type.
SmallInt is in 0...65535
--- The 32-bit unsigned integer type.
Int is in 0...4294967295
--- The 64-bit unsigned integer type.
BigInt is in 0...18446744073709551615
--- The 128-bit unsigned integer type.
LargeInt is in 0...340282366920938463463374607431768211455


--- Alias for the 8-bit unsigned integer type.
Byte is TinyInt
"#;

const STRING_GIN: &str = r#"use core.primitive.(Byte, List)

--- Canonical `String` type used for printing/codegen.
String has (bytes List(Byte))

ToString has (to_string String)
"#;

const BOOL_GIN: &str = r#"use core.Happy
use core.ToString

--- `Bool` represents a value, which could only be either `True` or `False`.
---
--- ## Basic usage
---
--- `Bool` implements various traits, such as BitAnd, BitOr, Not, etc.,
--- which allow us to perform boolean operations using &, | and !.
---
--- `if` requires a `Bool` value as its conditional.
Bool is True or False
Bool.Happy(value: Bool.True)
Bool.ToString(to_string: when self then 'true' else 'false')


false := Bool.False
true  := Bool.True


-- is_empty(v Maybe(x)) Bool:
--     if v is None return True
-- return False
"#;

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
Bool is True or False and\n\
    has ToString(to_string: when self then 'true' else 'false')\n\
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
