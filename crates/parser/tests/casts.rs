use ast::{
    Annotation, BinaryOperator, Expr, Expression, Item, SourceFile, SyntaxNodeKind, TypePrefixKind,
};
use parser::{ParseFailure, ParseLimit, ParseLimits, ParseOptions};

fn value(file: &SourceFile) -> &Expression {
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("binding");
    };
    &binding.value.as_ref().unwrap().expression
}

fn cast(expression: &Expression) -> (&Expression, &Expression) {
    let Expr::Cast { value, target, .. } = &expression.value else {
        panic!("cast: {expression:?}");
    };
    (value, target)
}

#[test]
fn pointer_annotations_and_raw_casts_are_source_syntax_not_authority() {
    let source = concat!(
        "convert(address @Outer) result Outer: return address as Outer\n",
        "Pointer is @@Outer\n",
        "Holder has field @Outer, method() @Outer\n",
        "ordinary: Outer(region: region)\n",
    );
    let file = parser::parse_strict(source).unwrap();
    assert_eq!(file.text.as_ref(), source);
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("function");
    };
    let parameter = &binding.parameters.as_ref().unwrap().value[0].value;
    let Annotation::Expression(expression) = &parameter.annotation.as_ref().unwrap().value else {
        panic!("pointer annotation");
    };
    let Expr::PrefixedType(pointer) = &expression.value else {
        panic!("pointer prefix");
    };
    assert_eq!(pointer.keyword.value, TypePrefixKind::Pointer);
    assert!(pointer.group.is_none());
    assert_eq!(pointer.target.value, Expr::Name("Outer".into()));
    assert_eq!(binding.result_binder.as_ref().unwrap().value, "result");
    let keyword = source.find("as Outer").unwrap();
    let hit = file.node_at_offset(keyword).unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::AsKeyword));
    assert_eq!(hit.node.span.extract(source), "as");
    assert_eq!(hit.declarations[0].name, "convert");
    let hit = file.node_at_offset(source.find('@').unwrap()).unwrap();
    assert!(matches!(hit.node.kind,
        SyntaxNodeKind::TypePrefixKeyword(prefix) if prefix.value == TypePrefixKind::Pointer));
    assert_eq!(hit.node.span.extract(source), "@");
}

#[test]
fn casts_bind_above_binary_operators_and_chain_left_to_right() {
    let file = parser::parse_strict("value: a + f(x).field as @Outer * b\n").unwrap();
    let Expr::Binary {
        operator, right, ..
    } = &value(&file).value
    else {
        panic!("sum");
    };
    assert_eq!(operator.value, BinaryOperator::Add);
    let Expr::Binary { left, operator, .. } = &right.value else {
        panic!("product");
    };
    assert_eq!(operator.value, BinaryOperator::Multiply);
    let (operand, target) = cast(left);
    assert!(matches!(operand.value, Expr::Member { .. }));
    let Expr::PrefixedType(pointer) = &target.value else {
        panic!("pointer target");
    };
    assert_eq!(pointer.keyword.value, TypePrefixKind::Pointer);
    assert_eq!(pointer.target.value, Expr::Name("Outer".into()));
    let file = parser::parse_strict("value: a as First as Second\n").unwrap();
    let (operand, target) = cast(value(&file));
    assert_eq!(target.value, Expr::Name("Second".into()));
    assert_eq!(cast(operand).1.value, Expr::Name("First".into()));
}

#[test]
fn target_calls_groups_and_prefixes_keep_written_structure() {
    let file = parser::parse_strict("value: a as @Box(ref Item).Member\n").unwrap();
    let (_, target) = cast(value(&file));
    let Expr::PrefixedType(pointer) = &target.value else {
        panic!("pointer");
    };
    assert!(matches!(pointer.target.value, Expr::Member { .. }));
    let file = parser::parse_strict("value: a as (@Outer)\n").unwrap();
    assert!(matches!(cast(value(&file)).1.value, Expr::Group(_)));
    let file = parser::parse_strict("value: (a as Outer)(x).field\n").unwrap();
    let Expr::Member { base, .. } = &value(&file).value else {
        panic!("member of call");
    };
    let Expr::Call { callee, .. } = &base.value else {
        panic!("call on grouped cast");
    };
    let Expr::Group(inner) = &callee.value else {
        panic!("explicit group");
    };
    assert!(matches!(inner.value, Expr::Cast { .. }));
    let file = parser::parse_strict("value: -a as Outer\n").unwrap();
    assert!(matches!(cast(value(&file)).0.value, Expr::Unary { .. }));
}

