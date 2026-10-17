mod support;

use support::expect_flaws_with;

const TYPES: &str = r#"
Byte is in 0 to 255
Byte has
    finish(self):
        return
Resource has
    pointer @Byte
    finish(self):
        return
read(value ref Resource) @Byte: return value.pointer
"#;

#[test]
fn consumed_owned_parameter_can_be_initialized_again() {
    expect_flaws_with(
        TYPES,
        r#"
reuse(value own Resource, replacement @Byte):
    value.finish
    value:: Resource(pointer: replacement)
    value.finish
    return
"#,
        &[],
    );
}

#[test]
fn initialization_cannot_discard_a_live_owned_parameter() {
    expect_flaws_with(
        TYPES,
        r#"
reuse(value own Resource, replacement @Byte):
    value:: Resource(pointer: replacement)
    value.finish
    return
"#,
        &["type-rebind-would-discard-value"],
    );
}

#[test]
fn initialization_after_consumption_on_every_branch_is_allowed() {
    expect_flaws_with(
        TYPES,
        r#"
reuse(value own Resource, replacement @Byte, flag Byte):
    when flag is
        0 then value.finish
        else value.finish
    value:: Resource(pointer: replacement)
    value.finish
    return
"#,
        &[],
    );
}

#[test]
fn initialization_after_consumption_on_only_one_branch_is_rejected() {
    expect_flaws_with(
        TYPES,
        r#"
reuse(value own Resource, replacement @Byte, flag Byte):
    when flag is
        0 then value.finish
        else 0
    value:: Resource(pointer: replacement)
    value.finish
    return
"#,
        &["type-rebind-would-discard-value"],
    );
}

#[test]
fn initialization_does_not_revive_a_reference_to_the_consumed_value() {
    expect_flaws_with(
        TYPES,
        r#"
reuse(value own Resource, replacement @Byte) @Byte:
    ref saved Resource: value
    value.finish
    value:: Resource(pointer: replacement)
    observed: read(saved)
    value.finish
    return observed
"#,
        &["type-use-of-invalidated-reference"],
    );
}

#[test]
fn initialized_replacement_can_be_borrowed_again() {
    expect_flaws_with(
        TYPES,
        r#"
reuse(value own Resource, replacement @Byte) @Byte:
    value.finish
    value:: Resource(pointer: replacement)
    ref fresh Resource: value
    observed: read(fresh)
    value.finish
    return observed
"#,
        &[],
    );
}

#[test]
fn caller_owned_slot_can_swap_its_value_without_dropping_it() {
    expect_flaws_with(
        TYPES,
        &format!(
            "{}{}",
            include_str!("../../../modules/core/alloc/slot.gin"),
            r#"
run(pointer @Byte) Resource or Empty:
    slot: empty(Resource)
    previous: swap(Resource, slot, full(Resource, Resource(pointer)))
    previous.unpack
    taken: take(Resource, slot)
    slot.unpack
    return taken.unpack
"#,
        ),
        &[],
    );
}

#[test]
fn mutation_cannot_discard_the_live_referent() {
    expect_flaws_with(
        TYPES,
        r#"
replace(value mut Resource, replacement @Byte):
    value:: Resource(pointer: replacement)
    return
run(pointer @Byte, replacement @Byte):
    value: Resource(pointer)
    replace(value, replacement)
    value.finish
    return
"#,
        &["type-rebind-would-discard-value"],
    );
}

#[test]
fn mutable_slot_can_be_reborrowed_for_observation() {
    expect_flaws_with(
        TYPES,
        &format!(
            "{}{}",
            include_str!("../../../modules/core/alloc/slot.gin"),
            r#"
observe(x, slot ref Slot(x)):
    return
forward(x, slot mut Slot(x)):
    observe(x, slot)
    return
run(pointer @Byte) Resource or Empty:
    slot: full(Resource, Resource(pointer))
    forward(Resource, slot)
    return slot.unpack
"#,
        ),
        &[],
    );
}

#[test]
fn inspecting_slot_cases_through_a_shared_reference_preserves_the_slot() {
    expect_flaws_with(
        TYPES,
        &format!(
            "{}{}",
            include_str!("../../../modules/core/alloc/slot.gin"),
            r#"
inspect(x, slot ref Slot(x)) Byte:
    return when deref slot is
        Empty then 0
        Full then 1
run(pointer @Byte) Resource or Empty:
    slot: full(Resource, Resource(pointer))
    inspect(Resource, slot)
    inspect(Resource, slot)
    return slot.unpack
"#,
        ),
        &[],
    );
}

#[test]
fn shared_slot_cannot_be_forwarded_to_a_mutating_operation() {
    expect_flaws_with(
        TYPES,
        &format!(
            "{}{}",
            include_str!("../../../modules/core/alloc/slot.gin"),
            r#"
forward(x, slot ref Slot(x)) Slot(x): return take(x, slot)
run(pointer @Byte) Resource or Empty:
    slot: full(Resource, Resource(pointer))
    previous: forward(Resource, slot)
    slot.unpack
    return previous.unpack
"#,
        ),
        &[
            "type-mutable-borrow-from-shared",
            "type-generic-specialization",
            "type-incompatible-union-value",
            "type-move-out-of-borrow",
        ],
    );
}

#[test]
fn mutation_consumes_the_owned_replacement() {
    expect_flaws_with(
        TYPES,
        r#"
replace(value mut Resource, next own Resource):
    pointer: value as @Resource
    previous: pointer.[0]
    previous.finish
    pointer.[0]:: next
    next.finish
    return
run(pointer @Byte, replacement @Byte):
    value: Resource(pointer)
    replace(value, Resource(pointer: replacement))
    value.finish
    return
"#,
        &["type-use-after-move", "type-use-of-moved-value"],
    );
}

#[test]
fn owned_values_cannot_be_extracted_from_mutable_borrows() {
    expect_flaws_with(
        TYPES,
        "take(value mut Resource) Resource: return deref value\n",
        &["type-move-out-of-borrow"],
    );
}

#[test]
fn moved_mutable_referent_cannot_be_read_before_replacement() {
    expect_flaws_with(
        TYPES,
        r#"
replace(value mut Resource, replacement @Byte):
    previous: deref value
    previous.finish
    duplicate: deref value
    duplicate.finish
    value:: Resource(pointer: replacement)
    return
run(pointer @Byte, replacement @Byte):
    value: Resource(pointer)
    replace(value, replacement)
    value.finish
    return
"#,
        &[
            "type-move-out-of-borrow",
            "type-move-out-of-borrow",
            "type-rebind-would-discard-value",
        ],
    );
}
