//! `"..."` is a format string. With nothing to interpolate it is plain text, the same literal as
//! `'...'`; an interpolation hole (`\( ... )`) is reported, not parsed, until its grammar is
//! decided.
use ast::{Expr, Item, LiteralKind, SourceFile};

fn value(file: &SourceFile) -> &ast::Expression {
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("binding")
    };
    &binding.value.as_ref().unwrap().expression
}

fn text<'a>(source: &'a str, file: &SourceFile, expression: &ast::Expression) -> &'a str {
    file.span_table.get(expression.span_id).extract(source)
}

#[test]
fn a_double_quoted_run_is_one_string_literal() {
    let source = "architecture: \"aarch64\"\n";
    let file = parser::parse_strict(source).unwrap();
    let literal = value(&file);
    assert_eq!(literal.value, Expr::Literal(LiteralKind::String));
    assert_eq!(text(source, &file, literal), "\"aarch64\"");
}

/// The text is not lexed as code, so characters that would be an error or a quote elsewhere are
/// just text.
#[test]
fn what_would_lex_as_something_else_is_text_inside_quotes() {
    for body in ["it's", "a - b", "x86_64", "tab\tstop", "世界", "a, b: c"] {
        let source = format!("note: \"{body}\"\n");
        let file = parser::parse_strict(&source).expect(&source);
        assert_eq!(text(&source, &file, value(&file)), format!("\"{body}\""));
    }
}

#[test]
fn an_escaped_quote_does_not_end_the_string() {
    let source = "note: \"say \\\"hi\\\" now\"\n";
    let file = parser::parse_strict(source).unwrap();
    assert_eq!(text(source, &file, value(&file)), "\"say \\\"hi\\\" now\"");
}

#[test]
fn an_interpolation_hole_is_reported_not_parsed() {
    let source = "note: \"value is \\(count) now\"\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    assert!(parser::parse_strict(source).is_err());
    // A backslash that does not open a hole is only text.
    assert!(parser::parse_strict("note: \"a \\ b\"\n").is_ok());
}

#[test]
fn an_unterminated_string_is_a_lexical_error_that_stops_at_the_line() {
    let source = "note: \"never closed\nlater: 1\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    assert!(
        output
            .file
            .declaration_outline()
            .any(|entry| entry.name == "later")
    );
}

#[test]
fn single_and_double_quotes_do_not_mix_up() {
    let source = "a: 'say \"hi\"'\nb: \"it's\"\n";
    let file = parser::parse_strict(source).unwrap();
    assert_eq!(file.items.len(), 2);
}

/// Call arguments are separated by commas or newlines, as list elements and parameters are, and
/// the closing parenthesis may sit on its own line.
#[test]
fn call_arguments_may_be_separated_by_newlines() {
    for source in [
        "x: make(index: 0, width: 64)\n",
        "x: make(\n    index: 0\n    width: 64\n)\n",
        "x: make(index: 0\n    width: 64)\n",
        "x: make(\n    index: 0,\n    width: 64,\n)\n",
    ] {
        let file = parser::parse_strict(source).expect(source);
        let Expr::Call { arguments, .. } = &value(&file).value else {
            panic!("call in {source:?}")
        };
        assert_eq!(arguments.value.len(), 2, "{source:?}");
    }
}

// Borrow expressions and `deref`: `ref place` and `mut place` create a borrow in a value
// position, and `deref value` copies the value a borrow refers to.

fn first_value(source: &str) -> ast::Expression {
    let file = parser::parse_strict(source).expect(source);
    value(&file).clone()
}

#[test]
fn ref_and_mut_create_borrows_in_value_positions() {
    for source in [
        "x: Region(data: ref self.address.[0; len])\n",
        "x: Region(region: mut storage.[0; len], storage: mut buffer.storage)\n",
        "x: ref y\n",
    ] {
        first_value(source);
    }
    let cast = first_value("pointer: mut reserve_bytes(1) as @Byte\n");
    // The borrow binds tighter than the cast: `(mut reserve_bytes(1)) as @Byte`.
    let Expr::Cast { value, .. } = &cast.value else {
        panic!("cast: {cast:?}")
    };
    assert!(matches!(value.value, Expr::PrefixedType(_)), "{value:?}");
}

#[test]
fn deref_is_a_prefix_operator() {
    for (source, operand_is_group) in [
        ("x: deref value\n", false),
        ("x: deref(ref stats)\n", true),
        ("x: Copied(value: deref self)\n", false),
    ] {
        let expression = first_value(source);
        let target = match &expression.value {
            Expr::Call { arguments, .. } => match &arguments.value[0].value {
                ast::Argument::Named { value, .. } => value.clone(),
                ast::Argument::Positional(value) => value.clone(),
                ast::Argument::Typed { annotation, .. } => annotation.clone(),
            },
            _ => expression,
        };
        let Expr::Unary { operator, operand } = &target.value else {
            panic!("unary in {source:?}: {target:?}")
        };
        assert_eq!(operator.value, ast::UnaryOperator::Deref, "{source:?}");
        assert_eq!(matches!(operand.value, Expr::Group(_)), operand_is_group);
    }
}

/// `deref value + 1` is `(deref value) + 1`.
#[test]
fn deref_binds_like_unary_minus() {
    let expression = first_value("x: deref value + 1\n");
    let Expr::Binary { left, .. } = &expression.value else {
        panic!("binary: {expression:?}")
    };
    assert!(matches!(left.value, Expr::Unary { .. }));
}
