use ast::{
    Argument, Expr, Expression, Item, LiteralKind, MatchExpression, MatchPattern, SourceFile,
    Statement, SyntaxNodeKind, UnaryOperator,
};
use parser::{ParseFailure, ParseLimit, ParseLimits, ParseOptions};
use std::cell::Cell;

fn value(file: &SourceFile, index: usize) -> &Expression {
    let Item::Binding(binding) = &file.items[index].value else {
        panic!("binding")
    };
    &binding.value.as_ref().unwrap().expression
}

fn case(expression: &Expression) -> &MatchExpression {
    let Expr::Match(case) = &expression.value else {
        panic!("case: {expression:#?}")
    };
    case
}

#[test]
fn two_slot_selector_retains_integer_arms_without_semantic_metadata() {
    let source = concat!(
        "choose(index in 0 to 1) Result:\n",
        "    return when index is\n",
        "        0 then Zero\n",
        "        1 then One\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let Expr::Block(body) = &value(&file, 0).value else {
        panic!("function body")
    };
    let Statement::Return(returned) = &body.statements[0].value else {
        panic!("return")
    };
    let case = case(returned.value.as_ref().unwrap());
    assert_eq!(case.arms.value.len(), 2);
    for (arm, (pattern, body)) in case.arms.value.iter().zip([("0", "Zero"), ("1", "One")]) {
        let MatchPattern::Integer(expression) = &arm.value.pattern.value else {
            panic!("integer pattern")
        };
        assert_eq!(expression.value, Expr::Literal(LiteralKind::Integer));
        assert_eq!(expression.span_id, arm.value.pattern.span_id);
        assert_eq!(
            file.span_table.get(expression.span_id).extract(source),
            pattern
        );
        assert_eq!(
            file.span_table.get(arm.span_id).extract(source),
            format!("{pattern} then {body}")
        );
        assert_eq!(
            file.span_table.get(arm.value.then_keyword).extract(source),
            "then"
        );
        assert!(matches!(&arm.value.body.value, Expr::Name(name) if name == body));
    }
    assert_eq!(
        file.span_table.get(case.when_keyword).extract(source),
        "when"
    );
    assert_eq!(file.span_table.get(case.is_keyword).extract(source), "is");
}

