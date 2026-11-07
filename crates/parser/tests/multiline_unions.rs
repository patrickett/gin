use ast::{
    Argument, BinaryOperator, Expr, Expression, Item, SourceFile, SyntaxNodeKind, TypeBody,
};
use parser::{ParseFailure, ParseLimit, ParseLimits, ParseOptions, parse_with_options};

const SUM_TYPES: &str = include_str!("../../test-fixtures/fixtures/type_sum_generic_payloads.gin");

fn body(file: &SourceFile, index: usize) -> &Expression {
    let Item::Type(declaration) = &file.items[index].value else {
        panic!("expected retained type: {:?}", file.items[index]);
    };
    let TypeBody::Is(expression) = &declaration.body.value else {
        panic!("expected written is expression");
    };
    expression
}

// Compare source structure without comparing snapshot-specific SpanIds. Preorder markers
// include arity, so grouping, association, calls, and names cannot collapse into a variant list.
fn shape(file: &SourceFile, expression: &Expression) -> Vec<String> {
    let mut pending = vec![expression];
    let mut result = Vec::new();
    for _ in 0..file.text.len() {
        let Some(expression) = pending.pop() else {
            return result;
        };
        match &expression.value {
            Expr::Name(name) => result.push(format!("name:{name}")),
            Expr::Binary {
                left,
                operator,
                right,
            } => {
                result.push(format!(
                    "binary:{:?}:{}",
                    operator.value,
                    file.span_table.get(operator.span_id).extract(&file.text)
                ));
                pending.push(right);
                pending.push(left);
            }
            Expr::Call { callee, arguments } => {
                result.push(format!("call:{}", arguments.value.len()));
                for argument in arguments.value.iter().rev() {
                    let Argument::Positional(expression) = &argument.value else {
                        panic!("fixture uses positional arguments");
                    };
                    pending.push(expression);
                }
                pending.push(callee);
            }
            Expr::Group(inner) => {
                result.push("group".into());
                pending.push(inner);
            }
            other => panic!("unexpected synthesized or recovered expression: {other:?}"),
        }
    }
    panic!("expression traversal exceeded source byte count");
}

fn assert_same_body(inline: &str, multiline: &str) {
    let expected = parser::parse_strict(inline).expect(inline);
    let actual = parser::parse_strict(multiline).expect(multiline);
    assert_eq!(actual.text.as_ref(), multiline);
    assert_eq!(actual.items.len(), 1);
    assert_eq!(
        shape(&actual, body(&actual, 0)),
        shape(&expected, body(&expected, 0))
    );
}

#[test]
fn all_three_union_line_break_positions_preserve_written_names() {
    for source in [
        "Choice is First\n    or Second\n",
        "Choice is\n    First\n    or Second\n",
        "Choice is First or\n    Second\n",
        "Choice is First\nor Second\n",
        "Choice is\n    First\nor Second\n",
    ] {
        assert_same_body("Choice is First or Second\n", source);
    }
}

#[test]
fn comments_blank_lines_and_tabs_do_not_terminate_union_continuations() {
    for source in [
        "Choice is First -- first\n\n    -- next\n    or Second\n",
        "Choice is\n    -- first\n\n    First\n    -- next\n\n    or Second\n",
        "Choice is First or -- next\n\n    -- payload\n    Second\n",
        "Choice is\n\tFirst\n\tor Second\n",
    ] {
        assert_same_body("Choice is First or Second\n", source);
    }
}

#[test]
fn real_generic_union_fixture_bytes_and_indented_calls_have_the_same_shape() {
    let inline = SUM_TYPES
        .split_inclusive('\n')
        .find(|line| line.starts_with("Attempt(element) is "))
        .expect("real generic union declaration");
    assert_eq!(
        inline,
        "Attempt(element) is Block(element) or Failed(Error)\n"
    );
    for source in [
        "Attempt(element) is Block(element)\n    or Failed(Error)\n",
        "Attempt(element) is\n    Block(element)\n    or Failed(Error)\n",
        "Attempt(element) is Block(element) or\n    Failed(Error)\n",
    ] {
        assert_same_body(inline, source);
        let file = parser::parse_strict(source).unwrap();
        let Item::Type(declaration) = &file.items[0].value else {
            unreachable!();
        };
        assert_eq!(declaration.parameters.as_ref().unwrap().value.len(), 1);
        assert_eq!(
            declaration.parameters.as_ref().unwrap().value[0]
                .value
                .name
                .value,
            "element"
        );
    }
}

