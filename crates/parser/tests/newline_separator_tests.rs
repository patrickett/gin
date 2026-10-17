//! Tests for newline-as-separator in delimited lists.
//!
//! In Gin, items inside `(...)` or `[...]` may be separated by either a comma
//! or a newline. A comma is required only when two items appear on the same line.
//!
//! These tests cover:
//! - Function parameter lists
//! - Type parameter lists
//! - Declare (tag) parameter lists
//! - `has` member lists

use parser::query::SourceParseExt;

mod support;
use support::*;

// `x y` in type-param position is valid Gin: `x` with type variable `y`
// (a pre-existing `parse_param_after_name` behavior). A true
// missing-separator rejection requires tokens that cannot form
// a valid `name type` annotation, e.g. two Tags back-to-back.
/// Uniform clean-parse cases; the first tuple field is the former test name,
/// kept so failures identify the case.
#[rustfmt::skip]
const NEWLINE_SEPARATOR_CASES: &[(&str, &str, &str, bool)] = &[
    ("func_params_comma_separated_inline_still_works", "add(a Int, b Int) Int := return a + b\n", "add", false),
    ("func_params_newline_separated_multiline", "\
add(
    a Int
    b Int
) Int := a + b
", "add", false),
    ("func_params_newline_with_trailing_comma", "\
add(
    a Int,
    b Int,
) Int := a + b
", "add", false),
    ("func_params_comma_and_newline_mixed", "\
add(
    a Int,
    b Int,
    c Int
) Int := a + b + c
", "add", false),
    ("func_params_default_value_with_newline", "\
foo(
    x Int
    y: 42
) Int := x + y
", "foo", false),
    // Suffix mode params (for example `self ref`) are parsed like other multiline
    // params because the mode appears after the parameter name.
    ("func_params_convention_suffix_with_newline", "\
set_item(
    self ref,
    val Int
) Int := 0
", "set_item", false),
    ("func_params_empty_parens_works", "main() Int := return 0\n", "main", false),
    ("func_params_single_param_with_newline_after_open", "\
foo(
    x Int
) Int := x
", "foo", false),
    ("func_params_three_params_multiline", "\
distance(
    x1 Float
    y1 Float
    x2 Float
) Float := 0.0
", "distance", false),
    ("declare_params_comma_separated_inline_works", "Pair(x, y) has first x, second y\n", "Pair", true),
    ("declare_params_newline_separated", "\
Pair(
    x
    y
) has
    first x
    second y
)
", "Pair", true),
    ("declare_params_and_has_members_use_newline_separators", "\
Pair(
    x
    y
) has
    first x
    second y
