use ast::{
    BinaryOperator, DomainBase, DomainKeyword, Expr, Item, RefinementPredicate, SourceFile,
    SyntaxNodeKind, TypeBody, TypeDomain, UnaryOperator,
};

fn domain(file: &SourceFile, index: usize) -> &TypeDomain {
    let Item::Type(declaration) = &file.items[index].value else {
        panic!("expected source type");
    };
    let TypeBody::Domain(domain) = &declaration.body.value else {
        panic!("expected explicit domain syntax");
    };
    &domain.value
}

#[test]
fn in_and_bare_ranges_preserve_written_bounds_without_validation() {
    let source = "Byte is in 0 to 255\nReverse is 9 to 1\nSigned is in -128 to 127\n";
    let file = parser::parse_strict(source).unwrap();
    assert!(domain(&file, 0).in_keyword.is_some());
    assert!(domain(&file, 1).in_keyword.is_none());
    let DomainBase::Range {
        lower,
        to_keyword,
        upper,
    } = &domain(&file, 1).base.value
    else {
        panic!("bare range stays a range");
    };
    assert_eq!(file.span_table.get(lower.span_id).extract(source), "9");
    assert_eq!(file.span_table.get(*to_keyword).extract(source), "to");
    assert_eq!(file.span_table.get(upper.span_id).extract(source), "1");
    let DomainBase::Range { lower, .. } = &domain(&file, 2).base.value else {
        panic!("signed range");
    };
    let Expr::Unary { operator, operand } = &lower.value else {
        panic!("a minus is not part of a decoded literal");
    };
    assert_eq!(operator.value, UnaryOperator::Negate);
    assert_eq!(file.span_table.get(operator.span_id).extract(source), "-");
    assert_eq!(file.span_table.get(operand.span_id).extract(source), "128");
}

#[test]
fn symbolic_domains_keep_expressions_and_ordered_refinement_clauses() {
    let source = "\
Count(limit) is in 0 to limit and self < limit and (self > 0 and self < 8)
Size is in core.Size and self % 8 = 0
Plain is in Size
";
    let file = parser::parse_strict(source).unwrap();
    let count = domain(&file, 0);
    assert_eq!(count.refinements.len(), 2);
    let DomainBase::Range { upper, .. } = &count.base.value else {
        panic!("symbolic upper bound");
    };
    assert_eq!(upper.value, Expr::Name("limit".into()));
    let RefinementPredicate::Expression(group) = &count.refinements[1].value.predicate.value else {
        panic!("written predicate expression");
    };
    assert!(matches!(group.value, Expr::Group(_)));
    assert!(matches!(
        domain(&file, 1).base.value,
        DomainBase::Expression(_)
    ));
    assert_eq!(domain(&file, 1).refinements.len(), 1);
    assert!(domain(&file, 2).refinements.is_empty());
}

#[test]
fn abbreviated_comparisons_do_not_invent_a_self_expression() {
    let file = parser::parse_strict("Small is in 0 to max and >= 1 and < max\n").unwrap();
    let refinements = &domain(&file, 0).refinements;
    assert_eq!(refinements.len(), 2);
    for (clause, expected) in refinements
        .iter()
        .zip([BinaryOperator::GreaterOrEqual, BinaryOperator::Less])
    {
        let RefinementPredicate::ImplicitComparison { operator, right } =
            &clause.value.predicate.value
        else {
            panic!("source omits the subject");
        };
        assert_eq!(operator.value, expected);
        assert!(matches!(right.value, Expr::Literal(_) | Expr::Name(_)));
    }
}

#[test]
fn domain_keywords_and_bounds_are_navigable_without_resolving_them() {
    let source = "Aligned is in -limit to max and self % 8 = 0\nlater := 2\n";
    let file = parser::parse_strict(source).unwrap();
    for (word, kind) in [
        ("in", DomainKeyword::In),
        ("to", DomainKeyword::To),
        ("and", DomainKeyword::And),
    ] {
        let hit = file
            .node_at_offset(source.find(&format!(" {word} ")).unwrap() + 1)
            .unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::DomainKeyword(found) if found == kind));
        assert_eq!(hit.node.span.extract(source), word);
        assert_eq!(hit.declarations[0].name, "Aligned");
    }
    for word in ["limit", "max", "self"] {
        let hit = file.node_at_offset(source.find(word).unwrap()).unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::Name(found) if found == word));
        assert_eq!(hit.declarations[0].name, "Aligned");
    }
    let names: Vec<_> = file.declaration_outline().map(|entry| entry.name).collect();
    assert_eq!(names, ["Aligned", "later"]);
}

