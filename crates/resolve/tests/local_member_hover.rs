//! Hover for `use '../string/'.ToString` and imported `ToString` in the file body.

use ast::source::SourceExt;
use parser::query::SourceParseExt;
use resolve::{ImportTarget, resolve_import_at, resolve_local_symbol_hover};
use test_fixtures::TempPackage;

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

/// `primitive/bool.gin` in gin_core uses `use core.ToString`; these tests need the
/// quoted local-member form exercised by `ImportSource::LocalMember`.
fn bool_gin_with_local_string_import() -> String {
    BOOL_GIN.replacen("use core.ToString", "use '../string/'.ToString", 1)
}

fn import_target_at(source: &str, byte: usize) -> ImportTarget {
    let output = source.parse_source_full();
    resolve_import_at(&output.ast, source, byte)
        .unwrap_or_else(|| panic!("expected import target at byte {byte}"))
}

#[test]
fn local_member_import_resolves_on_use_line_and_has_clause() {
    let pkg = TempPackage::new("bool");
    pkg.write_flask("core");
    pkg.write("string/string.gin", STRING_GIN);
    pkg.write("primitive/bool.gin", &bool_gin_with_local_string_import());

    let bool_path = pkg.join("primitive/bool.gin");
    let source = std::fs::read_to_string(&bool_path).unwrap();

    let use_byte = source.find("ToString").expect("ToString on use line");
    let has_byte = source.rfind("ToString").expect("ToString in has clause");
    assert_ne!(use_byte, has_byte);

    match import_target_at(&source, use_byte) {
        ImportTarget::LocalBundleSymbol { local_path, symbol } => {
            assert_eq!(symbol, "ToString");
            assert_eq!(local_path, std::path::PathBuf::from("../string/"));
        }
        other => panic!("use line: expected LocalBundleSymbol, got {other:?}"),
    }

    match import_target_at(&source, has_byte) {
        ImportTarget::LocalBundleSymbol { symbol, .. } => assert_eq!(symbol, "ToString"),
        other => panic!("has clause: expected LocalBundleSymbol, got {other:?}"),
    }
}

#[test]
fn local_member_import_hover_matches_definition_in_string_module() {
    let pkg = TempPackage::new("hover");
    pkg.write_flask("core");
    let string_path = pkg.write("string/string.gin", STRING_GIN);
    pkg.write("primitive/bool.gin", &bool_gin_with_local_string_import());

    let bool_path = pkg.join("primitive/bool.gin");
    let bool_source = std::fs::read_to_string(&bool_path).unwrap();
    let string_source = STRING_GIN;

    let string_output = string_source.parse_source_full();
    let string_typed =
        typecheck::transform::transform_file(string_output.ast.clone(), typecheck::FileId(0));
    let def_byte = string_source
        .find("ToString")
        .expect("ToString in string.gin");
    let (line, character) = string_source.byte_offset_to_position(def_byte);
    let def_hover = string_typed
        .hover_at(&string_source, line, character)
        .expect("hover on ToString definition");

    for byte in [
        bool_source.find("ToString").unwrap(),
        bool_source.rfind("ToString").unwrap(),
    ] {
        let ImportTarget::LocalBundleSymbol { local_path, symbol } =
            import_target_at(&bool_source, byte)
        else {
            panic!("expected LocalBundleSymbol at byte {byte}");
        };
        let hover = resolve_local_symbol_hover(&bool_path, &local_path, &symbol, &|p| {
            resolve::ParsedFile::read(p)
        })
        .unwrap_or_else(|| panic!("hover at byte {byte}"));
        assert_eq!(
            hover, def_hover,
            "hover at byte {byte} must match ToString definition in string/string.gin"
        );
    }
    let _ = string_path;
}
