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

// ── Function parameters ────────────────────────────────────────────────

#[test]
fn func_params_comma_separated_inline_still_works() {
    let src = "add(a Int, b Int) Int := a + b\n";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "comma-separated inline params must work"
    );
    assert!(has_def(&out, "add"), "add should parse as a bind");
}

#[test]
fn func_params_newline_separated_multiline() {
    let src = "\
add(
    a Int
    b Int
) Int := a + b
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "newline-separated multiline params must parse cleanly: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(has_def(&out, "add"), "add should parse as a bind");
}

#[test]
fn func_params_newline_with_trailing_comma() {
    let src = "\
add(
    a Int,
    b Int,
) Int := a + b
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "trailing comma after newline-separated params"
    );
    assert!(has_def(&out, "add"));
}

#[test]
fn func_params_comma_and_newline_mixed() {
    let src = "\
add(
    a Int,
    b Int,
    c Int
) Int := a + b + c
";
    let out = src.parse_source_full();
    assert_eq!(count_errors(&out), 0, "mixed comma and newline separators");
    assert!(has_def(&out, "add"));
}

#[test]
fn func_params_default_value_with_newline() {
    let src = "\
foo(
    x Int
    y: 42
) Int := x + y
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "default value params with newline separators: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(has_def(&out, "foo"));
}

#[test]
fn func_params_convention_prefix_with_newline() {
    // Convention-prefixed params need a comma before the next param
    // because bare Id after a convention param name is parsed as a type
    // variable (not the next param name). This matches existing Gin behavior.
    let src = "\
set_item(
    ref self,
    val Int
) Int := 0
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "convention prefix params with newline separators: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(has_def(&out, "set_item"));
}

#[test]
fn func_params_empty_parens_works() {
    let src = "main() Int := 0\n";
    let out = src.parse_source_full();
    assert_eq!(count_errors(&out), 0, "empty parens must still work");
    assert!(has_def(&out, "main"));
}

#[test]
fn func_params_single_param_with_newline_after_open() {
    let src = "\
foo(
    x Int
) Int := x
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "single param after newline must work"
    );
    assert!(has_def(&out, "foo"));
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
fn func_params_three_params_multiline() {
    let src = "\
distance(
    x1 Float
    y1 Float
    x2 Float
) Float := 0.0
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "three newline-separated params must work"
    );
    assert!(has_def(&out, "distance"));
}

// ── Type parameters (tag declarations) ──────────────────────────────

#[test]
fn declare_params_comma_separated_inline_works() {
    let src = "Pair(x, y) has first x, second y\n";
    let out = src.parse_source_full();
    assert_eq!(count_errors(&out), 0, "comma-separated type params inline");
    assert!(has_tag(&out, "Pair"));
}

#[test]
fn declare_params_newline_separated() {
    let src = "\
Pair(
    x
    y
) has
    first x
    second y
)
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "newline-separated type params: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(has_tag(&out, "Pair"));
}

#[test]
fn declare_params_and_has_members_use_newline_separators() {
    let src = "\
Pair(
    x
    y
) has
    first x
    second y
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "type params and has members with newline separators"
    );
    assert!(has_tag(&out, "Pair"));
}

// `x y` in type-param position is valid Gin: `x` with type variable `y`
// (a pre-existing `parse_param_after_name` behavior). A true
// missing-separator rejection requires tokens that cannot form
// a valid `name type` annotation, e.g. two Tags back-to-back.

#[test]
fn declare_params_with_type_default_newline() {
    // `name: Default` (no type annotation) is valid Gin for default params.
    // `name Type: Default` is not currently supported by parse_param_after_name.
    let src = "\
Map(
    key
    value
    allocator: DefaultAllocator
) has
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "type params with default and newline separators: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(has_tag(&out, "Map"));
}

// ── Type expressions with type params ──────────────────────────────────

#[test]
fn type_expr_type_params_newline_separated() {
    // A function with a return type that takes multiple type arguments.
    // `Maybe(Int, Str)` exercises `parse_tag_type_params_delimited`.
    let src = "\
foo(x Int) Maybe(Int, Str) := 0
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "comma-separated type params in type expr: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(has_def(&out, "foo"));
}

#[test]
fn type_expr_type_params_newline_multiline() {
    // Type expression with multiline type arguments — exercises
    // `parse_tag_type_params_delimited` with newline separators.
    // This requires the type expression to be in a position where
    // it doesn't look like a function call. Return type works.
    let src = "\
bar(x Int) Maybe(
    Int
    Str
) := 0
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "newline-separated type params in type expr: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(has_def(&out, "bar"));
}

