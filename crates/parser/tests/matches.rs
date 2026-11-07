use ast::{
    Expr, Expression, Item, MatchExpression, MatchPattern, SourceFile, Statement, SyntaxNodeKind,
    VariantPattern,
};

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

fn variant(pattern: &MatchPattern) -> &VariantPattern {
    let MatchPattern::Variant(variant) = pattern else {
        panic!("variant: {pattern:#?}")
    };
    variant
}

#[test]
fn ordinary_cases_in_returns_and_local_initializers_keep_source_facts() {
    let source = concat!(
        "Slot(element) is Empty or Full(element)\n",
        "inspect(element Type, slot Slot(element)) Result(element):\n",
        "    result := when slot is\n",
        "        Empty then None\n",
        "        Full(value) then Found(value: value)\n",
        "    return when slot is\n",
        "        Empty() then None\n",
        "        Full(value, value) then value\n",
        "later := 2\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let Expr::Block(body) = &value(&file, 1).value else {
        panic!("body")
    };
    assert_eq!(body.statements.len(), 2);
    let Statement::Binding(local) = &body.statements[0].value else {
        panic!("local")
    };
    let first = case(&local.value.as_ref().unwrap().expression);
    assert!(
        variant(&first.arms.value[0].value.pattern.value)
            .fields
            .is_none()
    );
    let Statement::Return(returned) = &body.statements[1].value else {
        panic!("return")
    };
    let second = case(returned.value.as_ref().unwrap());
    assert!(
        variant(&second.arms.value[0].value.pattern.value)
            .fields
            .as_ref()
            .unwrap()
            .binders
            .value
            .is_empty()
    );
    let binders = &variant(&second.arms.value[1].value.pattern.value)
        .fields
        .as_ref()
        .unwrap()
        .binders
        .value;
    assert_eq!(binders[0].value, binders[1].value);
    assert_ne!(binders[0].span_id, binders[1].span_id);
    assert_eq!(
        file.declaration_outline()
            .map(|entry| entry.name)
            .collect::<Vec<_>>(),
        ["Slot", "inspect", "later"]
    );
    for (id, spelling) in [
        (first.when_keyword, "when"),
        (first.is_keyword, "is"),
        (first.arms.value[0].value.then_keyword, "then"),
    ] {
        assert_eq!(file.span_table.get(id).extract(source), spelling);
    }
}

