//! Hover tests for semantic hover (`analyze::hover_at`).
//!
//! Uses a `†` (dagger, U+2020) cursor marker to indicate the hover position.
//! The [`hover_at_marker`] helper strips the marker and calls [`analyze::hover_at`].
//!
//! # Example
//!
//! ```ignore
//! #[test]
//! fn hover_my_function() {
//!     assert_hover(
//!         "--- Adds two numbers.\nadd(a Int, b Int) Int:\n    return a + b\n\n†add(1, 2)\n",
//!         &["add(a Int, b Int) Int", "Adds two numbers"],
//!     );
//! }
//! ```

use parser::expr::parse_source;
use typeck::hover_at;

// ─── Helpers ─────────────────────────────────────────────────────────────

/// Parse `source` containing a single `†` cursor marker and return the hover
/// markdown at that position.
///
/// The marker is stripped before parsing. Its byte offset in the original
/// source becomes the hover target in the cleaned source (the character that
/// was immediately after `†`).
///
/// Returns `None` when there is nothing hover-able at the marked position.
///
/// # Panics
///
/// Panics if the source does not contain exactly one `†` marker.
fn hover_at_marker(source: &str) -> Option<String> {
    let count = source.matches('†').count();
    assert_eq!(
        count, 1,
        "expected exactly one † cursor marker, found {count}"
    );

    let cursor_byte = source.find('†').unwrap();
    let cleaned = source.replace('†', "");

    let ast = parse_source(&cleaned);
    hover_at(&cleaned, &ast, cursor_byte)
}

/// Assert that hovering at the `†` marker in `source` produces markdown
/// containing every string in `expected_fragments`.
///
/// Panics with a diff-style message if any fragment is missing.
fn assert_hover(source: &str, expected_fragments: &[&str]) {
    let result = hover_at_marker(source).unwrap_or_else(|| {
        panic!(
            "expected a hover result but got None\n\
             source:\n{source}"
        )
    });

    for fragment in expected_fragments {
        assert!(
            result.contains(fragment),
            "hover result is missing expected fragment {:?}\n\n\
             full hover output:\n\
             ---\n\
             {result}\n\
             ---",
            fragment,
        );
    }
}

/// Assert that hovering at the `†` marker in `source` returns `None`
/// (nothing hover-able at that position).
#[allow(dead_code)]
fn assert_no_hover(source: &str) {
    let result = hover_at_marker(source);
    assert!(
        result.is_none(),
        "expected no hover result but got:\n\
         ---\n\
         {}\n\
         ---",
        result.unwrap(),
    );
}

// ─── Function hover ──────────────────────────────────────────────────────

#[test]
fn hover_simple_function() {
    assert_hover(
        "add(a Int, b Int) Int:\n    return a + b\n\n†add(1, 2)\n",
        &["add(a Int, b Int) Int"],
    );
}

#[test]
fn hover_function_on_definition() {
    assert_hover(
        "†add(a Int, b Int) Int:\n    return a + b\n",
        &["add(a Int, b Int) Int"],
    );
}

#[test]
fn hover_function_with_doc_comment() {
    assert_hover(
        "--- Adds two numbers together.\nadd(a Int, b Int) Int:\n    return a + b\n\n†add(1, 2)\n",
        &["add(a Int, b Int) Int", "Adds two numbers together."],
    );
}

#[test]
fn hover_function_with_multiline_doc_comment() {
    assert_hover(
        "--- Adds two numbers.\n--- Returns the sum.\nadd(a Int, b Int) Int:\n    return a + b\n\n†add(1, 2)\n",
        &[
            "add(a Int, b Int) Int",
            "Adds two numbers.",
            "Returns the sum.",
        ],
    );
}

#[test]
fn hover_function_with_complexity_attribute() {
    assert_hover(
        "#[complexity(Linear(n))]\n†find_index(target Byte, buf Buffer, len Int) Int:\n    i: 0\nreturn -1\n",
        &["find_index(target Byte, buf Buffer, len Int) Int", "O(n)"],
    );
}