#[test]
fn domain_layout_preserves_later_types_and_bindings() {
    let source = "\
Aligned is
    in 0 to
        limit
    and self % 8 = 0
    and < limit
Aligned has
    value Int
later := 2
";
    let file = parser::parse_strict(source).unwrap();
    assert_eq!(domain(&file, 0).refinements.len(), 2);
    let names: Vec<_> = file.declaration_outline().map(|entry| entry.name).collect();
    assert_eq!(names, ["Aligned", "Aligned", "later"]);
}

#[test]
fn missing_domain_parts_retain_source_nodes_and_following_declarations() {
    for prefix in [
        "T is in",
        "T is in 0 to",
        "T is in Byte and",
        "T is in 0 to 9 and <",
    ] {
        for suffix in ["", "\nNext has\n    value Int\nlast := 2\n"] {
            let source = format!("{prefix}{suffix}");
            let output = parser::parse(&source).unwrap();
            assert!(output.has_errors(), "{source}");
            let _ = domain(&output.file, 0);
            let names: Vec<_> = output
                .file
                .declaration_outline()
                .map(|entry| entry.name)
                .collect();
            if suffix.is_empty() {
                assert_eq!(names, ["T"]);
            } else {
                assert_eq!(names, ["T", "Next", "last"], "{source}");
            }
            assert!(parser::parse_strict(&source).is_err());
        }
    }
}

#[test]
fn missing_domain_parts_keep_their_type_owner_at_the_true_eof_cursor() {
    for prefix in [
        "T is in",
        "T is in 0 to",
        "T is in Byte and",
        "T is in 0 to 9 and <",
        "T is in -",
    ] {
        let source = format!("{prefix}\n    -- hé 🦀\n    ");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors());
        let hit = output.file.node_at_offset(source.len()).unwrap();
        assert!(matches!(
            hit.node.kind,
            SyntaxNodeKind::Expression(expression) if expression.value == Expr::Missing
        ));
        assert_eq!(hit.node.span.start(), source.len());
        assert_eq!(hit.node.span.end(), source.len());
        assert_eq!(hit.declarations[0].name, "T");
    }
}

#[test]
fn domain_numbers_are_not_limited_by_backend_integer_or_float_conversion() {
    let huge = "9".repeat(200);
    let source = format!("Huge is in -{huge} to {huge}\nNotInteger is in 1.5 to 2.5\n");
    let file = parser::parse_strict(&source).unwrap();
    assert_eq!(file.items.len(), 2);
    let DomainBase::Range { upper, .. } = &domain(&file, 0).base.value else {
        panic!("huge written number");
    };
    assert_eq!(file.span_table.get(upper.span_id).extract(&source), huge);
}

#[test]
fn aliases_and_unions_do_not_become_domains_or_refinement_metadata() {
    let source =
        "Alias is Byte\nComposition is Input and Output\nChoice is\n    First\n    or Second\n";
    let file = parser::parse_strict(source).unwrap();
    for item in &file.items {
        let Item::Type(declaration) = &item.value else {
            panic!("written type");
        };
        assert!(matches!(declaration.body.value, TypeBody::Is(_)));
    }
    for source in [
        "Thing has value is in Byte: 1\n",
        "f(self is in Byte): return self\n",
        "Thing has value own{arena} Int\n",
        "T is in 0 to 9 and has Trait\n",
        "T is in 0 to 9 to 10\n",
    ] {
        assert!(
            parser::parse(source).unwrap().has_errors(),
            "{source}"
        );
    }
}

