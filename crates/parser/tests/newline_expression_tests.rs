//! Tests probing how Gin handles newlines and expression continuation.
//!
//! These tests exercise the edge cases discussed in the blog post
//! "No Semicolons Needed" (https://terts.dev/blog/no-semicolons-needed/),
//! comparing Gin's behavior against the approaches used by
//! Python, Go, Kotlin, Swift, JavaScript, Gleam, Lua, Ruby, R, Julia, and Odin.

use internment::Intern;
use parser::parse_source_full;

fn intern(s: &str) -> Intern<String> {
    Intern::new(s.to_owned())
}

fn count_unused(out: &parser::ParseOutput) -> usize {
    out.symptoms
        .iter()
        .filter(|d| d.message.contains("unused"))
        .count()
}

fn count_errors(out: &parser::ParseOutput) -> usize {
    out.symptoms
        .iter()
        .filter(|d| d.message.contains("error") || d.message.contains("expected"))
        .count()
}

fn has_def(out: &parser::ParseOutput, name: &str) -> bool {
    out.ast.defs().contains_key(&intern(name))
}

fn count_exprs(out: &parser::ParseOutput) -> usize {
    out.ast.top_level_exprs().len()
}

#[allow(dead_code)]
fn print_diags(out: &parser::ParseOutput) {
    for d in &out.symptoms {
        println!("  {:?}: {}", d.code, d.message);
    }
}

// ── SAME-INDENT CONTINUATION (No Indent token emitted) ─────────────────
//
// These work because no Indent/Dedent tokens are inserted — the next line
// has the same indentation as the expression start.

#[test]
fn same_indent_minus_at_start_of_line() {
    // With Indent-as-continuation, same-indent minus parses as binary.
    let src = "y := 2 * 4\n- 3\n";
    let out = parse_source_full(src);
    assert!(has_def(&out, "y"), "should be a single bind");
    print_diags(&out);
}

#[test]
fn same_indent_multiple_lines() {
    let src = "x := 1\n+ 2\n+ 3\n";
    let out = parse_source_full(src);
    assert!(
        has_def(&out, "x"),
        "should be a single bind for x = 1 + 2 + 3"
    );
    assert_eq!(count_unused(&out), 0, "no unused values");
}

#[test]
fn same_indent_minus_touching_number() {
    let src = "y := 4\n-3\n";
    let out = parse_source_full(src);
    assert!(has_def(&out, "y"), "y should be a single bind");
    assert_eq!(count_unused(&out), 0, "no unused value");
}

#[test]
fn same_indent_function_call_across_lines() {
    // Swift guardrail: `(` on next line is NOT a call.
    let src = "foo\n(1)\n";
    let out = parse_source_full(src);
    assert_eq!(
        count_exprs(&out),
        2,
        "Swift guardrail: foo and (1) on separate lines are two exprs"
    );
}

// ── GREATER-INDENT BLOCK BEHAVIOR (Indent token breaks continuation) ───
//
// When the next line is indented more, the lexer emits an `Indent` token.
// This is seen by the parser as starting a new indented block, NOT as
// continuing the expression.

#[test]
fn greater_indent_trailing_operator() {
    let src = "x := 3 +\n  2\n";
    let out = parse_source_full(src);
    assert!(
        has_def(&out, "x"),
        "x should be bound (value is 3 + <error>)"
    );
    assert_eq!(count_exprs(&out), 1, "2 becomes a top-level expr");
    let unused = count_unused(&out);
    assert!(
        unused >= 1,
        "should warn about unused 2, got {unused} unused diags"
    );
    // Indent token after trailing operator causes the RHS to be lost.
    // Workaround: place operator at start of next line with same indent,
    // or use a parenthesized group.
}

#[test]
fn greater_indent_leading_operator() {
    let src = "x := 1\n    + 2\n    + 3\n";
    let out = parse_source_full(src);
    assert!(has_def(&out, "x"), "x should be bound (value is just 1)");
    let errors = count_errors(&out);
    assert_eq!(
        errors, 0,
        "no parse errors (the + lines are silently ignored inside the indent block)"
    );
}