#[test]
fn hover_function_with_doc_comment_before_complexity() {
    // This is the case that was previously broken: doc comments before #[complexity(...)]
    // were consumed by error handling and never attached to the bind.
    assert_hover(
        "--- Find the index of a target value.\n--- Scans left to right.\n#[complexity(Linear(len))]\n†find_index(target Byte, buf Buffer, len Int) Int:\n    i: 0\nreturn -1\n",
        &[
            "find_index(target Byte, buf Buffer, len Int) Int",
            "O(len)",
            "Find the index of a target value.",
            "Scans left to right.",
        ],
    );
}

#[test]
fn hover_function_with_complexity_before_doc_comment() {
    // Attributes before doc comments should also work.
    assert_hover(
        "#[complexity(Constant)]\n--- Always returns 42.\n†magic(x Int) Int:\nreturn 42\n",
        &["magic(x Int) Int", "O(1)", "Always returns 42."],
    );
}

#[test]
fn hover_method_name_on_call_shows_doc_comment() {
    assert_hover(
        "\
Range(x) has (start x, end x)

--- create a new range
Range(x).new(start x, end x) Range(x): (start, end)

main:
    r := Range.†new(12, 1200)
return
",
        &["Range.new(start x, end x) Range(x)", "create a new range"],
    );
}

// ─── Variable hover ──────────────────────────────────────────────────────

#[test]
fn hover_variable() {
    assert_hover("x: 42\n†x\n", &["x"]);
}

#[test]
fn hover_const_bind() {
    assert_hover("PI := 314\n†PI\n", &["PI"]);
}

// ─── Tag / Declare hover ─────────────────────────────────────────────────

#[test]
fn hover_tag_union() {
    assert_hover(
        "--- A value that may or may not exist.\nMaybe(x) is Some(x) or None\n\nval: †Maybe(3)\n",
        &["Maybe", "A value that may or may not exist."],
    );
}

#[test]
fn hover_tag_with_params() {
    assert_hover(
        "--- A point in 2D space.\nPoint is Record(x Int, y Int)\n\n†Point\n",
        &["Point", "A point in 2D space."],
    );
}

#[test]
fn hover_tag_without_doc() {
    assert_hover("Bool is True or False\n\n†Bool\n", &["Bool"]);
}

// ─── Size / Align metadata ───────────────────────────────────────────────

#[test]
fn hover_declare_bool_union_size_align() {
    // Bool is True or False — empty variants, just a 1-byte discriminant
    assert_hover(
        "Bool is True or False\n\n†Bool\n",
        &["Bool", "size = 1, align = 1"],
    );
}

#[test]
fn hover_declare_maybe_union_size_align() {
    // Maybe(x) is Some(x) or None — discriminant (1 byte) + opaque payload (8 bytes)
    assert_hover(
        "Maybe(x) is Some(x) or None\n\n†Maybe\n",
        &["Maybe", "size = 9, align = 8"],
    );
}

#[test]
fn hover_declare_record_size_align() {
    // Point is Record(x Int, y Int) — record fields summed
    assert_hover(
        "Point is Record(x Int, y Int)\n\n†Point\n",
        &["Point", "size = 8, align = 8"],
    );
}

#[test]
fn hover_declare_range_size_align() {
    // Int is 1...400 — range type
    assert_hover("Int is 1...400\n\n†Int\n", &["Int", "size = 2, align = 2"]);
}

#[test]
fn hover_function_no_size_align() {
    // Functions should NOT show size/align — they are behavior, not memory layout
    let result =
        hover_at_marker("add(a Int, b Int) Int:\n    return a + b\n\n†add(1, 2)\n").unwrap();
    assert!(
        result.contains("add(a Int, b Int) Int"),
        "should contain signature"
    );
    assert!(
        !result.contains("size ="),
        "functions should not show size, got:\n{result}"
    );
    assert!(
        !result.contains("align ="),
        "functions should not show align, got:\n{result}"
    );
}

#[test]
fn hover_function_with_complexity_no_size_align() {
    // Function with complexity should show complexity but NOT size/align
    let result = hover_at_marker(
        "#[complexity(Linear(n))]\n†find_index(target Byte, buf Buffer, len Int) Int:\n    i: 0\nreturn -1\n",
    ).unwrap();
    assert!(
        result.contains("complexity = O(n)"),
        "should contain complexity"
    );
    assert!(
        !result.contains("size ="),
        "functions should not show size, got:\n{result}"
    );
    assert!(
        !result.contains("align ="),
        "functions should not show align, got:\n{result}"
    );
}