#[test]
fn pointer_type_arguments_do_not_require_the_parser_to_resolve_argument_roles() {
    for source in [
        "value: identity(@Public, address)\n",
        "value: Box(value: identity(@Region, address))\n",
        "value: identity((@Public), address)\n",
        "value: identity(@@Public, address)\n",
    ] {
        let file = parser::parse_strict(source).unwrap();
        let hit = file.node_at_offset(source.find('@').unwrap()).unwrap();
        assert!(matches!(hit.node.kind,
            SyntaxNodeKind::TypePrefixKeyword(prefix)
                if prefix.value == TypePrefixKind::Pointer));
        assert_eq!(hit.node.span.extract(source), "@");
        assert!(
            hit.ancestors
                .iter()
                .any(|node| matches!(node.kind, SyntaxNodeKind::Argument(_)))
        );
        assert_eq!(hit.declarations[0].name, "value");
    }
}

#[test]
fn pointer_syntax_does_not_enable_other_value_operations_or_leak_type_context() {
    for source in [
        "value: new Outer",
        "value: unsafe address",
        "T is @Outer to limit",
        "value: a as @{arena} Outer",
    ] {
        let source = format!("{source}\nlater: 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert!(parser::parse_strict(&source).is_err(), "{source}");
        let hit = output
            .file
            .node_at_offset(source.find("later").unwrap())
            .unwrap();
        assert!(
            matches!(hit.node.kind, SyntaxNodeKind::Name("later")),
            "{source}"
        );
    }
}

#[test]
fn missing_cast_and_pointer_targets_retain_missing_nodes_and_later_declarations() {
    for source in [
        "value: address as",
        "value: address as @",
        "value: f(address as, next)",
        "value: (address as)",
        "Pointer is @",
    ] {
        let source = format!("{source}\nlater: 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        let hit = output
            .file
            .node_at_offset(source.find("later").unwrap())
            .unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("later")));
        let insertion = source.find('\n').unwrap();
        let offset = if source.as_bytes()[insertion - 1] == b')' {
            source.find([',', ')']).unwrap()
        } else {
            insertion
        };
        let hit = output.file.node_at_offset(offset).unwrap();
        assert!(
            matches!(hit.node.kind,
            SyntaxNodeKind::Expression(expression) if expression.value == Expr::Missing),
            "{source}: {hit:?}"
        );
        assert!(hit.node.span.is_empty());
    }
}

#[test]
fn unfinished_calls_do_not_swallow_pointer_annotated_declarations() {
    let source = "Holder has\n    broken: f(\n    field @Outer\n)\n    next Int\nlater: 2\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    let hit = output
        .file
        .node_at_offset(source.find("field").unwrap())
        .unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("field")));
    let owners: Vec<_> = hit.declarations.iter().map(|entry| entry.name).collect();
    assert_eq!(owners, ["Holder", "field"]);
}

#[test]
fn malformed_cast_and_pointer_combinations_make_bounded_progress() {
    let pieces = ["as", "@", "Outer", "value", "(", ")", ",", ".", "+", "ref"];
    for first in pieces {
        for second in pieces {
            for third in pieces {
                let source = format!("value: {first} {second} {third}\nlater: 2\n");
                let output = parser::parse(&source).unwrap();
                for diagnostic in &output.diagnostics {
                    assert!(source.get(diagnostic.span.to_range()).is_some());
                }
                for offset in 0..=source.len() {
                    let hit = output.file.node_at_offset(offset).unwrap();
                    assert!(source.get(hit.node.span.to_range()).is_some());
                }
                let hit = output
                    .file
                    .node_at_offset(source.find("later").unwrap())
                    .unwrap();
                assert!(
                    matches!(hit.node.kind, SyntaxNodeKind::Name("later")),
                    "{source}"
                );
            }
        }
    }
}

#[test]
fn casts_and_pointer_prefixes_obey_depth_and_diagnostic_budgets() {
    for (source, limits, expected) in [
        (
            "Pointer is @@Outer",
            ParseLimits {
                expression_depth_max: 2,
                ..Default::default()
            },
            ParseLimit::ExpressionDepth,
        ),
        (
            "value: a as First as Second",
            ParseLimits {
                expression_depth_max: 2,
                ..Default::default()
            },
            ParseLimit::ExpressionDepth,
        ),
        (
            "value: a as @{group}",
            ParseLimits {
                diagnostics_max: 1,
                ..Default::default()
            },
            ParseLimit::Diagnostics,
        ),
    ] {
        let failure = parser::parse_with_options(
            source,
            ParseOptions {
                limits,
                ..Default::default()
            },
        )
        .unwrap_err();
        let ParseFailure::LimitExceeded { limit, span, .. } = failure else {
            panic!("bounded refusal");
        };
        assert_eq!(limit, expected);
        assert!(source.get(span.to_range()).is_some());
    }
}