#[test]
fn nested_cases_and_ordinary_arm_blocks_share_the_iterative_stack() {
    let source = concat!(
        "inspect(slot Slot): when slot is\n",
        "    Empty then None\n",
        "    Full(value) then\n",
        "        Empty := slot\n",
        "        local := when value is\n",
        "            Empty then None\n",
        "            Full(other) then when other is\n",
        "                Empty then None\n",
        "                Full(deep) then deep\n",
        "        return local\n",
        "later := 2\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let outer = case(value(&file, 0));
    let Expr::Block(body) = &outer.arms.value[1].value.body.value else {
        panic!("arm block")
    };
    assert_eq!(body.statements.len(), 3);
    assert!(matches!(
        body.statements[0].value,
        Statement::Destructure(_)
    ));
    assert!(matches!(body.statements[1].value, Statement::Binding(_)));
    assert!(matches!(body.statements[2].value, Statement::Return(_)));
    assert_eq!(file.items.len(), 2);
}

#[test]
fn arm_binder_queries_have_only_their_own_arm_as_ancestor() {
    let source =
        "inspect: when slot is\n    Full(first) then first\n    Full(second) then second\n";
    let file = parser::parse_strict(source).unwrap();
    let outer = case(value(&file, 0));
    for arm in &outer.arms.value {
        let fields = variant(&arm.value.pattern.value).fields.as_ref().unwrap();
        let binder = &fields.binders.value[0];
        let hit = file
            .node_at_offset(file.span_table.get(binder.span_id).start())
            .unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::PatternBinder(_)));
        let arms: Vec<_> = hit
            .ancestors
            .iter()
            .filter_map(|node| {
                if let SyntaxNodeKind::MatchArm(ancestor) = node.kind {
                    Some(ancestor.span_id)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(arms, [arm.span_id]);
        assert_eq!(hit.declarations.last().unwrap().name, binder.value);
    }
}

#[test]
fn bare_and_explicit_nullary_destructuring_are_distinct() {
    let file =
        parser::parse_strict("main:\n    Empty := slot\n    Empty() := slot\n").unwrap();
    let Expr::Block(body) = &value(&file, 0).value else {
        panic!("block")
    };
    for (index, present) in [(0, false), (1, true)] {
        let Statement::Destructure(statement) = &body.statements[index].value else {
            panic!("pattern")
        };
        assert_eq!(statement.pattern.value.fields.is_some(), present);
    }
}

#[test]
fn arm_blocks_preserve_bare_and_value_returns() {
    let source = concat!(
        "inspect: when slot is\n",
        "    Empty then\n",
        "        return\n",
        "    Full(value) then\n",
        "        return value\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let outer = case(value(&file, 0));
    for (index, values) in [(0, 0), (1, 1)] {
        let Expr::Block(body) = &outer.arms.value[index].value.body.value else {
            panic!("block")
        };
        let Statement::Return(returned) = &body.statements[0].value else {
            panic!("return")
        };
        assert_eq!(usize::from(returned.value.is_some()), values);
    }
}

#[test]
fn missing_arm_bodies_and_keywords_are_not_unwritten_syntax() {
    let source = "inspect: when slot is\n    Empty then\n    Full(value)\nlater := 2\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    let outer = case(value(&output.file, 0));
    assert_eq!(outer.arms.value.len(), 2);
    for arm in &outer.arms.value {
        assert!(matches!(arm.value.body.value, Expr::Missing));
        assert!(
            output
                .file
                .span_table
                .get(arm.value.body.span_id)
                .is_empty()
        );
    }
    assert_eq!(
        output
            .file
            .span_table
            .get(outer.arms.value[0].value.then_keyword)
            .extract(source),
        "then"
    );
    assert!(
        output
            .file
            .span_table
            .get(outer.arms.value[1].value.then_keyword)
            .is_empty()
    );
    assert_eq!(output.file.items.len(), 2);
}

#[test]
fn case_bodies_obey_expression_depth_and_cancellation_budgets() {
    use parser::{ParseFailure, ParseLimit, ParseLimits, ParseOptions};
    use std::cell::Cell;
    let source = "inspect: when slot is\n    Empty then\n        return - -value\n";
    let failure = parser::parse_with_options(
        source,
        ParseOptions {
            limits: ParseLimits {
                expression_depth_max: 4,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(matches!(
        failure,
        ParseFailure::LimitExceeded {
            limit: ParseLimit::ExpressionDepth,
            ..
        }
    ));
    let checks = Cell::new(0);
    let count = || {
        checks.set(checks.get() + 1);
        false
    };
    parser::parse_with_options(
        source,
        ParseOptions {
            is_cancelled: Some(&count),
            ..Default::default()
        },
    )
    .unwrap();
    let total = checks.get();
    for budget in [0, total / 2, total - 2] {
        checks.set(0);
        let cancel = || {
            checks.set(checks.get() + 1);
            checks.get() > budget
        };
        assert!(matches!(
            parser::parse_with_options(
                source,
                ParseOptions {
                    is_cancelled: Some(&cancel),
                    ..Default::default()
                }
            ),
            Err(ParseFailure::Cancelled { .. })
        ));
    }
}

#[test]
fn deeply_nested_case_subjects_are_bounded_before_building_owned_trees() {
    use parser::{ParseFailure, ParseLimit};
    let source = format!("inspect: {}slot\nlater := 2\n", "when ".repeat(2000));
    assert!(matches!(
        parser::parse(&source),
        Err(ParseFailure::LimitExceeded {
            limit: ParseLimit::ExpressionDepth,
            ..
        })
    ));
}

#[test]
fn stray_arm_body_delimiters_recover_without_restarting_at_the_same_token() {
    use parser::{ParseLimits, ParseOptions};
    let source = "inspect: when slot is\n    Empty then\n        x := )\nlater := 2\n";
    let output = parser::parse_with_options(
        source,
        ParseOptions {
            limits: ParseLimits {
                diagnostics_max: 1000,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .expect("syntax recovery, not budget refusal or step-bound panic");
    assert!(output.has_errors());
    assert!(
        output
            .file
            .declaration_outline()
            .any(|entry| entry.name == "later")
    );
    for suffix in [")", ",", ")", "then", "is", "when", "(", "Empty then )"] {
        let source =
            format!("inspect: when slot is\n    Empty then\n        {suffix}\nlater := 2\n");
        let output = parser::parse(&source).expect(&source);
        assert!(output.has_errors(), "{source}");
        assert!(
            output
                .file
                .declaration_outline()
                .any(|entry| entry.name == "later"),
            "{source}"
        );
    }
}

#[test]
fn nested_layout_subject_missing_outer_is_preserves_the_next_declaration() {
    let source = "inspect: when when slot is\n    Empty then None\nlater := 2\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    let outer = case(value(&output.file, 0));
    assert!(output.file.span_table.get(outer.is_keyword).is_empty());
    assert!(matches!(outer.subject.value, Expr::Match(_)));
    assert!(
        output
            .file
            .declaration_outline()
            .any(|entry| entry.name == "later")
    );
}

#[test]
fn unsupported_parenthesized_case_layout_is_explicit_and_preserves_later_items() {
    for expression in [
        "consume(when slot is\n    Empty then None)",
        "(when slot is\n    Empty then None)",
    ] {
        let source = format!("inspect: {expression}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors());
        assert!(output.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("case arm layout inside parenthesized calls/groups")
        }));
        assert!(
            output
                .file
                .declaration_outline()
                .any(|entry| entry.name == "later")
        );
    }
}

#[test]
fn short_malformed_arm_body_token_combinations_always_make_progress() {
    let tokens = [
        "when",
        "is",
        "then",
        "Empty",
        "x",
        ")",
        "(",
        "return",
        ":=",
        "\n        ",
    ];
    for first in tokens {
        for second in tokens {
            for third in tokens {
                let source = format!(
                    "inspect: when slot is\n    Empty then\n        {first} {second} {third}\nlater := 2\n"
                );
                let output = std::panic::catch_unwind(|| parser::parse(&source))
                    .unwrap_or_else(|_| panic!("parser panicked on {source:?}"))
                    .expect(&source);
                for offset in 0..=source.len() {
                    output.file.node_at_offset(offset).unwrap();
                }
            }
        }
    }
}

#[test]
fn arm_blocks_accept_ordinary_local_declarations_and_function_body_forms() {
    let source = concat!(
        "inspect: when slot is\n",
        "    Full(value) then\n",
        "        declared Int\n",
        "        helper: return value\n",
        "        nested:\n",
        "            return value\n",
        "        return helper\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let Expr::Block(body) = &case(value(&file, 0)).arms.value[0].value.body.value else {
        panic!("arm body")
    };
    assert_eq!(body.statements.len(), 4);
    for (index, initialized) in [(0, false), (1, true), (2, true)] {
        let Statement::Binding(local) = &body.statements[index].value else {
            panic!("local")
        };
        assert_eq!(local.value.is_some(), initialized);
        if let Some(value) = &local.value {
            assert!(matches!(value.expression.value, Expr::Block(_)));
        }
    }
}

#[test]
fn case_header_and_parameter_default_reentry_is_explicitly_unsupported() {
    for header in [
        "inspect is when slot is",
        "inspect(arg: when slot is): None",
    ] {
        let source = format!("{header}\n    Empty then None\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors());
        assert!(output.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("case expressions are unsupported in headers and parameter defaults")
        }));
        assert!(
            output
                .file
                .declaration_outline()
                .any(|entry| entry.name == "later")
        );
    }
}

#[test]
fn unsupported_patterns_and_guards_recover_before_later_declarations() {
    for arm in [
        "_ then value",
        "Full(value) if value then value",
        "else then value",
        "Full(value) value",
    ] {
        let source = format!("inspect: when slot is\n    {arm}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{arm}");
        assert!(
            output
                .file
                .declaration_outline()
                .any(|entry| entry.name == "later"),
            "{arm}"
        );
    }
}
