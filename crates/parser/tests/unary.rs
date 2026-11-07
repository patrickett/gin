use ast::{BinaryOperator, Expr, Item, UnaryOperator};

#[test]
fn negation_wraps_postfix_chains_and_binds_before_binary_operators() {
    let file =
        parser::parse_strict("value := -service.read(2).field * -(-limit + 1)\n").unwrap();
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("written binding");
    };
    let Expr::Binary {
        left,
        operator,
        right,
    } = &binding.value.as_ref().unwrap().expression.value
    else {
        panic!("multiplication");
    };
    assert_eq!(operator.value, BinaryOperator::Multiply);
    let Expr::Unary { operator, operand } = &left.value else {
        panic!("negative complete postfix expression");
    };
    assert_eq!(operator.value, UnaryOperator::Negate);
    assert!(matches!(operand.value, Expr::Member { .. }));
    assert_eq!(
        file.span_table.get(left.span_id).extract(&file.text),
        "-service.read(2).field"
    );
    let Expr::Unary { operand, .. } = &right.value else {
        panic!("negative group");
    };
    assert!(matches!(operand.value, Expr::Group(_)));
}

#[test]
fn negative_named_arguments_and_missing_operands_keep_their_source_shape() {
    for source in [
        "f(value: -2)\n",
        "value := -\nnext := 2\n",
        "value := - -limit\n",
    ] {
        let output = parser::parse(source).unwrap();
        if source.contains(":= -\n") {
            assert!(output.has_errors());
            let names: Vec<_> = output
                .file
                .declaration_outline()
                .map(|entry| entry.name)
                .collect();
            assert_eq!(names, ["value", "next"]);
        } else {
            assert!(!output.has_errors(), "{:?}", output.diagnostics);
        }
    }
}

#[test]
fn prefix_chains_obey_the_expression_depth_budget() {
    let options = parser::ParseOptions {
        limits: parser::ParseLimits {
            expression_depth_max: 2,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(
        parser::parse_with_options("value := -x\n", options)
            .unwrap()
            .diagnostics
            .is_empty()
    );
    assert!(matches!(
        parser::parse_with_options("value := - -x\n", options),
        Err(parser::ParseFailure::LimitExceeded {
            limit: parser::ParseLimit::ExpressionDepth,
            ..
        })
    ));
}

#[test]
fn a_prefix_without_a_positional_argument_is_not_silently_dropped() {
    for source in ["f(-)", "f(-\n)", "f(-"] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors(), "{source}");
        let Item::Expression(expression) = &output.file.items[0].value else {
            panic!("call structure survives");
        };
        let Expr::Call { arguments, .. } = &expression.value else {
            panic!("written call");
        };
        assert_eq!(arguments.value.len(), 1);
        let ast::Argument::Positional(argument) = &arguments.value[0].value else {
            panic!("positional argument");
        };
        let Expr::Unary { operand, .. } = &argument.value else {
            panic!("written unary prefix");
        };
        assert_eq!(operand.value, Expr::Missing);
        assert!(parser::parse_strict(source).is_err());
    }
}

#[test]
fn a_prefix_before_a_named_argument_is_not_moved_onto_its_value() {
    for source in ["f(-label: 2)", "f(-\nlabel: 2)"] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors(), "{source}");
        for offset in 0..=source.len() {
            let hit = output.file.node_at_offset(offset).unwrap();
            assert!(source.get(hit.node.span.to_range()).is_some());
        }
        assert!(parser::parse_strict(source).is_err());
    }
}
