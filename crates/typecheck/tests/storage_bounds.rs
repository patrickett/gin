mod support;

use support::expect_flaws_with;

const TYPES: &str = r#"
Byte is in 0 to 255
Byte has
    finish(self):
        return
#fixed_array(element, count)
Buffer(element, count BigInt) has
    finish(self):
        return
"#;

#[test]
fn constant_read_at_capacity_is_rejected() {
    expect_flaws_with(
        TYPES,
        "get(values ref Buffer(Byte, 4)) Byte: return values.[4]\n",
        &["type-array-index-out-of-bounds"],
    );
}

#[test]
fn negative_constant_read_is_rejected() {
    expect_flaws_with(
        TYPES,
        "get(values ref Buffer(Byte, 4)) Byte: return values.[-1]\n",
        &["type-array-index-out-of-bounds"],
    );
}

#[test]
fn constant_write_at_capacity_is_rejected() {
    expect_flaws_with(
        TYPES,
        "replace(previous Byte, next Byte) Byte: return next\nset(values mut Buffer(Byte, 4), value Byte):\n    values.[4]:: replace(values.[0], value)\n    return\n",
        &[
            "type-array-index-out-of-bounds",
            "type-rebind-would-discard-value",
        ],
    );
}

#[test]
fn both_valid_boundary_indices_are_allowed() {
    expect_flaws_with(
        TYPES,
        "first(values ref Buffer(Byte, 4)) Byte: return values.[0]\nlast(values ref Buffer(Byte, 4)) Byte: return values.[3]\n",
        &[],
    );
}

#[test]
fn zero_capacity_has_no_valid_index() {
    expect_flaws_with(
        TYPES,
        "get(values ref Buffer(Byte, 0)) Byte: return values.[0]\n",
        &["type-array-index-out-of-bounds"],
    );
}

#[test]
fn dynamic_access_is_allowed_for_native_bounds_checking() {
    expect_flaws_with(
        TYPES,
        "get(values ref Buffer(Byte, 4), index Byte) Byte: return values.[index]\n",
        &[],
    );
}

#[test]
fn a_scalar_reference_has_only_one_element() {
    expect_flaws_with(
        TYPES,
        "get(value ref Byte) Byte: return value.[1]\n",
        &["type-array-index-out-of-bounds"],
    );
    expect_flaws_with(TYPES, "get(value ref Byte) Byte: return value.[0]\n", &[]);
}

#[test]
fn dynamic_scalar_reference_indexing_requires_native_bounds_checking() {
    expect_flaws_with(
        TYPES,
        "get(value ref Byte, index Byte) Byte: return value.[index]\n",
        &[],
    );
}
