use ast::{Expr, Item, LiteralKind};
use parser::{ParseFailure, ParseLimit, ParseLimits, ParseOptions};

#[test]
fn numeric_syntax_is_not_rejected_by_backend_ranges_or_float_scratch_limits() {
    let integer = "9".repeat(1_000);
    let float = format!("{integer}.0");
    let source = format!("integer := {integer}\nfloat := {float}\n");
    let output = parser::parse(&source).expect("source fits parser budgets");
    assert!(!output.has_errors(), "{:?}", output.diagnostics);
    for (item, spelling, expected) in [
        (
            &output.file.items[0],
            integer.as_str(),
            LiteralKind::Integer,
        ),
        (&output.file.items[1], float.as_str(), LiteralKind::Float),
    ] {
        let Item::Binding(binding) = &item.value else {
            panic!("source binding");
        };
        let expression = &binding.value.as_ref().unwrap().expression;
        assert_eq!(expression.value, Expr::Literal(expected));
        assert_eq!(
            output
                .file
                .span_table
                .get(expression.span_id)
                .extract(&source),
            spelling
        );
    }
}

#[test]
fn malformed_spelling_becomes_a_bounded_diagnostic_in_the_parser_adapter() {
    for (spelling, code, expected) in [
        ("1__2", "lex-invalid-integer", LiteralKind::Integer),
        ("1.2_", "lex-invalid-float", LiteralKind::Float),
    ] {
        let source = format!("value := {spelling}\nnext := 2\n");
        let output = parser::parse(&source).unwrap();
        assert_eq!(output.diagnostics.len(), 1);
        assert_eq!(output.diagnostics[0].code.slug(), code);
        assert_eq!(output.diagnostics[0].span.extract(&source), spelling);
        assert_eq!(output.file.items.len(), 2);
        let Item::Binding(binding) = &output.file.items[0].value else {
            panic!("recognizable source binding survives malformed spelling");
        };
        assert_eq!(
            binding.value.as_ref().unwrap().expression.value,
            Expr::Literal(expected)
        );
        assert!(parser::parse_strict(&source).is_err());
    }
}

#[test]
fn unterminated_string_preserves_its_binding_under_a_single_diagnostic_budget() {
    let source = "x := 'bad\nnext := 2\n";
    let options = ParseOptions {
        limits: ParseLimits {
            diagnostics_max: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    let output = parser::parse_with_options(source, options)
        .expect("one malformed literal consumes one diagnostic");
    assert_eq!(output.diagnostics.len(), 1);
    assert_eq!(output.diagnostics[0].code.slug(), "lex-unclosed-string");
    assert_eq!(output.diagnostics[0].span.to_range(), 5..9);
    assert_eq!(output.file.items.len(), 2);
    let Item::Binding(binding) = &output.file.items[0].value else {
        panic!("unterminated literal must not discard its binding");
    };
    let expression = &binding.value.as_ref().unwrap().expression;
    assert_eq!(expression.value, Expr::Literal(LiteralKind::String));
    assert_eq!(
        output
            .file
            .span_table
            .get(expression.span_id)
            .extract(source),
        "'bad"
    );
    let Item::Binding(next) = &output.file.items[1].value else {
        panic!("following declaration survives");
    };
    assert_eq!(next.name.value, "next");
    assert!(output.into_result().is_err());
    let diagnostics = parser::parse_strict(source).unwrap_err();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code.slug(), "lex-unclosed-string");
}

#[test]
fn unknown_scalar_markers_get_rich_diagnostics_only_in_the_adapter() {
    let source = "first := 1\r\nnext := 2\né";
    let output = parser::parse(source).unwrap();
    assert_eq!(output.diagnostics.len(), 2);
    assert_eq!(output.file.items.len(), 2);
    for (diagnostic, spelling) in output.diagnostics.iter().zip(["\r", "é"]) {
        assert_eq!(diagnostic.code.slug(), "lex-unexpected-character");
        assert_eq!(diagnostic.span.extract(source), spelling);
    }
}

#[test]
fn unknown_tokens_count_toward_work_budgets_before_diagnostic_materialization() {
    let options = ParseOptions {
        limits: ParseLimits {
            tokens_max: 2,
            diagnostics_max: 10,
            ..Default::default()
        },
        ..Default::default()
    };
    let failure = parser::parse_with_options("\r\r\r", options).unwrap_err();
    let ParseFailure::LimitExceeded { limit, span, .. } = failure else {
        panic!("work-budget refusal");
    };
    assert_eq!(limit, ParseLimit::Tokens);
    assert_eq!(span.to_range(), 2..3);
}

#[test]
fn names_and_named_arguments_get_their_spelling_from_the_snapshot() {
    let source = "value := module.call(first, label: Some)\n#todo\n";
    let output = parser::parse(source).unwrap();
    assert!(!output.has_errors());
    let Item::Binding(binding) = &output.file.items[0].value else {
        panic!("source binding");
    };
    let Expr::Call { callee, arguments } = &binding.value.as_ref().unwrap().expression.value else {
        panic!("source call");
    };
    let Expr::Member { base, name } = &callee.value else {
        panic!("source member access");
    };
    assert_eq!(base.value, Expr::Name("module".into()));
    assert_eq!(name.value, "call");
    let ast::Argument::Named { name, value } = &arguments.value[1].value else {
        panic!("named argument");
    };
    assert_eq!(name.value, "label");
    assert_eq!(value.value, Expr::Name("Some".into()));
    assert!(matches!(
        output.file.items[1].value,
        Item::Expression(ast::Spanned {
            value: Expr::Hole,
            ..
        })
    ));
}