#[test]
fn domains_obey_expression_layout_and_diagnostic_budgets() {
    use parser::{ParseFailure, ParseLimit, ParseLimits, ParseOptions};

    for (source, limits, expected) in [
        (
            "T is in - -limit to max\n",
            ParseLimits {
                expression_depth_max: 2,
                ..Default::default()
            },
            ParseLimit::ExpressionDepth,
        ),
        (
            "T is\n    in 0 to\n        max\n",
            ParseLimits {
                body_depth_max: 1,
                ..Default::default()
            },
            ParseLimit::BodyDepth,
        ),
        (
            "T is in 0 to and\n",
            ParseLimits {
                diagnostics_max: 1,
                ..Default::default()
            },
            ParseLimit::Diagnostics,
        ),
    ] {
        let options = ParseOptions {
            limits,
            ..Default::default()
        };
        let ParseFailure::LimitExceeded { limit, span, .. } =
            parser::parse_with_options(source, options).unwrap_err()
        else {
            panic!("expected budget refusal");
        };
        assert_eq!(limit, expected);
        assert!(source.get(span.to_range()).is_some());
    }
}

#[test]
fn cancellation_reaches_domain_continuation_work() {
    use parser::{ParseFailure, ParseOptions};
    use std::cell::Cell;

    let source = format!(
        "T is in 0 to limit\n{}    and < limit\n",
        "\n".repeat(2_000)
    );
    let baseline = Cell::new(0usize);
    let count = || {
        baseline.set(baseline.get() + 1);
        false
    };
    let options = ParseOptions {
        is_cancelled: Some(&count),
        ..Default::default()
    };
    assert!(
        !parser::parse_with_options(&source, options)
            .unwrap()
            .has_errors()
    );
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
        parser::parse_with_options(&source, options),
        Err(ParseFailure::Cancelled { .. })
    ));
    assert_eq!(visits.get(), cutoff);
}

#[test]
fn many_refinement_clauses_remain_a_flat_source_ordered_list() {
    let source = format!("T is in 0 to limit{}\n", " and < limit".repeat(2_000));
    let file = parser::parse_strict(&source).unwrap();
    assert_eq!(domain(&file, 0).refinements.len(), 2_000);
    assert_eq!(file, file.clone());
    let hit = file.node_at_offset(source.rfind("limit").unwrap()).unwrap();
    assert_eq!(hit.declarations[0].name, "T");
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("limit")));
}

#[test]
fn only_function_results_take_alternatives() {
    for source in [
        "value Int and value > 0: 1",
        "f(value Int and value > 0): return value",
        "T(value Int and value > 0) is Ready",
        "Thing has value Int and value > 0",
    ] {
        assert!(parser::parse_strict(source).is_ok(), "{source}");
    }
    for source in [
        "f() Good or Bad: return Good",
        "Thing has choose(self) Good or Bad: return Good",
    ] {
        assert!(parser::parse_strict(source).is_ok(), "{source}");
    }
    for header in ["value Int or Bad: 1", "Thing has value Int or Bad"] {
        let source = format!("{header}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| { matches!(diagnostic.span.extract(&source), "and" | "or") })
        );
        let hit = output
            .file
            .node_at_offset(source.find("later").unwrap())
            .unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("later")));
        assert!(parser::parse_strict(&source).is_err());
    }
    let source = "value Box(Input and Output): 1\nf(value Box(Input or Output)): return value\n";
    assert!(!parser::parse(source).unwrap().has_errors());
}

#[test]
fn abbreviated_comparisons_do_not_bypass_precedence_or_chain_diagnostics() {
    for predicate in ["< x < y", "= x = y", "< x or y"] {
        let source = format!("T is in 0 to 9 and {predicate}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert!(parser::parse_strict(&source).is_err());
        let hit = output
            .file
            .node_at_offset(source.find("later").unwrap())
            .unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("later")));
    }
    for predicate in [
        "< (x < y)",
        "= (x = y)",
        "< (x or y)",
        "< x + 1",
        "self < x or y",
    ] {
        let source = format!("T is in 0 to 9 and {predicate}\n");
        assert!(parser::parse_strict(&source).is_ok(), "{source}");
    }
}