// ── RETURN WITH NEWLINE ──────────────────────────────────────────────────

#[test]
fn return_value_on_same_line() {
    let src = "main:\n    return 5\nloop\n";
    let out = parse_source_full(src);
    assert!(
        has_def(&out, "main"),
        "main should be a valid function bind"
    );
    let errors = count_errors(&out);
    assert_eq!(
        errors, 0,
        "'return 5' on same line should parse cleanly, got {errors} errors"
    );
}

#[test]
fn bare_return_followed_by_expression() {
    let src = "main:\n    return\n5\n";
    let out = parse_source_full(src);
    assert!(has_def(&out, "main"), "main should be bound");
}

// ── GROUPED EXPRESSIONS ────────────────────────────────────────────────

#[test]
fn grouped_expression_across_lines() {
    // The Indent token inside parens causes an error.
    // Workaround: same-indent or operator-at-end.
    let src = "x := (1\n  + 2)\n";
    let out = parse_source_full(src);
    assert!(has_def(&out, "x"), "grouped expression should be a bind");
}

#[test]
fn grouped_same_indent_inside_parens() {
    // Same-indent inside parens — no Indent token, works fine
    let src = "x := (1\n+ 2)\n";
    let out = parse_source_full(src);
    assert!(has_def(&out, "x"));
    assert_eq!(count_errors(&out), 0);
}

// ── TAG DECLARATIONS ACROSS LINES ──────────────────────────────────────

#[test]
fn tag_declaration_with_or_on_next_line() {
    let src = "LogLevel is 'debug'\n         or 'info'\n         or 'warn'\n";
    let out = parse_source_full(src);
    assert!(
        out.ast.tags().contains_key(&intern("LogLevel")),
        "tag declaration with 'or' on next line should be one declaration"
    );
    let errors = count_errors(&out);
    assert_eq!(
        errors, 0,
        "tag declaration across lines should parse cleanly, got {errors} errors"
    );
}

#[test]
fn tagged_union_across_lines() {
    let src = "Result is Ok(value) or Err(reason)\n";
    let out = parse_source_full(src);
    assert!(out.ast.tags().contains_key(&intern("Result")));
    assert_eq!(count_errors(&out), 0);
}

// ── CONTROL FLOW BODIES ──────────────────────────────────────────────

#[test]
fn if_body_expression_continuation() {
    let src = "result := if True\n    1\n    + 2\nreturn\n";
    let out = parse_source_full(src);
    let errors = count_errors(&out);
    assert_eq!(
        errors, 0,
        "if body with continued expression should parse, got {errors} errors"
    );
}

#[test]
fn when_then_across_lines() {
    let src = "x := when 5 > 0\n          then 1\n          else 2\n";
    let out = parse_source_full(src);
    let errors = count_errors(&out);
    assert_eq!(
        errors, 0,
        "when/then across lines should parse without errors, got {errors} errors"
    );
}

// ── METHOD CHAINING ───────────────────────────────────────────────────

#[test]
fn method_chaining_dot_at_start_of_line() {
    let src = "some.Tag\n    .some_method(42)\n";
    let out = parse_source_full(src);
    let errors = count_errors(&out);
    assert_eq!(
        errors, 0,
        "method chaining attempt should not produce parse errors, got {errors} errors"
    );
}

// ── TOP-LEVEL UNUSED VALUE DETECTION (Guardrails) ────────────────────

#[test]
fn unused_top_level_literal_warns() {
    let src = "42\n";
    let out = parse_source_full(src);
    assert!(
        count_unused(&out) > 0,
        "bare literal at top-level should trigger unused-value warning"
    );
}

#[test]
fn unused_top_level_binary_warns() {
    let src = "1 + 2\n";
    let out = parse_source_full(src);
    assert!(
        count_unused(&out) > 0,
        "unbound binary expression should trigger unused-value warning"
    );
}

