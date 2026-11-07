//! `Name is <type>` is a transparent alias (local or imported); a domain may take tag
//! parameters (`Count(unit)`); a number and an address convert to each other unchecked.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

fn check(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module(name, &source, dependencies, Limits::default())
}

const BASE: &str = "#unrestricted\nByte is in 0 to 255\n#unrestricted\nWord is in 0 to 65535\n";

#[test]
fn an_alias_is_the_type_it_names() {
    let text = format!("{BASE}BytePointer is @Byte\nid(p BytePointer) @Byte: return p\nback(p @Byte) BytePointer: return p\n");
    check("a", &text, &[]).unwrap();
}

#[test]
fn an_alias_is_exported_and_imported() {
    let library = check("lib.types", &format!("{BASE}BytePointer is @Byte\n"), &[]).unwrap();
    check(
        "app",
        "use lib.types BytePointer, Byte\nid(p BytePointer) @Byte: return p\n",
        &[&library],
    )
    .unwrap();
    // The alias is its own name only where it is imported.
    assert!(
        check("app", "id(p BytePointer) BytePointer: return p\n", &[&library]).is_err()
    );
}

#[test]
fn an_alias_of_a_generic_type_names_one_instance() {
    let text = format!("{BASE}Count(unit) is in 0 to 1000\nCount(unit) has\n    release(self): return\n\nByteCount is Count(Byte)\nmake() ByteCount: return 1 as ByteCount\nsame() Count(Byte): return make()\n");
    check("a", &text, &[]).unwrap();
}

#[test]
fn a_cycle_of_aliases_is_refused() {
    let text = format!("{BASE}A is @B\nB is @A\n");
    assert!(check("a", &text, &[]).is_err());
}

#[test]
fn domains_of_different_tags_are_different_types() {
    let counts = format!("{BASE}Count(unit) is in 0 to 1000\nCount(unit) has\n    release(self): return\n\n");
    check("a", &format!("{counts}one() Count(Byte): return 1 as Count(Byte)\n"), &[]).unwrap();
    assert_eq!(
        check(
            "a",
            &format!("{counts}one() Count(Byte): return 1 as Count(Byte)\nmix() Count(Word): return one()\n"),
            &[]
        )
        .err(),
        Some(SourceCheckError::Type)
    );
}

#[test]
fn a_number_and_an_address_convert_to_each_other() {
    let text = format!("{BASE}#unrestricted\nSize is in 0 to 18446744073709551615\n\nto_address(n Size) @Byte: return #unchecked(n as @Byte)\nto_number(p @Byte) Size: return #unchecked(p as Size)\nfrom_plain(n in 0 to 100) @Byte: return #unchecked(n as @Byte)\n");
    check("a", &text, &[]).unwrap();
}

#[test]
fn a_linear_domain_cannot_be_cast_to_an_address() {
    let text = format!("{BASE}Hidden is in 0 to 100\nHidden has\n    release(self): return\n\nto_address(n Hidden) @Byte: return #unchecked(n as @Byte)\n");
    assert!(check("a", &text, &[]).is_err());
}
