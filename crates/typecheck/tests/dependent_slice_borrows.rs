mod support;

use support::{check_with, expect_flaw_code, expect_no_flaws, flaw_codes};

const PRELUDE: &str = r#"
Byte is in 0 to 255
Byte has finish(self): return
Length is in 0 to 255
Length has finish(self): return
"#;

#[test]
fn prefix_has_the_requested_dependent_extent() {
    let typed = check_with(
        PRELUDE,
        r#"
prefix(n Length, source ref [Byte; 4]) ref [Byte; n]: return ref source.[0; n]
"#,
    );
    expect_no_flaws(&typed);
}

#[test]
fn mutable_prefix_preserves_permission() {
    let typed = check_with(
        PRELUDE,
        r#"
prefix(n Length, source mut [Byte; 4]) mut [Byte; n]: return mut source.[0; n]
"#,
    );
    expect_no_flaws(&typed);
}

#[test]
fn a_shared_source_cannot_supply_a_mutable_slice() {
    let typed = check_with(
        PRELUDE,
        r#"
prefix(n Length, source ref [Byte; 4]) mut [Byte; n]: return mut source.[0; n]
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-permission");
}

#[test]
fn a_slice_cannot_escape_local_backing_storage() {
    let typed = check_with(
        PRELUDE,
        r#"
escape(seed Byte) ref [Byte; 2]:
    source [Byte; 4]: [seed, 2, 3, 4]
    return ref source.[0; 2]
"#,
    );
    assert!(!typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn an_oversized_constant_prefix_is_rejected() {
    let typed = check_with(
        PRELUDE,
        r#"
prefix(source ref [Byte; 4]) ref [Byte; 5]: return ref source.[0; 5]
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-bounds");
}

#[test]
fn empty_tail_is_a_valid_borrow() {
    let typed = check_with(
        PRELUDE,
        r#"
tail(source ref [Byte; 4]) ref [Byte; 0]: return ref source.[4; 0]
"#,
    );
    expect_no_flaws(&typed);
}

#[test]
fn raw_pointer_does_not_establish_owner_lifetime() {
    let typed = check_with(
        PRELUDE,
        r#"
borrow(n Length, source @Byte) ref [Byte; n]: return ref source.[0; n]
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-borrow");
}

#[test]
fn owned_storage_borrow_is_tied_to_the_owner() {
    let typed = check_with(
        PRELUDE,
        r#"
Owner has
    address own Byte
    finish(self) Self: return self
borrow(n Length, owner ref Owner) ref [Byte; n]: return ref owner.address.[0; n]
"#,
    );
    expect_no_flaws(&typed);
}

#[test]
fn owned_storage_cannot_supply_mutable_slice_through_shared_owner() {
    let typed = check_with(
        PRELUDE,
        r#"
Owner has
    address own Byte
    finish(self) Self: return self
borrow(n Length, owner ref Owner) mut [Byte; n]: return mut owner.address.[0; n]
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-permission");
}

#[test]
fn a_slice_does_not_survive_consuming_its_owner() {
    let typed = check_with(
        PRELUDE,
        r#"
consume(source [Byte; 4]): return
bad(source [Byte; 4]) Byte:
    view: ref source.[0; 2]
    consume(source)
    return view.[0]
"#,
    );
    assert!(!typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn consuming_owned_elements_invalidates_existing_slices() {
    for extraction in [
        "take(owner mut Owner, index Length) Byte: return owner.address.[index]",
        "take(owner mut Owner, index Length) Byte:\n    value: owner.address.[index]\n    return value",
        "take(owner mut Owner, index Length): return discard(owner.address.[index])",
    ] {
        let typed = check_with(
            PRELUDE,
            &format!(
                "Owner has\n    address own Byte\n    finish(self) Self: return self\ndiscard(value Byte): return\n{extraction}\nbad(n Length, owner mut Owner) Byte:\n    view: ref owner.address.[0; n]\n    take(owner, 0)\n    return view.[0]\n"
            ),
        );
        assert!(
            flaw_codes(&typed).contains(&"type-use-of-invalidated-reference".to_string()),
            "{extraction}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn a_shared_record_cannot_expose_its_mutable_array_reference() {
    let typed = check_with(
        PRELUDE,
        r#"
View has
    len Length
    data mut [Byte; len]
    finish(self):
        return
bad(n Length, view ref View) mut [Byte; n]: return mut view.data.[0; n]
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-permission");
}

#[test]
fn an_owned_region_borrow_cannot_escape_an_owned_parameter() {
    let typed = check_with(
        PRELUDE,
        r#"
Owner has
    address own Byte
    finish(self) Self: return self
escape(n Length, owner Owner) ref [Byte; n]: return ref owner.address.[0; n]
"#,
    );
    assert!(!typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn a_negative_slice_start_is_rejected() {
    let typed = check_with(
        PRELUDE,
        r#"
bad(source ref [Byte; 4]) ref [Byte; 2]: return ref source.[-1; 2]
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-bounds");
}

#[test]
fn a_negative_slice_length_is_rejected() {
    let typed = check_with(
        PRELUDE,
        r#"
bad(source ref [Byte; 4]):
    view: ref source.[0; -1]
    return
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-bounds");
}

#[test]
fn a_bare_slice_cannot_copy_unsized_storage() {
    let typed = check_with(
        PRELUDE,
        r#"
bad(source ref [Byte; 4]):
    view: source.[0; 2]
    return
"#,
    );
    expect_flaw_code(&typed, "type-dependent-array-borrow");
}

#[test]
fn a_slice_does_not_survive_consumption_while_evaluating_its_length() {
    let typed = check_with(
        PRELUDE,
        r#"
consume(source [Byte; 4]) Length: return 2 as Length
bad(source [Byte; 4]) Byte:
    view: ref source.[0; consume(source)]
    return view.[0]
"#,
    );
    assert!(!typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn returning_a_borrowed_literal_uses_static_promotion() {
    let typed = check_with(
        PRELUDE,
        r#"
literal(seed Byte) ref [Byte; 2]: return ref [10 as Byte, 20 as Byte]
"#,
    );
    expect_no_flaws(&typed);
}

#[test]
fn reference_lengths_do_not_require_compile_time_arguments() {
    let typed = check_with(
        PRELUDE,
        r#"
prefix(source ref [Byte; 4], length Length) ref [Byte; length]: return ref source.[0; length]
read(source ref [Byte; 4], length Length) Byte:
    view: prefix(source, length)
    return view.[0]
"#,
    );

    let bind = &typed.defs[&typecheck::DefId::named(internment::Intern::from_ref("prefix"))];
    assert_eq!(
        bind.param_requirements.as_slice(),
        [typecheck::typed::AvailabilityRequirement::Any; 2]
    );
    expect_no_flaws(&typed);
}