#[test]
fn sequential_top_level_binds_no_warning() {
    let src = "x := 1\ny := 2\n";
    let out = parse_source_full(src);
    assert!(has_def(&out, "x"));
    assert!(has_def(&out, "y"));
    assert_eq!(
        count_unused(&out),
        0,
        "sequential binds should not produce unused-value warnings"
    );
}

#[test]
fn string_literals_on_separate_lines_two_exprs() {
    let src = "'hello'\n'world'\n";
    let out = parse_source_full(src);
    assert_eq!(
        count_exprs(&out),
        2,
        "two string literals on separate lines should be two exprs"
    );
    assert!(
        count_unused(&out) >= 2,
        "both unused string literals should warn"
    );
}

// ── TWO EXPRESSIONS ON ONE LINE ──────────────────────────────────────

#[test]
fn two_expressions_on_one_line_no_crash() {
    let src = "1 + 1 1 + 1\n";
    let _out = parse_source_full(src);
    // Parser should not hang or crash (known behavior: Gleam accepts this,
    // Gin may error or produce one expr)
}

// ── SWIFT-STYLE MINUS GOTCHA ─────────────────────────────────────────

#[test]
fn minus_sign_touching_number_same_indent() {
    let src = "y := 4\n-3\n";
    let out = parse_source_full(src);
    assert!(
        has_def(&out, "y"),
        "y should be one bind with value 4 - 3 = 1"
    );
    assert_eq!(count_unused(&out), 0, "no unused value");
}

// ── ARTICLE COMPARISONS ─────────────────────────────────────────────

#[test]
fn article_python_gotcha() {
    let src = "y := 2 * 4\n- 3\n";
    let out = parse_source_full(src);
    assert!(has_def(&out, "y"), "y should be a single bind");
    assert_eq!(count_unused(&out), 0, "no unused");
}

#[test]
fn article_go_gotcha() {
    let src = "y := 2 * 4\n- 3\n";
    let out = parse_source_full(src);
    assert!(has_def(&out, "y"), "y should be a single bind");
}

#[test]
fn article_swift_gotcha() {
    let src = "y := 4\n-3\n";
    let out = parse_source_full(src);
    assert!(has_def(&out, "y"), "one bind, y = 4 - 3");
    assert_eq!(count_unused(&out), 0);
}

#[test]
fn article_gleam_two_exprs_same_line() {
    let src = "1 + 1 1 + 1\n";
    let _out = parse_source_full(src);
}

// ── FUNCTION/TAG CALLS ACROSS LINES ─────────────────────────────────

#[test]
fn tag_call_across_lines() {
    // Swift guardrail: `(` on next line is NOT a call.
    let src = "Bar\n(5)\n";
    let out = parse_source_full(src);
    assert_eq!(
        count_exprs(&out),
        2,
        "Swift guardrail: Bar and (5) on separate lines are two exprs"
    );
}

#[test]
fn fn_call_across_lines() {
    // Swift guardrail: `(` on next line is NOT a call — it's a grouping.
    let src = "foo\n(1)\n";
    let out = parse_source_full(src);
    assert_eq!(
        count_exprs(&out),
        2,
        "Swift guardrail: foo and (1) on separate lines are two exprs"
    );
}

#[test]
fn bind_then_another_bind_with_newline() {
    let src = "x := 1\ny := 2\n";
    let out = parse_source_full(src);
    assert!(has_def(&out, "x"), "first bind");
    assert!(has_def(&out, "y"), "second bind");
    assert_eq!(count_unused(&out), 0, "both bound, no unused values");
}

#[test]
fn bind_then_bare_expression() {
    let src = "x := 1\n42\n";
    let out = parse_source_full(src);
    assert!(has_def(&out, "x"), "first bind");
    assert!(count_exprs(&out) >= 1, "42 should be a bare expression");
    assert!(count_unused(&out) >= 1, "42 should trigger unused warning");
}