#[test]
fn cast_target_parsing_checks_cancellation() {
    use std::cell::Cell;

    let source = format!("value: a{}\n", " as @Outer".repeat(100));
    let count = Cell::new(0usize);
    let baseline = || {
        count.set(count.get() + 1);
        false
    };
    parser::parse_with_options(
        &source,
        ParseOptions {
            is_cancelled: Some(&baseline),
            ..Default::default()
        },
    )
    .unwrap();
    let cutoff = count.get() - 10;
    let visits = Cell::new(0usize);
    let cancel = || {
        visits.set(visits.get() + 1);
        visits.get() == cutoff
    };
    let failure = parser::parse_with_options(
        &source,
        ParseOptions {
            is_cancelled: Some(&cancel),
            ..Default::default()
        },
    )
    .unwrap_err();
    let ParseFailure::Cancelled { span } = failure else {
        panic!("cancellation");
    };
    assert!(span.start() > source.len() / 2);
    assert!(source.get(span.to_range()).is_some());
}

/// `#unchecked(x)` is an ordinary query call: the callee is `Query("unchecked")` and `x` the one
/// positional argument. Whether it is a valid marker is the checker's to say.
fn unchecked(expression: &Expression) -> &Expression {
    let Expr::Call { callee, arguments } = &expression.value else {
        panic!("call: {expression:?}");
    };
    let Expr::Query { name, .. } = &callee.value else {
        panic!("query: {callee:?}");
    };
    assert_eq!(name.value, "unchecked");
    assert_eq!(arguments.value.len(), 1);
    let ast::Argument::Positional(operand) = &arguments.value[0].value else {
        panic!("positional operand");
    };
    operand
}

#[test]
fn unchecked_marks_the_whole_cast_or_address_inside_it() {
    let file = parser::parse_strict("raw: #unchecked(address as @Byte)\n").unwrap();
    cast(unchecked(value(&file)));
    let hit = file.node_at_offset(file.text.find("unchecked").unwrap()).unwrap();
    assert_eq!(hit.node.span.extract(&file.text), "unchecked");

    // An address, and a marker inside a call argument, each stay with their own operation.
    let file = parser::parse_strict("p: #unchecked(@count)\n").unwrap();
    assert!(matches!(&unchecked(value(&file)).value, Expr::PrefixedType(_)));
    let file = parser::parse_strict("p: f(#unchecked(a as @Byte), b)\n").unwrap();
    let Expr::Call { arguments, .. } = &value(&file).value else {
        panic!("call");
    };
    assert_eq!(arguments.value.len(), 2);

    // A binary operator before it keeps the marker on the right operand only.
    let file = parser::parse_strict("p: a + #unchecked(b as @Byte)\n").unwrap();
    let Expr::Binary { right, .. } = &value(&file).value else {
        panic!("binary: {:?}", value(&file));
    };
    cast(unchecked(right));

    // A marker may follow a cast or sit inside a larger expression.
    let file = parser::parse_strict("p: #unchecked(a) as @Byte\n").unwrap();
    let (marked, _) = cast(value(&file));
    unchecked(marked);
}

#[test]
fn the_postfix_marker_is_a_parse_error_everywhere() {
    for source in [
        "p: address as @Byte #allow_unchecked\n",
        "p: @count #allow_unchecked\n",
        "p: a #unchecked\n",
        "p: a #other\n",
        "p: f(a #allow_unchecked, b)\n",
    ] {
        assert!(parser::parse_strict(source).is_err(), "{source}");
    }
}

#[test]
fn unchecked_alone_or_with_other_argument_shapes_still_parses_as_a_call_for_the_checker_to_judge() {
    // No operand, two operands, a named operand: the parser does not decide the arity.
    let file = parser::parse_strict("p: #unchecked()\n").unwrap();
    let Expr::Call { arguments, .. } = &value(&file).value else { panic!("call") };
    assert!(arguments.value.is_empty());
    let file = parser::parse_strict("p: #unchecked(a, b)\n").unwrap();
    let Expr::Call { arguments, .. } = &value(&file).value else { panic!("call") };
    assert_eq!(arguments.value.len(), 2);
    let file = parser::parse_strict("p: #unchecked(value: a)\n").unwrap();
    let Expr::Call { arguments, .. } = &value(&file).value else { panic!("call") };
    assert!(matches!(arguments.value[0].value, ast::Argument::Named { .. }));
    // A bare `#unchecked` is the query, not a call.
    let file = parser::parse_strict("p: #unchecked\n").unwrap();
    assert!(matches!(&value(&file).value, Expr::Query { .. }));
}

#[test]
fn a_marker_inside_a_marker_parses_as_nested_calls() {
    let file = parser::parse_strict("p: #unchecked(#unchecked(a as @Byte))\n").unwrap();
    let inner = unchecked(value(&file));
    cast(unchecked(inner));
}