", "Pair", true),
    // `name: Default` (no type annotation) is valid Gin for default params.
    // `name Type: Default` is not currently supported by parse_param_after_name.
    ("declare_params_with_type_default_newline", "\
Map(
    key
    value
    allocator: DefaultAllocator
) has
", "Map", true),
    // A function with a return type that takes multiple type arguments.
    // `Maybe(Int, Str)` exercises `parse_tag_type_params_delimited`.
    ("type_expr_type_params_newline_separated", "\
foo(x Int) Maybe(Int, Str) := 0
", "foo", false),
    // Type expression with multiline type arguments — exercises
    // `parse_tag_type_params_delimited` with newline separators.
    // This requires the type expression to be in a position where
    // it doesn't look like a function call. Return type works.
    ("type_expr_type_params_newline_multiline", "\
bar(x Int) Maybe(
    Int
    Str
) := 0
", "bar", false),
    ("has_members_comma_separated_inline", "Allocator has allocate(size Int) Ptr, free(ptr Ptr)\n", "Allocator", true),
    ("has_members_newline_separated", "\
Allocator has
    allocate(size Int) Ptr
    free(ptr Ptr)
", "Allocator", true),
    ("has_members_empty_works", "Marker has \n", "Marker", true),
    ("has_members_three_methods_multiline", "\
Allocator has
    allocate(size Int) Ptr
    free(ptr Ptr)
    resize(ptr Ptr, old_size Int, new_size Int) Ptr
", "Allocator", true),
    ("has_members_with_return_type_newline", "\
Hasher has
    hash(data Slice(Byte)) Int
    reset()
", "Hasher", true),
    ("call_args_newline_separated", "\
main:
    write(
        fd
        buf
        len
    )
    return
", "main", false),
    ("list_lit_newline_separated", "\
xs : [
    1
    2
    3
]
", "xs", false),
    ("list_lit_comma_and_newline_mixed", "\
xs : [
    1,
    2,
    3
]
", "xs", false),
    ("list_lit_trailing_comma_multiline", "\
xs : [
    1,
    2,
]
", "xs", false),
    ("list_lit_empty_with_layout", "xs : [\n\n\n]\n", "xs", false),
    ("value_row_lit_newline_separated", "\
pair : [
    1
    2
]
", "pair", false),
    // Record literals use the `[name: value]` syntax inside function bodies.
    // The field separator is comma, but newlines also work because the loop
    // naturally advances past layout tokens between fields.
    ("record_fields_newline_separated", "\
main:
    point : [
        x: 1
        y: 2
    ]
    return
", "main", false),
    ("record_fields_comma_and_newline_mixed", "\
main:
    point : [
        x: 1,
        y: 2,
    ]
    return
", "main", false),
];

#[test]
fn newline_separated_sources_parse_without_errors() {
    for (test_name, src, name, expect_tag) in NEWLINE_SEPARATOR_CASES {
        let out = src.parse_source_full();
        assert_eq!(
            count_errors(&out),
            0,
            "{test_name}: {:?}",
            out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        if *expect_tag {
            assert!(has_tag(&out, name), "{test_name}: expected tag `{name}`");
        } else {
            assert!(has_def(&out, name), "{test_name}: expected def `{name}`");
        }
    }
}

#[test]
fn func_params_same_line_missing_comma_rejected() {
    let src = "add(a Int b Int) Int := a + b\n";
    let out = src.parse_source_full();
    let errors = count_errors(&out);
    assert!(
        errors > 0,
        "same-line params without comma should produce an error"
    );
}

#[test]
fn has_members_same_line_missing_comma_rejected() {
    let src = "Allocator has allocate(size Int) Ptr free(ptr Ptr)\n";
    let out = src.parse_source_full();
    let errors = count_errors(&out);
    assert!(
        errors > 0,
        "same-line has members without comma should produce an error"
    );
}

#[test]
fn call_args_same_line_missing_comma_rejected() {
    let src = "\
main:
    write(fd buf len)
    return
";
    let out = src.parse_source_full();
    let errors = count_errors(&out);
    assert!(
        errors > 0,
        "same-line call args without comma should produce an error"
    );
}

#[test]
fn list_lit_same_line_missing_comma_rejected() {
    let src = "xs : [1 2 3]\n";
    let out = src.parse_source_full();
    let errors = count_errors(&out);
    assert!(
        errors > 0,
        "same-line list elements without comma should produce an error"
    );
}

#[test]
fn value_row_lit_same_line_missing_comma_rejected() {
    let src = "pair : (1 2 3)\n";
    let out = src.parse_source_full();
    let errors = count_errors(&out);
    assert!(
        errors > 0,
        "same-line value-row elements without comma should produce an error"
    );
}

#[test]
fn bundle_exports_newline_separated() {
    let src = "use 'module'\n    Foo\n    Bar\n";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "newline-separated bundle exports: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn bundle_exports_comma_and_newline_mixed() {
    let src = "use 'module'\n    Foo,\n    Bar\n";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "mixed comma and newline in bundle exports"
    );
}

#[test]
fn bundle_exports_same_line_missing_comma_rejected() {
    let src = "use 'module' Foo Bar\n";
    let out = src.parse_source_full();
    let errors = count_errors(&out);
    assert!(
        errors > 0,
        "same-line bundle exports without comma should produce an error: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}