#[test]
fn has_members_comma_separated_inline() {
    let src = "Allocator has allocate(size Int) Ptr, free(ptr Ptr)\n";
    let out = src.parse_source_full();
    assert_eq!(count_errors(&out), 0, "comma-separated has members inline");
    assert!(has_tag(&out, "Allocator"));
}

#[test]
fn has_members_newline_separated() {
    let src = "\
Allocator has
    allocate(size Int) Ptr
    free(ptr Ptr)
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "newline-separated has members: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(has_tag(&out, "Allocator"));
}

#[test]
fn has_members_multiline_without_commas() {
    let src = "\
Allocator has
    allocate(size Int) Ptr
    free(ptr Ptr)
";
    let out = src.parse_source_full();
    assert_eq!(count_errors(&out), 0, "has members with newline separators");
    assert!(has_tag(&out, "Allocator"));
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
fn has_members_empty_works() {
    let src = "Marker has \n";
    let out = src.parse_source_full();
    assert_eq!(count_errors(&out), 0, "empty has declaration must work");
    assert!(has_tag(&out, "Marker"));
}

#[test]
fn has_members_three_methods_multiline() {
    let src = "\
Allocator has
    allocate(size Int) Ptr
    free(ptr Ptr)
    resize(ptr Ptr, old_size Int, new_size Int) Ptr
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "three newline-separated has members: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(has_tag(&out, "Allocator"));
}

#[test]
fn has_members_with_return_type_newline() {
    let src = "\
Hasher has
    hash(data Slice(Byte)) Int
    reset()
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "has members with return types and newline separators"
    );
    assert!(has_tag(&out, "Hasher"));
}

// ── Function call arguments ──────────────────────────────────────────────

#[test]
fn call_args_newline_separated() {
    let src = "\
main:
    write(
        fd
        buf
        len
    )
return
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "newline-separated call args: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(has_def(&out, "main"));
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

// ── List literals ───────────────────────────────────────────────────

#[test]
fn list_lit_newline_separated() {
    let src = "\
xs : [
    1
    2
    3
]
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "newline-separated list elements: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(has_def(&out, "xs"));
}

#[test]
fn list_lit_comma_and_newline_mixed() {
    let src = "\
xs : [
    1,
    2,
    3
]
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "mixed comma and newline in list: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(has_def(&out, "xs"));
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
fn list_lit_trailing_comma_multiline() {
    let src = "\
xs : [
    1,
    2,
]
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "trailing comma in multiline list: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(has_def(&out, "xs"));
}

#[test]
fn list_lit_empty_with_layout() {
    let src = "xs : [\n\n\n]\n";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "empty list with blank lines inside: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(has_def(&out, "xs"));
}

// ── Tuple literals ───────────────────────────────────────────────────────

#[test]
fn tuple_lit_newline_separated() {
    let src = "\
pair : (
    1
    2
)
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "newline-separated tuple elements: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(has_def(&out, "pair"));
}

#[test]
fn tuple_lit_same_line_missing_comma_rejected() {
    let src = "pair : (1 2 3)\n";
    let out = src.parse_source_full();
    let errors = count_errors(&out);
    assert!(
        errors > 0,
        "same-line tuple elements without comma should produce an error"
    );
}

// ── Bundle export lists (use 'module'.(Foo, Bar)) ────────────────────

#[test]
fn bundle_exports_newline_separated() {
    let src = "use 'module'.(\n    Foo\n    Bar\n)\n";
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
    let src = "use 'module'.(\n    Foo,\n    Bar,\n)\n";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "mixed comma and newline in bundle exports"
    );
}

#[test]
fn bundle_exports_same_line_missing_comma_rejected() {
    let src = "use 'module'.(Foo Bar)\n";
    let out = src.parse_source_full();
    let errors = count_errors(&out);
    assert!(
        errors > 0,
        "same-line bundle exports without comma should produce an error: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

// ── Record literal fields ────────────────────────────────────────────

#[test]
fn record_fields_newline_separated() {
    // Record literals use the `(name: value)` syntax inside function bodies.
    // The field separator is comma, but newlines also work because the loop
    // naturally advances past layout tokens between fields.
    let src = "\
main:
    point : (
        x: 1
        y: 2
    )
return
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "newline-separated record fields: {:?}",
        out.symptoms.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(has_def(&out, "main"));
}

#[test]
fn record_fields_comma_and_newline_mixed() {
    let src = "\
main:
    point : (
        x: 1,
        y: 2,
    )
return
";
    let out = src.parse_source_full();
    assert_eq!(
        count_errors(&out),
        0,
        "mixed comma and newline in record fields"
    );
    assert!(has_def(&out, "main"));
}