#[test]
fn mixed_patterns_keep_unbounded_spelling_sign_spans_and_nominal_forms() {
    let source = concat!(
        "--- hé 🦀\ninspect: when index is\n",
        "    00_001 then Zero\n",
        "    - 9_999_999_999_999_999_999_999_999_999 then Negative\n",
        "    -0 then MinusZero\n",
        "    0 then Zero\n",
        "    0 then Duplicate\n",
        "    Empty then None\n",
        "    Empty() then None\n",
        "    Full(value) then Found(value)\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let case = case(value(&file, 0));
    assert_eq!(case.arms.value.len(), 8);
    for (index, spelling) in [
        (0, "00_001"),
        (1, "- 9_999_999_999_999_999_999_999_999_999"),
        (2, "-0"),
    ] {
        let pattern = &case.arms.value[index].value.pattern;
        let MatchPattern::Integer(expression) = &pattern.value else {
            panic!("integer pattern")
        };
        let span = file.span_table.get(pattern.span_id);
        let start = source.find(spelling).unwrap();
        assert_eq!(span.to_range(), start..start + spelling.len());
        let literal = match &expression.value {
            Expr::Literal(LiteralKind::Integer) => expression,
            Expr::Unary { operator, operand } => {
                assert_eq!(operator.value, UnaryOperator::Negate);
                let sign = file.span_table.get(operator.span_id);
                assert_eq!(sign.to_range(), start..start + 1);
                let hit = file.node_at_offset(start).unwrap();
                assert!(matches!(hit.node.kind, SyntaxNodeKind::UnaryOperator(_)));
                assert_eq!(hit.node.span, sign);
                operand
            }
            other => panic!("literal or unary literal: {other:?}"),
        };
        assert_eq!(literal.value, Expr::Literal(LiteralKind::Integer));
        let literal_span = file.span_table.get(literal.span_id);
        assert_eq!(
            literal_span.extract(source),
            spelling.trim_start_matches('-').trim_start()
        );
        for offset in literal_span.to_range() {
            let hit = file.node_at_offset(offset).unwrap();
            assert!(matches!(
                hit.node.kind,
                SyntaxNodeKind::Expression(node) if std::ptr::eq(node, literal)
            ));
            assert_eq!(hit.node.span, literal_span);
            assert!(hit.ancestors.iter().any(|node| matches!(
                node.kind,
                SyntaxNodeKind::MatchPattern(node) if std::ptr::eq(node, pattern)
            )));
            assert_eq!(hit.declarations.len(), 1);
            assert_eq!(hit.declarations[0].name, "inspect");
        }
    }
    for (index, fields_present) in [(5, false), (6, true), (7, true)] {
        let pattern = &case.arms.value[index].value.pattern;
        let MatchPattern::Variant(variant) = &pattern.value else {
            panic!("nominal pattern")
        };
        assert_eq!(variant.fields.is_some(), fields_present);
        let hit = file
            .node_at_offset(file.span_table.get(pattern.span_id).start())
            .unwrap();
        assert!(hit.ancestors.iter().any(|node| matches!(
            node.kind,
            SyntaxNodeKind::VariantPattern(node) if std::ptr::eq(node, variant)
        )));
    }
    let Expr::Call { arguments, .. } = &case.arms.value[7].value.body.value else {
        panic!("ordinary argument syntax")
    };
    assert!(matches!(
        &arguments.value[0].value,
        Argument::Positional(expression) if expression.value == Expr::Name("value".into())
    ));
}

#[test]
fn numeric_cases_reuse_root_local_nested_and_then_block_continuations() {
    let source = concat!(
        "inspect: when index is\n",
        "    0 then when other is\n",
        "        -1 then None\n",
        "        1 then One\n",
        "    1 then\n",
        "        local := when other is\n",
        "            0 then Zero\n",
        "            1 then One\n",
        "        Empty := slot\n",
        "        return when other is\n",
        "            0 then Zero\n",
        "            1 then local\n",
        "later := 2\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let root = case(value(&file, 0));
    assert_eq!(case(&root.arms.value[0].value.body).arms.value.len(), 2);
    let Expr::Block(body) = &root.arms.value[1].value.body.value else {
        panic!("then block")
    };
    assert_eq!(body.statements.len(), 3);
    assert!(matches!(body.statements[0].value, Statement::Binding(_)));
    assert!(matches!(
        body.statements[1].value,
        Statement::Destructure(_)
    ));
    assert!(matches!(body.statements[2].value, Statement::Return(_)));
    assert_eq!(file.items.len(), 2);
}

#[test]
fn computed_and_malformed_signed_patterns_diagnose_and_preserve_later_declarations() {
    for (pattern, diagnostic) in [
        ("1 + 1", "computed patterns"),
        ("1 to 2", "ranges"),
        ("1 if ready", "guards"),
        ("1()", "computed patterns"),
        ("(1)", "computed patterns"),
        ("index", "computed patterns"),
        ("compute()", "computed patterns"),
        ("-index", "integer literal after a single '-'"),
        ("-(1)", "integer literal after a single '-'"),
        ("- -1", "integer literal after a single '-'"),
        ("+1", "signed patterns"),
        ("-+1", "signed patterns"),
        ("-", "integer literal after a single '-'"),
        ("-1.0", "integer literal after a single '-'"),
        ("1.0", "integer literal pattern"),
    ] {
        let source = format!("inspect: when index is\n    {pattern} then Zero\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|item| item.message.contains(diagnostic)),
            "{source}"
        );
        assert!(
            output
                .file
                .declaration_outline()
                .any(|item| item.name == "later"),
            "{source}"
        );
        assert!(parser::parse_strict(&source).is_err(), "{source}");
    }
    for pattern in ["0", "-1"] {
        let source = format!("main:\n    {pattern} := index\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors());
        assert!(
            output
                .file
                .declaration_outline()
                .any(|item| item.name == "later")
        );
    }
}

#[test]
fn malformed_numeric_pattern_punctuation_makes_progress_with_large_diagnostic_budget() {
    let tokens = [
        "0", "-", "+", ")", "(", ",", ".", "then", "is", "when", ":", ":=",
    ];
    for first in tokens {
        for second in tokens {
            for third in tokens {
                let source = format!(
                    "inspect: when index is\n    {first} {second} {third} then Zero\nlater := 2\n"
                );
                let output = parser::parse_with_options(
                    &source,
                    ParseOptions {
                        limits: ParseLimits {
                            diagnostics_max: 1000,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                )
                .expect(&source);
                assert!(
                    output
                        .file
                        .declaration_outline()
                        .any(|item| item.name == "later"),
                    "{source}"
                );
                for offset in 0..=source.len() {
                    output.file.node_at_offset(offset).unwrap();
                }
            }
        }
    }
}

#[test]
fn literal_pattern_depth_and_many_arms_obey_existing_budgets() {
    for (pattern, depth) in [("0", 2), ("-0", 3)] {
        let source = format!("inspect: when index is\n    {pattern} then Zero\n");
        for (maximum, passes) in [(depth - 1, false), (depth, true)] {
            let result = parser::parse_with_options(
                &source,
                ParseOptions {
                    limits: ParseLimits {
                        expression_depth_max: maximum,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
            if passes {
                assert!(!result.unwrap().has_errors());
            } else {
                assert!(matches!(
                    result,
                    Err(ParseFailure::LimitExceeded {
                        limit: ParseLimit::ExpressionDepth,
                        ..
                    })
                ));
            }
        }
    }
    let source = format!(
        "inspect: when index is\n{}",
        "    -0 then Zero\n".repeat(1000)
    );
    assert_eq!(
        case(value(&parser::parse_strict(&source).unwrap(), 0))
            .arms
            .value
            .len(),
        1000
    );
    let checks = Cell::new(0);
    let count = || {
        checks.set(checks.get() + 1);
        false
    };
    parser::parse_with_options(
        &source,
        ParseOptions {
            is_cancelled: Some(&count),
            ..Default::default()
        },
    )
    .unwrap();
    let budget = checks.get() - 2;
    checks.set(0);
    let cancel = || {
        checks.set(checks.get() + 1);
        checks.get() > budget
    };
    assert!(matches!(
        parser::parse_with_options(
            &source,
            ParseOptions {
                is_cancelled: Some(&cancel),
                ..Default::default()
            },
        ),
        Err(ParseFailure::Cancelled { .. })
    ));
}