#[test]
fn hover_variable_bind_size_align() {
    // Variable bind (no params) shows size/align of its value type
    assert_hover("x: 42\n†x\n", &["x", "size = 8, align = 8"]);
}

// ─── Parameter hover ──────────────────────────────────────────────────────

#[test]
fn hover_tagged_parameter_in_definition() {
    // Hovering `target` in the definition should show `target Byte`
    assert_hover(
        "find_index(target Byte, buf Buffer, len Int) Int:\n    †target\nreturn -1\n",
        &["target Byte"],
    );
}

#[test]
fn hover_tagged_parameter_at_call_site() {
    // Hovering a parameter name at a call site still shows its declaration type
    assert_hover(
        "find_index(target Byte, buf Buffer, len Int) Int:\n    return -1\n\nfind_index(†target, buf, 10)\n",
        &["target Byte"],
    );
}

#[test]
fn hover_generic_parameter() {
    // Generic parameter (no type tag) shows just the name
    let result = hover_at_marker("foo(x):\n    return x\n\n†x\n");
    assert!(result.is_some());
    let md = result.unwrap();
    assert!(md.contains("x"), "should contain 'x', got:\n{md}");
}

#[test]
fn hover_default_parameter() {
    // Parameter with default value shows `name: value`
    assert_hover(
        "greet(name: \"world\"):\n    return name\n\n†name\n",
        &["name"],
    );
}

// ─── Body-level bind hover ───────────────────────────────────────────────

#[test]
fn hover_body_typed_bind_with_size_align() {
    // val Maybe(3): Some(3) should show `val Maybe(3)` with sizing
    assert_hover(
        "Maybe(x) is Some(x) or None\n\nmain:\n    val Maybe(3): Some(3)\n    †val\nreturn\n",
        &["val Maybe(3)", "size = 9, align = 8"],
    );
}

#[test]
fn hover_body_untyped_bind() {
    // val: Some(3) — no type annotation, just shows the name
    assert_hover("main:\n    val: Some(3)\n    †val\nreturn\n", &["val"]);
}

#[test]
fn hover_body_inner_bind_in_if() {
    // `four` is bound inside an if-block — still discoverable
    assert_hover(
        "main:\n    val: Some(3)\n    if val is Some(v)\n        four: v + 1\n    return four\nreturn\n    †four\n",
        &["four"],
    );
}

// ─── Edge cases ──────────────────────────────────────────────────────────

#[test]
fn hover_on_unknown_word_returns_basic() {
    // Unknown words still get a basic code-block hover.
    let result = hover_at_marker("x: 5\n†y\n");
    assert!(result.is_some(), "should return Some for any word");
    assert!(result.unwrap().contains("y"));
}

#[test]
fn hover_extern_bind() {
    assert_hover(
        "--- External function.\nputs(s Ptr) extern\n\n†puts\n",
        &["puts", "External function."],
    );
}

// ─── Attribute combinations ──────────────────────────────────────────────

#[test]
fn hover_with_inline_attribute() {
    assert_hover(
        "#[inline]\n†fast_add(a Int, b Int) Int:\nreturn a + b\n",
        &["fast_add(a Int, b Int) Int"],
    );
}

#[test]
fn hover_with_test_attribute() {
    assert_hover(
        "#[test]\n†check_add(x Int):\nreturn x + 1\n",
        &["check_add(x Int)"],
    );
}

#[test]
fn hover_with_inline_attribute_and_doc() {
    assert_hover(
        "--- Runs a benchmark.\n--- Reports timing to stdout.\n#[inline]\n†bench_sort(n Int) Int:\nreturn n\n",
        &[
            "bench_sort(n Int) Int",
            "Runs a benchmark.",
            "Reports timing to stdout.",
        ],
    );
}

// ─── Tests from main.gin example ─────────────────────────────────────────
//
// These tests mirror constructs from packages/example/src/main.gin
// to ensure the LSP hover experience works well on real code.