#[test]
fn union_continuations_keep_inline_precedence_association_and_groups() {
    for (inline, multiline) in [
        (
            "Choice is First or Second or Third\n",
            "Choice is First\n    or Second\n    or Third\n",
        ),
        (
            "Choice is First and Second or Third and Fourth\n",
            "Choice is First and Second\n    or Third and Fourth\n",
        ),
        (
            "Choice is First or Second and Third or Fourth\n",
            "Choice is\n    First\n    or Second and Third\n    or Fourth\n",
        ),
        (
            "Choice is (First or Second) or Box(Third)\n",
            "Choice is (First or Second)\n    or Box(Third)\n",
        ),
    ] {
        assert_same_body(inline, multiline);
    }
}

#[test]
fn following_has_blocks_and_bindings_keep_separate_outlines_and_query_spans() {
    let source = "\
Choice is
    First
    or Box(Second)
Choice has
    value Int
later := 2
";
    let file = parser::parse_strict(source).expect("union followed by extension");
    assert_eq!(file.items.len(), 3);
    let outline: Vec<_> = file.declaration_outline().collect();
    assert_eq!(
        outline.iter().map(|entry| entry.name).collect::<Vec<_>>(),
        ["Choice", "Choice", "later"]
    );
    assert_eq!(
        outline[0].span.extract(source),
        "Choice is\n    First\n    or Box(Second)"
    );
    assert_eq!(outline[1].span.extract(source), "Choice has\n    value Int");
    assert_eq!(outline[2].span.extract(source), "later := 2");
    assert_eq!(
        file.span_table.get(body(&file, 0).span_id).extract(source),
        "First\n    or Box(Second)"
    );
    for (spelling, owner) in [("Second", "Choice"), ("value", "value"), ("later", "later")] {
        let offset = source.find(spelling).unwrap();
        let hit = file.node_at_offset(offset).unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::Name(name) if name == spelling));
        assert_eq!(hit.node.span.extract(source), spelling);
        assert_eq!(hit.declarations.last().unwrap().name, owner);
        for ancestor in hit.ancestors {
            assert!(ancestor.span.start() <= hit.node.span.start());
            assert!(hit.node.span.end() <= ancestor.span.end());
        }
    }
    let operator = file.node_at_offset(source.find("or Box").unwrap()).unwrap();
    assert!(matches!(
        operator.node.kind,
        SyntaxNodeKind::BinaryOperator(operator) if operator.value == BinaryOperator::Or
    ));
    assert_eq!(operator.node.span.extract(source), "or");
    assert_eq!(operator.declarations.len(), 1);
}

#[test]
fn missing_alternatives_keep_the_type_hole_and_later_declarations() {
    for prefix in [
        "Choice is First or",
        "Choice is First\n    or",
        "Choice is\n    First\n    or",
    ] {
        for suffix in ["", "\nNext has\n    value Int\nlater := 2\n"] {
            let source = format!("{prefix}{suffix}");
            let output = parser::parse(&source).expect("recoverable missing alternative");
            assert!(output.has_errors(), "{source}");
            assert!(parser::parse_strict(&source).is_err());
            let Expr::Binary {
                left,
                operator,
                right,
            } = &body(&output.file, 0).value
            else {
                panic!("retain written union and missing right operand: {source}");
            };
            assert_eq!(left.value, Expr::Name("First".into()));
            assert_eq!(operator.value, BinaryOperator::Or);
            assert_eq!(right.value, Expr::Missing);
            let span = output.file.span_table.get(right.span_id);
            assert_eq!(span.start(), span.end());
            assert!(span.start() <= source.len());
            let names: Vec<_> = output
                .file
                .declaration_outline()
                .map(|entry| entry.name)
                .collect();
            if suffix.is_empty() {
                assert_eq!(names, ["Choice"]);
            } else {
                assert_eq!(names, ["Choice", "Next", "later"]);
            }
            for diagnostic in &output.diagnostics {
                assert!(source.get(diagnostic.span.to_range()).is_some());
            }
        }
    }
}

