mod support;

use support::{check_with, expect_flaw_code, expect_no_flaws};

const PRELUDE: &str = r#"
Byte is in 0 to 255
Byte has finish(self): return
Length is in 0 to 255
Length has finish(self): return
Slice(element) has
    length Length
    data ref [element; length]
    finish(self):
        return
"#;

#[test]
fn runtime_length_reference_supports_iteration() {
    let typed = check_with(
        PRELUDE,
        r#"
visit(n Length, bytes ref [Byte; n]):
    for value in bytes
        observed is in Byte: value
    return
"#,
    );
    expect_no_flaws(&typed);
}

#[test]
fn dependent_record_rejects_an_oversized_backing_claim() {
    let typed = check_with(
        PRELUDE,
        r#"
run:
    source [Byte; 4]: [10, 20, 30, 40]
    slice: Slice(length: 5 as Length, data: source)
    slice.finish
    return
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-length");
}

#[test]
fn dependent_record_accepts_matching_backing_length() {
    let typed = check_with(
        PRELUDE,
        r#"
run:
    source [Byte; 4]: [10, 20, 30, 40]
    slice: Slice(data: source, length: 4 as Length)
    slice.finish
    return
"#,
    );
    expect_no_flaws(&typed);
}

#[test]
fn dependent_record_accepts_a_symbolically_matching_reference() {
    let typed = check_with(
        PRELUDE,
        r#"
borrow(n Length, source ref [Byte; n]) Slice(Byte): return Slice(length: n, data: source)
"#,
    );
    expect_no_flaws(&typed);
}

#[test]
fn call_rejects_a_length_that_does_not_match_its_reference() {
    let typed = check_with(
        PRELUDE,
        r#"
peek(n Length, source ref [Byte; n]) Byte: return source.[0]
run Byte:
    source [Byte; 4]: [10, 20, 30, 40]
    return peek(5, source)
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-length");
}

#[test]
fn dependent_record_length_cannot_be_replaced_independently() {
    let typed = check_with(
        PRELUDE,
        r#"
replace(previous Length, next Length) Length: return next
bad(slice mut Slice(Byte)):
    slice.length:: replace(slice.length, 5)
    return
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-mutation");
}

#[test]
fn dependent_record_length_cannot_be_mutably_borrowed() {
    let typed = check_with(
        PRELUDE,
        r#"
change(length mut Length): return
bad(slice mut Slice(Byte)):
    change(slice.length)
    return
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-mutation");
}

#[test]
fn unproven_constructor_length_is_rejected() {
    let typed = check_with(
        PRELUDE,
        r#"
borrow(source ref [Byte; 4], n Length) Slice(Byte): return Slice(length: n, data: source)
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-length");
}

#[test]
fn dependent_parameter_length_cannot_be_rebound() {
    let typed = check_with(
        PRELUDE,
        r#"
replace(previous Length, next Length) Length: return next
bad(n Length, source ref [Byte; n]):
    n:: replace(n, 5)
    return
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-mutation");
}

#[test]
fn dependent_parameter_length_cannot_be_mutably_borrowed() {
    let typed = check_with(
        PRELUDE,
        r#"
change(length mut Length): return
bad(n Length, source ref [Byte; n]):
    change(n)
    return
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-mutation");
}

#[test]
fn dependent_record_storage_supports_iteration() {
    let typed = check_with(
        PRELUDE,
        r#"
visit(slice ref Slice(Byte)):
    for value in slice.data
        observed is in Byte: value
    return
"#,
    );
    expect_no_flaws(&typed);
}

#[test]
fn record_constructor_checks_values_in_argument_evaluation_order() {
    let typed = check_with(
        PRELUDE,
        r#"
replace(previous Length, next Length) Length: return next
change(n mut Length, source ref [Byte; 4]) ref [Byte; 4]:
    n:: replace(deref n, 4)
    return source
run:
    source [Byte; 4]: [10, 20, 30, 40]
    n Length: 5
    slice: Slice(length: n, data: change(n, source))
    slice.finish
    return
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-length");
}

#[test]
fn borrowed_dependent_record_cannot_escape_local_storage() {
    let typed = check_with(
        PRELUDE,
        r#"
escape Slice(Byte):
    source [Byte; 4]: [10, 20, 30, 40]
    return Slice(length: 4 as Length, data: source)
"#,
    );
    expect_flaw_code(&typed, "type-reference-return-local-target");
}

#[test]
fn matching_lengths_do_not_accept_different_element_types() {
    let typed = check_with(
        PRELUDE,
        r#"
peek(n Length, source ref [Byte; n]) Byte: return source.[0]
run Byte:
    source [Length; 4]: [1, 2, 3, 4]
    return peek(4, source)
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-element");
}

#[test]
fn dependent_parameter_length_cannot_be_shadowed() {
    let typed = check_with(
        PRELUDE,
        r#"
bad(n Length, source ref [Byte; n]) Byte:
    n Length: 5
    return source.[4]
"#,
    );
    expect_flaw_code(&typed, "type-binding-shadows-visible-name");
}

#[test]
fn a_record_cannot_be_shadowed_while_its_dependent_storage_is_borrowed() {
    let typed = check_with(
        PRELUDE,
        r#"
bad(slice ref Slice(Byte), other ref Slice(Byte)) Byte:
    data: slice.data
    slice: other
    return data.[4]
"#,
    );
    expect_flaw_code(&typed, "type-binding-shadows-visible-name");
}

#[test]
fn mutable_dependent_iteration_allows_access_through_the_current_element() {
    let typed = check_with(
        PRELUDE,
        r#"
replace(previous Byte, next Byte) Byte: return next
set(value mut Byte):
    value:: replace(deref value, 7)
    return
visit(n Length, source mut [Byte; n]):
    for value in source
        set(value)
    return
"#,
    );
    expect_no_flaws(&typed);
}

#[test]
fn mutable_dependent_iteration_rejects_access_through_the_source() {
    let typed = check_with(
        PRELUDE,
        r#"
read(value ref Byte) Byte: return deref value
bad(n Length, source mut [Byte; n]):
    for value in source
        read(source.[0])
    return
"#,
    );
    expect_flaw_code(&typed, "type-sequence-borrow-conflict");
}

#[test]
fn a_raw_pointer_does_not_establish_a_borrowed_array_extent() {
    let typed = check_with(
        PRELUDE,
        r#"
borrow(n Length, source ref @Byte) Slice(Byte): return Slice(length: n, data: source)
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-length");
}

#[test]
fn a_proven_runtime_length_can_construct_a_dependent_record() {
    let typed = check_with(
        PRELUDE,
        r#"
borrow(n Length and n = 4, source ref [Byte; 4]) Slice(Byte): return Slice(length: n, data: source)
"#,
    );
    expect_no_flaws(&typed);
}

#[test]
fn record_literal_cannot_bypass_the_dependent_length_check() {
    let typed = check_with(
        PRELUDE,
        r#"
bad(source ref [Byte; 4]) Slice(Byte): return [length: 5 as Length, data: source]
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-length");
}

#[test]
fn record_literal_accepts_matching_backing_length() {
    let typed = check_with(
        PRELUDE,
        r#"
borrow(source ref [Byte; 4]) Slice(Byte): return [length: 4 as Length, data: source]
"#,
    );
    expect_no_flaws(&typed);
}

#[test]
fn a_return_annotation_cannot_inflate_a_reference_length() {
    let typed = check_with(
        PRELUDE,
        r#"
bad(n Length, source ref [Byte; 4]) ref [Byte; n]: return source
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-length");
}

#[test]
fn a_return_annotation_cannot_inflate_a_new_borrow() {
    let typed = check_with(
        PRELUDE,
        r#"
bad(n Length, source ref [Byte; 4]) ref [Byte; n]: return ref deref source
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-length");
}

#[test]
fn a_binding_annotation_cannot_inflate_a_reference_length() {
    let typed = check_with(
        PRELUDE,
        r#"
bad(n Length, source ref [Byte; 4]):
    ref saved [Byte; n]: source
    return
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-length");
}

#[test]
fn a_return_annotation_accepts_a_matching_runtime_reference() {
    let typed = check_with(
        PRELUDE,
        r#"
borrow(n Length, source ref [Byte; n]) ref [Byte; n]: return source
"#,
    );
    expect_no_flaws(&typed);
}

#[test]
fn a_return_annotation_accepts_a_proven_runtime_length() {
    let typed = check_with(
        PRELUDE,
        r#"
borrow(n Length and n = 4, source ref [Byte; 4]) ref [Byte; n]: return source
"#,
    );
    expect_no_flaws(&typed);
}

#[test]
fn a_record_literal_cannot_upgrade_shared_storage_to_mutable() {
    let typed = check_with(
        PRELUDE,
        r#"
MutableSlice(element) has
    length Length
    data mut [element; length]
    finish(self):
        return
bad(source ref [Byte; 4]) MutableSlice(Byte): return [length: 4 as Length, data: source]
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-permission");
}

#[test]
fn a_return_annotation_cannot_inflate_a_call_result() {
    let typed = check_with(
        PRELUDE,
        r#"
four(source ref [Byte; 4]) ref [Byte; 4]: return source
bad(n Length, source ref [Byte; 4]) ref [Byte; n]: return four(source)
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-length");
}

#[test]
fn a_return_annotation_cannot_inflate_a_conditional_result() {
    let typed = check_with(
        PRELUDE,
        r#"
bad(n Length, source ref [Byte; 4]) ref [Byte; n]: return when n is
    0 then source
    else source
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-length");
}