#[test]
fn hover_maybe_tag_union() {
    // Maybe(x) is Some(x) or None
    assert_hover(
        "Maybe(x) is Some(x) or None\n\nval: †Maybe(3)\n",
        &["Maybe(x) is Some(x) or None"],
    );
}

#[test]
fn hover_int_range_decl() {
    // Int is 1...400
    assert_hover("Int is 1...400\n\nx: †Int\n", &["Int is 1...400"]);
}

#[test]
fn hover_find_index_full_example() {
    // The exact pattern from main.gin: doc comments, then #[complexity], then the function.
    // This was the primary bug — doc comments before attributes were silently dropped.
    let src = indoc::indoc! {"
        --- Find the index of a target value in a buffer.
        --- Scans each byte from left to right until a match is found.
        #[complexity(Linear(len))]
        find_index(target Byte, buf Buffer, len Int) Int:
            i: 0
            while i < len
                if buf.(i) = target
                return i
                i: i + 1
            loop
        return -1
    "};
    assert_hover(
        &format!("{src}\n†find_index(0, buf, 10)\n"),
        &[
            "find_index(target Byte, buf Buffer, len Int) Int",
            "O(len)",
            "Find the index of a target value in a buffer.",
            "Scans each byte from left to right until a match is found.",
        ],
    );
}

#[test]
fn hover_find_index_on_definition() {
    // Hovering directly on the definition name (not a call site).
    let src = indoc::indoc! {"
        --- Find the index of a target value in a buffer.
        --- Scans each byte from left to right until a match is found.
        #[complexity(Linear(len))]
        †find_index(target Byte, buf Buffer, len Int) Int:
            i: 0
        return -1
    "};
    assert_hover(
        src,
        &[
            "find_index(target Byte, buf Buffer, len Int) Int",
            "O(len)",
            "Find the index of a target value in a buffer.",
        ],
    );
}

#[test]
fn hover_main_function() {
    // main: with no params, no return tag.
    assert_hover("†main:\n    x: 5\nreturn x\n", &["main"]);
}

#[test]
fn hover_main_at_call_site() {
    assert_hover("main:\n    return 0\n\n†main\n", &["main"]);
}

#[test]
fn hover_some_variant_is_word_fallback() {
    // `Some` is a variant inside the Maybe union, not a top-level tag.
    // It falls through to the basic word hover.
    let result = hover_at_marker("Maybe(x) is Some(x) or None\n\nval: †Some(3)\n");
    assert!(result.is_some());
    let md = result.unwrap();
    assert!(md.contains("Some"), "should contain 'Some', got:\n{md}");
}

#[test]
fn hover_none_variant_is_word_fallback() {
    // `None` is a variant inside the Maybe union, not a top-level tag.
    let result = hover_at_marker("Maybe(x) is Some(x) or None\n\nv: †None\n");
    assert!(result.is_some());
    let md = result.unwrap();
    assert!(md.contains("None"), "should contain 'None', got:\n{md}");
}

#[test]
fn hover_body_variable_typed_bind() {
    // `val` is a body bind with type annotation — shows `val Maybe(3)` with sizing.
    let src = indoc::indoc! {"
        Maybe(x) is Some(x) or None

        main:
            val Maybe(3): Some(3)
            if val is Some(v)
                four: v + 1
            return four
        return
    "};
    assert_hover(
        &format!("{src}\n    †val\n"),
        &["val Maybe(3)", "size = 9, align = 8"],
    );
}

#[test]
fn hover_inner_four_body_bind() {
    // `four` is bound inside an if-block — still discovered as a body bind.
    let src = indoc::indoc! {"
        main:
            val: Some(3)
            if val is Some(v)
                four: v + 1
            return four
        return
    "};
    assert_hover(&format!("{src}\n    †four\n"), &["four"]);
}

#[test]
fn hover_typed_bind_with_tag_annotation() {
    // val Maybe(3): Some(3) — the `Maybe(3)` is a type annotation on the bind.
    // Hovering `Maybe` on a use site should still find the tag.
    assert_hover(
        "Maybe(x) is Some(x) or None\n\nval †Maybe(3): Some(3)\n",
        &["Maybe(x) is Some(x) or None"],
    );
}

// ─── Flow narrowing hover ────────────────────────────────────────────────

#[test]
fn hover_val_narrowed_inside_if() {
    // Inside `if val is Some(v)`, hovering `val` should show the narrowed type
    // with payload from constant propagation: `val Some(3)` instead of `val Maybe(3)`.
    // Size/align still comes from the original type annotation `Maybe(3)`.
    assert_hover(
        "Maybe(x) is Some(x) or None\n\nmain:\n    val Maybe(3): Some(3)\n    if val is Some(v)\n        result: †val\n        four: v + 1\n    return four\nreturn\n",
        &["val Some(3)", "size = 9, align = 8"],
    );
}

#[test]
fn hover_val_not_narrowed_on_declaration() {
    // On the declaration line, `val` should show the full type annotation `Maybe(3)`
    // with sizing — no narrowing is in effect yet.
    assert_hover(
        "Maybe(x) is Some(x) or None\n\nmain:\n    †val Maybe(3): Some(3)\n    if val is Some(v)\n        result: val\n        four: v + 1\n    return four\nreturn\n",
        &["val Maybe(3)", "size = 9, align = 8"],
    );
}

#[test]
fn hover_untyped_val_narrowed_inside_if() {
    // An untyped bind `val: Some(3)` narrowed inside `if val is Some(v)`
    // should show `val Some` with no size/align (no type annotation to derive sizing from).
    assert_hover(
        "Maybe(x) is Some(x) or None\n\nmain:\n    val: Some(3)\n    if val is Some(v)\n        result: †val\n        four: v + 1\n    return four\nreturn\n",
        &["val Some"],
    );
}

#[test]
fn hover_narrowed_val_no_size_align_when_untyped() {
    // Narrowed untyped bind should NOT include size/align metadata
    // (there's no type annotation to derive sizing from).
    let src = indoc::indoc! {"
        Maybe(x) is Some(x) or None

        main:
            val: Some(3)
            if val is Some(v)
                result: †val
                four: v + 1
            return four
        return
    "};
    let result = hover_at_marker(src).unwrap();
    assert!(
        !result.contains("size ="),
        "untyped narrowed bind should not show size/align, got:\n{result}"
    );
}

#[test]
fn hover_pattern_extracted_variable() {
    // Inside `if val is Some(v)` where val holds `Some(3)`,
    // hovering `v` should show `v 3` via pattern extraction.
    assert_hover(
        "Maybe(x) is Some(x) or None\n\nmain:\n    val Maybe(3): Some(3)\n    if val is Some(v)\n        result: val\n        four: †v + 1\n    return four\nreturn\n",
        &["v 3"],
    );
}

#[test]
fn hover_constant_folded_bind() {
    // `four: v + 1` where `v = 3` → constant folding gives `four = 4`.
    assert_hover(
        "Maybe(x) is Some(x) or None\n\nmain:\n    val Maybe(3): Some(3)\n    if val is Some(v)\n        result: val\n        †four: v + 1\n    return four\nreturn\n",
        &["four 4"],
    );
}

#[test]
fn hover_val_none_after_early_return() {
    // After `if val is Some(v) return four`, val must be `None`
    // since Maybe is `Some(x) or None` and we early-returned the Some case.
    assert_hover(
        "Maybe(x) is Some(x) or None\n\nmain:\n    val Maybe(3): Some(3)\n    if val is Some(v)\n        result: val\n        four: v + 1\n    return four\n    †val\nreturn\n",
        &["val None"],
    );
}

// ─── Full example integration tests ──────────────────────────────────────
//
// These tests validate the exact example from the design discussion:
//
//   main:
//       val Maybe(3): Some(3)     -- val Maybe(3)
//       if val is Some(v)         -- val Maybe(3); v 3
//           val                   -- val Some(3)
//           four: v + 1           -- v 3; four 4
//       return four
//       val                       -- val None (after early return of Some case)
//   return

/// Shared source for the full example integration tests.
fn full_example_source() -> String {
    indoc::indoc! {"
        Maybe(x) is Some(x) or None

        main:
            val Maybe(3): Some(3)
            if val is Some(v)
                val
                four: v + 1
            return four
            val
        return
    "}
    .to_string()
}

#[test]
fn example_val_on_declaration() {
    // `val Maybe(3): Some(3)` → shows the full type annotation with sizing.
    let src = full_example_source().replace("val Maybe(3)", "†val Maybe(3)");
    assert_hover(&src, &["val Maybe(3)", "size = 9, align = 8"]);
}

#[test]
fn example_val_on_if_condition() {
    // `if val is Some(v)` → hovering `val` shows `val Maybe(3)` (not narrowed yet).
    let src = full_example_source().replace("if val is", "if †val is");
    assert_hover(&src, &["val Maybe(3)"]);
}

#[test]
fn example_v_on_if_condition() {
    // `if val is Some(v)` → hovering `v` shows `v 3` via pattern extraction.
    let src = full_example_source().replace("Some(v)", "Some(†v)");
    assert_hover(&src, &["v 3"]);
}

#[test]
fn example_val_inside_if_body() {
    // Inside the if body, `val` is narrowed to `Some(3)` (with payload from constant propagation).
    let src = full_example_source().replace(
        "    if val is Some(v)\n        val\n",
        "    if val is Some(v)\n        †val\n",
    );
    assert_hover(&src, &["val Some(3)", "size = 9, align = 8"]);
}

#[test]
fn example_v_inside_if_body() {
    // Inside the if body, `v` is known to be `3`.
    let src = full_example_source().replace("four: v + 1", "four: †v + 1");
    assert_hover(&src, &["v 3"]);
}

#[test]
fn example_four_inside_if_body() {
    // `four: v + 1` where `v = 3` → constant folding gives `four 4`.
    let src = full_example_source().replace("four: v + 1", "†four: v + 1");
    assert_hover(&src, &["four 4"]);
}

#[test]
fn example_val_after_if() {
    // After `if val is Some(v) return four`, val must be `None`
    // since Maybe is `Some(x) or None` and we early-returned the Some case.
    let src =
        full_example_source().replace("    return four\n    val\n", "    return four\n    †val\n");
    assert_hover(&src, &["val None"]);
}

// ─── While loop comparison narrowing ─────────────────────────────────────
//
// Inside a `while cond` loop, the condition is known to be true.
// After the loop, the condition is known to be false (its negation).

/// Shared source for while loop narrowing tests — matches the actual main.gin
/// (the `if buf.(i) == target` is commented out there, omitted here for simplicity).
fn while_loop_source() -> String {
    indoc::indoc! {"
        find_index(target Byte, buf Buffer, len Int) Int:
            i: 0
            while i < len
                i: i + 1
            loop
            i
        return -1
    "}
    .to_string()
}

#[test]
fn while_i_inside_loop() {
    // Inside `while i < len`, hovering `i` on the assignment should show `i < len`.
    let src = while_loop_source().replace("i: i + 1", "†i: i + 1");
    assert_hover(&src, &["i < len"]);
}

#[test]
fn while_i_after_loop() {
    // After `while i < len loop`, hovering `i` should show `i >= len`
    // (the loop exits when the condition fails).
    let src = while_loop_source().replace("\n    i\n", "\n    †i\n");
    assert_hover(&src, &["i >= len"]);
}

#[test]
fn while_i_on_condition() {
    // On the condition line itself, `i` has its pre-loop constant value.
    // The comparison narrowing only applies inside the loop body.
    let src = while_loop_source().replace("while i < len", "while †i < len");
    assert_hover(&src, &["i 0"]);
}

// ─── Boolean if condition narrowing ──────────────────────────────────────
//
// When `if` uses a boolean condition like `if num < 10`, comparison narrowing
// should apply inside the body and negated narrowing after early return.

#[test]
fn hover_num_narrowed_inside_if_less_than() {
    // Inside `if num < 10`, hovering `num` (a parameter) should show `num Int < 10`.
    assert_hover(
        indoc::indoc! {"
            less_than_ten(num Int) Maybe(Int):
                if num < 10
                    †num
                return Some(num)
            return None
        "},
        &["num Int < 10"],
    );
}

#[test]
fn hover_num_narrowed_after_early_return_if_less_than() {
    // After `if num < 10 return Some(num)`, hovering `num` (a parameter) should show `num Int >= 10`.
    assert_hover(
        indoc::indoc! {"
            less_than_ten(num Int) Maybe(Int):
                if num < 10
                    num
                return Some(num)
                †num
            return None
        "},
        &["num Int >= 10"],
    );
}

#[test]
fn hover_num_narrowed_inside_if_greater_than() {
    // Inside `if x > 0`, hovering `x` should show `x > 0`.
    assert_hover(
        indoc::indoc! {"
            test:
                x: 5
                if x > 0
                    †x
                return
            return
        "},
        &["x > 0"],
    );
}

#[test]
fn hover_num_narrowed_after_early_return_if_greater_than() {
    // After `if x > 0 return 0`, hovering `x` should show `x <= 0`.
    // Uses `return 0` instead of bare `return` to avoid a parser ambiguity
    // where bare `return` would consume the next line's `x` as its return value.
    assert_hover(
        indoc::indoc! {"
            test:
                x: 5
                if x > 0
                    x
                return 0
                †x
            return
        "},
        &["x <= 0"],
    );
}

#[test]
fn hover_if_bool_condition_with_constant_bound() {
    // Inside `if count >= 3`, hovering `count` should show `count >= 3`.
    assert_hover(
        indoc::indoc! {"
            test:
                count: 0
                if count >= 3
                    †count
                return
            return
        "},
        &["count >= 3"],
    );
}

#[test]
fn hover_if_bool_body_bind_narrowed_after_early_return() {
    // After `if num < 10 return Some(num)`, hovering `num` (a body bind) should show `num >= 10`.
    assert_hover(
        indoc::indoc! {"
            Maybe(x) is Some(x) or None

            main:
                num: 5
                if num < 10
                    num
                return Some(num)
                †num
            return None
        "},
        &["num >= 10"],
    );
}

// ─── Full main.gin source ────────────────────────────────────────────────
//
// The tests below use the exact source from packages/example/src/main.gin
// to validate hover against real code rather than minimal snippets.

/// Returns the full source of main.gin with a `†` marker stripped out.
/// Tests call this and then insert their own marker at the desired position.
fn main_gin_source() -> String {
    indoc::indoc! {"
        Maybe(x) is Some(x) or None

        Int is 1...400


        -- is_empty(v Maybe(x)) Bool: when v is None then True else False

        --- Find the index of a target value in a buffer.
        --- Scans each byte from left to right until a match is found.
        #[complexity(Linear(len))]
        find_index(target Byte, buf Buffer, len Int) Int:
            i: 0
            while i < len
                if buf.(i) = target
                return i
                i: i + 1
            loop
        return -1

        main:
            val Maybe(3): Some(3)

            if val is Some(v)
                val
                four: v + 1
            return four

            -- when v is Some(x)
            -- then S
            -- else D

            val
        return
    "}
    .to_string()
}

#[test]
fn hover_main_gin_find_index_on_definition() {
    // Hover right on the definition of find_index in the real main.gin source.
    let src = main_gin_source().replace("find_index(", "†find_index(");
    assert_hover(
        &src,
        &[
            "find_index(target Byte, buf Buffer, len Int) Int",
            "O(len)",
            "Find the index of a target value in a buffer.",
            "Scans each byte from left to right until a match is found.",
        ],
    );
}

#[test]
fn hover_main_gin_maybe_tag() {
    // Hover Maybe in the type annotation `val Maybe(3)`.
    let src = main_gin_source().replace("val Maybe(3)", "val †Maybe(3)");
    assert_hover(&src, &["Maybe(x) is Some(x) or None"]);
}

#[test]
fn hover_main_gin_int_range() {
    // Hover Int in the range declaration.
    let src = main_gin_source().replace("Int is", "†Int is");
    assert_hover(&src, &["Int is 1...400"]);
}

#[test]
fn hover_main_gin_main_function() {
    // Hover on the main function definition.
    let src = main_gin_source().replace("main:", "†main:");
    assert_hover(&src, &["main"]);
}