#[test]
fn continuation_is_not_a_general_multiline_expression_rule() {
    for source in [
        "value := First\n    or Second\n",
        "value := First or\n    Second\n",
        "Choice is First\n    and Second\n",
        "Choice is First and\n    Second\n",
        "Choice is First +\n    Second\n",
        "Choice is Box(First\n    or Second)\n",
    ] {
        let output = parser::parse(source).expect("bounded invalid source");
        assert!(
            output.has_errors(),
            "must not broaden expression grammar: {source}"
        );
        assert!(parser::parse_strict(source).is_err(), "{source}");
    }
}

#[test]
fn existing_nested_newline_operators_keep_their_written_shape() {
    for source in [
        "Choice is (First\n    or Second)\n",
        "Choice is (First or\n    Second)\n",
    ] {
        assert_same_body("Choice is (First or Second)\n", source);
    }
    assert_same_body(
        "Choice is Box(First or Second)\n",
        "Choice is Box(First or\n    Second)\n",
    );
}

#[test]
fn trailing_layout_keeps_the_missing_alternative_at_the_eof_cursor() {
    for source in [
        "Choice is First or\n",
        "Choice is First\n    or\n\n    ",
        "Choice is\n    First or\n    -- hé 🦀\n",
        "Choice is\n    ",
    ] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors());
        let hit = output.file.node_at_offset(source.len()).unwrap();
        assert!(
            matches!(
                hit.node.kind,
                SyntaxNodeKind::Expression(expression) if expression.value == Expr::Missing
            ),
            "{source}: {hit:?}"
        );
        assert_eq!(hit.node.span.start(), source.len());
        assert_eq!(hit.node.span.end(), source.len());
        assert_eq!(hit.declarations[0].name, "Choice");
    }
}

#[test]
fn multiline_union_obeys_expression_depth_limit() {
    let source = "Choice is\n    First\n    or Second\n    or Third\n";
    let options = ParseOptions {
        limits: ParseLimits {
            expression_depth_max: 2,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(matches!(
        parse_with_options(source, options),
        Err(ParseFailure::LimitExceeded {
            limit: ParseLimit::ExpressionDepth,
            ..
        })
    ));
}

#[test]
fn multiline_union_obeys_cancellation() {
    let source = "Choice is\n    First\n    or Second\n    or Third\n";
    let cancelled = || true;
    let options = ParseOptions {
        is_cancelled: Some(&cancelled),
        ..Default::default()
    };
    assert!(matches!(
        parse_with_options(source, options),
        Err(ParseFailure::Cancelled { .. })
    ));
}

#[test]
fn cancellation_is_observable_late_in_a_long_union_continuation() {
    use std::cell::Cell;

    let source = format!("Choice is First\n{}    or Second\n", "\n".repeat(2_000));
    let baseline = Cell::new(0usize);
    let count = || {
        baseline.set(baseline.get() + 1);
        false
    };
    let options = ParseOptions {
        is_cancelled: Some(&count),
        ..Default::default()
    };
    assert!(!parse_with_options(&source, options).unwrap().has_errors());
    let cutoff = baseline.get() - 1_000;
    let visits = Cell::new(0usize);
    let cancelled = || {
        visits.set(visits.get() + 1);
        visits.get() == cutoff
    };
    let options = ParseOptions {
        is_cancelled: Some(&cancelled),
        ..Default::default()
    };
    assert!(matches!(
        parse_with_options(&source, options),
        Err(ParseFailure::Cancelled { .. })
    ));
    assert_eq!(visits.get(), cutoff);
}
