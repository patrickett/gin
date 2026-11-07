//! A record may hold compile-time `Text` (register names, banks): it is constructible and
//! readable while compiling, and has no machine form (`lower_next.rs` skips functions of it).
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

const REGISTER: &str = "\
#unrestricted
Register has
    architecture Text
    bank Text
    index in 0 to 255

";

#[test]
fn a_record_with_text_fields_is_built_by_a_nullary_function() {
    check(&format!(
        "{REGISTER}x0 Register: Register(architecture: 'aarch64', bank: 'general', index: 0)\n"
    ))
    .unwrap();
    // Double-quoted text without holes is the same text.
    check(&format!(
        "{REGISTER}x0 Register: Register(architecture: \"aarch64\", bank: \"general\", index: 0)\n"
    ))
    .unwrap();
}

#[test]
fn its_text_is_read_while_compiling() {
    check(&format!(
        "{REGISTER}is_general(r Register) in 0 to 1:\n    return when r.bank is\n        'general' then 1\n        else 0\n"
    ))
    .unwrap();
}

#[test]
fn a_text_field_takes_text_not_a_number() {
    assert_eq!(
        check(&format!("{REGISTER}x0 Register: Register(architecture: 1, bank: 'general', index: 0)\n")),
        Err(SourceCheckError::Type)
    );
}

#[test]
fn a_field_must_be_given() {
    assert!(check(&format!("{REGISTER}x0 Register: Register(architecture: 'a', index: 0)\n")).is_err());
}
