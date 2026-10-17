mod support;

use support::check;

#[test]
fn repeated_array_lengths_infer_from_checked_arithmetic_annotations() {
    for (length, valid) in [
        ("1 + 2", true),
        ("3 + 0", true),
        ("3 * 1", true),
        ("2 + 2", false),
    ] {
        let typed = check(&format!(
            "Length is in 0 to 18446744073709551615\nLength has finish(self): return\nsame(left [in 0 to 255; count], right [in 0 to 255; count]) Length: return count as Length\nrun(left [in 0 to 255; {length}], right [in 0 to 255; 3]) Length: return same(left, right)\n"
        ));
        if valid {
            assert!(
                typed.all_flaws().is_empty(),
                "{length}: {:?}",
                typed.all_flaws()
            );
        } else {
            assert!(
                typed
                    .all_flaws()
                    .iter()
                    .any(|(_, flaw)| flaw.code.slug() == "type-array-length-inference"),
                "{length}: {:?}",
                typed.all_flaws()
            );
        }
    }
}

#[test]
fn array_lengths_are_inferred_from_values_and_reused() {
    let typed = check(
        r#"Length is in 0 to 18446744073709551615
Length has
    finish(self):
        return
size(bytes [in 0 to 255; count]) Length: return count as Length
empty: size('')
short: size('abc')
again: size('xyz')
unicode: size('é🦀')
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_eq!(
        typed
            .defs
            .keys()
            .filter(|id| id.0.as_str().starts_with("size$array$"))
            .count(),
        3,
    );
}

#[test]
fn array_elements_can_use_a_named_integer_domain() {
    let typed = check(&format!(
        "{}Length is in 0 to 18446744073709551615\nLength has\n    finish(self):\n        return\nsize(bytes [in Byte; count]) Length: return count as Length\nvalue: size('abc')\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn repeated_array_length_requires_matching_arguments() {
    let typed = check(
        r#"Length is in 0 to 18446744073709551615
Length has
    finish(self):
        return
same(left [in 0 to 255; count], right [in 0 to 255; count]) Length: return count as Length
bad: same('a', 'bc')
"#,
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-array-length-inference")
    );
}

#[test]
fn inferred_lengths_do_not_relax_element_types() {
    let typed = check(
        r#"Length is in 0 to 18446744073709551615
Length has
    finish(self):
        return
size(bytes [in 0 to 255; count]) Length: return count as Length
bad(values [in 0 to 65535; 3]) Length: return size(values)
"#,
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-array-length-inference")
    );
}

#[test]
fn specialized_bodies_are_checked() {
    let typed = check(
        r#"Length is in 0 to 18446744073709551615
Length has
    finish(self):
        return
bad(bytes [in 0 to 255; count]) Length: return missing(bytes)
value: bad('abc')
"#,
    );
    assert!(!typed.all_flaws().is_empty());
}

#[test]
fn array_results_and_named_arguments_keep_inferred_lengths() {
    let typed = check(
        r#"Length is in 0 to 18446744073709551615
Length has
    finish(self):
        return
identity(bytes [in 0 to 255; count]) [in 0 to 255; count]: return bytes
same(left [in 0 to 255; count], right [in 0 to 255; count]) Length: return count as Length
value: same(right: identity('abc'), left: 'def')
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn passing_an_array_by_value_preserves_move_checks() {
    let typed = check(
        r#"Length is in 0 to 18446744073709551615
Length has
    finish(self):
        return
size(bytes [in 0 to 255; count]) Length: return count as Length
entry:
    bytes [in 0 to 255; 3]: [97, 98, 99]
    first: size(bytes)
    return size(bytes)
"#,
    );
    assert!(typed.all_flaws().iter().any(|(_, flaw)| matches!(
        flaw.code.slug().as_str(),
        "type-use-after-move" | "type-use-of-moved-value"
    )));
}

#[test]
fn an_existing_value_is_not_rebound_as_an_inferred_length() {
    let typed = check(
        r#"Length is in 0 to 18446744073709551615
Length has
    finish(self):
        return
count := 2
size(bytes [in 0 to 255; count]) Length: return count as Length
value: size('abc')
"#,
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-array-length-inference")
    );
}
