use ginc::cli::Profile;

#[test]
fn generic_identity_preserves_result_family_labels_at_runtime() {
    let source = r#"less(left is in 0 to 511, right is in 0 to 511) True thus left < right or False thus left >= right:
    when left < right then
        return True
    return False
identity(value x): return value
#entry
main:
    for value in 0 to 511
        expected: when value < 256 then 1 else 0
        actual: when identity(less(value, 256)) is True then 1 else 0
        when not (actual = expected) then
            return 1
    return 0
"#;
    for profile in [Profile::Debug, Profile::Release] {
        crate::assert_program(source, "generic_result_family_labels", profile);
    }
}
