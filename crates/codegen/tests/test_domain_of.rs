mod support;

#[test]
fn library_range_bound_in_integer_method_generates_code() {
    let source = r#"
Bounds(lo to hi) is lo to hi
Bounds(lo to hi) has
    lower: lo
    upper: hi
Word is in -128 to 127
Word has
    minimum() Self: return Bounds(domain of Self).lower as Self
minimum() Word: return Word.minimum()
"#;
    let (verified, diagnostics) = support::codegen_verifies(source, "domain_of.gin");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(verified);
}

#[test]
fn stored_domain_is_erased_after_library_projection() {
    let source = r#"
Bounds(lo to hi) is lo to hi
Bounds(lo to hi) has
    lower: lo
Word is in -128 to 127
values: domain of Word
minimum() Word: return Bounds(values).lower as Word
"#;
    let (verified, diagnostics) = support::codegen_verifies(source, "stored_domain.gin");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(verified);
}
