use ast::{
    Annotation, AnnotationKeyword, BinaryOperator, Binding, DomainBase, DomainKeyword, Expr, Item,
    RefinementPredicate, SourceFile, SpanId, Statement, SyntaxNodeKind, TypeBody, TypeMember,
};

fn binding(file: &SourceFile, index: usize) -> &Binding {
    let Item::Binding(binding) = &file.items[index].value else {
        panic!("expected ordinary binding");
    };
    binding
}

fn text(file: &SourceFile, span: SpanId) -> &str {
    file.span_table.get(span).extract(&file.text)
}

#[test]
fn implicit_domain_refinement_is_retained_after_a_base_type() {
    let file = parser::parse_strict("y Int and in 100 to 200: 155\n").unwrap();
    let Annotation::Refined {
        base: Some(base),
        refinements,
    } = &binding(&file, 0).annotation.as_ref().unwrap().value
    else {
        panic!("refined annotation with a base type");
    };
    assert_eq!(text(&file, base.span_id), "Int");
    assert_eq!(refinements.len(), 1);
    let RefinementPredicate::ImplicitDomain(domain) = &refinements[0].predicate.value else {
        panic!("implicit domain refinement");
    };
    assert!(domain.in_keyword.is_some());
    assert_eq!(text(&file, domain.span_id), "in 100 to 200");
    let DomainBase::Range { to_keyword, .. } = &domain.base.value else {
        panic!("range domain");
    };
    assert_eq!(text(&file, *to_keyword), "to");
}

#[test]
fn four_forms_retain_written_source_without_resolution_or_synthetic_subjects() {
    let source = concat!(
        "plain Box(Int): value\n",
        "bounded in 9 to 1 and < max: value\n",
        "refined Int and refined > 0 and < max and < max: value\n",
        "predicate is >= 0 and < max: value\n",
    );
    let file = parser::parse_strict(source).unwrap();
    assert_eq!(file.items.len(), 4);
    let Annotation::Expression(expression) = &binding(&file, 0).annotation.as_ref().unwrap().value
    else {
        panic!("plain annotation");
    };
    assert_eq!(text(&file, expression.span_id), "Box(Int)");
    let Annotation::Domain { is_keyword, domain } =
        &binding(&file, 1).annotation.as_ref().unwrap().value
    else {
        panic!("domain annotation");
    };
    assert_eq!(*is_keyword, None);
    let DomainBase::Range { lower, upper, .. } = &domain.value.base.value else {
        panic!("reversed range is source syntax, not a typecheck error");
    };
    assert_eq!(text(&file, lower.span_id), "9");
    assert_eq!(text(&file, upper.span_id), "1");
    let Annotation::Refined { base, refinements } =
        &binding(&file, 2).annotation.as_ref().unwrap().value
    else {
        panic!("refined expression");
    };
    assert_eq!(text(&file, base.as_ref().unwrap().span_id), "Int");
    assert_eq!(refinements.len(), 3);
    assert!(matches!(
        &refinements[0].value.predicate.value,
        RefinementPredicate::Expression(expression)
            if text(&file, expression.span_id) == "refined > 0"
    ));
    let spans: Vec<_> = refinements.iter().map(|clause| clause.span_id).collect();
    assert_eq!(text(&file, spans[1]), "and < max");
    assert_eq!(text(&file, spans[2]), "and < max");
    assert!(file.span_table.get(spans[1]).end() <= file.span_table.get(spans[2]).start());
    let Annotation::Predicate {
        predicate,
        refinements,
        ..
    } = &binding(&file, 3).annotation.as_ref().unwrap().value
    else {
        panic!("predicate annotation");
    };
    let RefinementPredicate::ImplicitComparison { operator, right } = &predicate.value else {
        panic!("implicit subject must remain implicit");
    };
    assert_eq!(operator.value, BinaryOperator::GreaterOrEqual);
    assert_eq!(text(&file, right.span_id), "0");
    assert_eq!(refinements.len(), 1);
}

#[test]
fn parameter_return_and_local_roles_stop_at_their_own_delimiters() {
    let source = concat!(
        "f(x in Byte, y Count and < max) in Count:\n",
        "    local is >= 0 and < max: x\n",
        "    return local\n",
        "defaults(x: 1, y Int): return y\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let function = binding(&file, 0);
    let parameters = &function.parameters.as_ref().unwrap().value;
    assert_eq!(parameters.len(), 2);
    assert_eq!(
        text(
            &file,
            parameters[0].value.annotation.as_ref().unwrap().span_id
        ),
        "in Byte"
    );
    assert_eq!(
        text(
            &file,
            parameters[1].value.annotation.as_ref().unwrap().span_id
        ),
        "Count and < max"
    );
    assert_eq!(
        text(&file, function.annotation.as_ref().unwrap().span_id),
        "in Count"
    );
    for annotation in [
        parameters[0].value.annotation.as_ref().unwrap(),
        function.annotation.as_ref().unwrap(),
    ] {
        assert!(matches!(
            annotation.value,
            Annotation::Domain {
                is_keyword: None,
                ..
            }
        ));
    }
    let Expr::Block(block) = &function.value.as_ref().unwrap().expression.value else {
        panic!("function body");
    };
    let Statement::Binding(local) = &block.statements[0].value else {
        panic!("local annotation");
    };
    assert!(matches!(
        local.annotation.as_ref().unwrap().value,
        Annotation::Predicate { .. }
    ));
    let defaults = &binding(&file, 1).parameters.as_ref().unwrap().value;
    assert!(defaults[0].value.annotation.is_none());
    assert!(defaults[0].value.default.is_some());
    assert!(defaults[1].value.default.is_none());
}

#[test]
fn generic_and_nonreceiver_method_parameters_use_parameter_annotations() {
    let source = concat!(
        "Generic(element in Type, size in 0 to max) is Ready\n",
        "Thing has\n",
        "    method(self, value Int and > 0) Int: return value\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let Item::Type(generic) = &file.items[0].value else {
        panic!("generic type")
    };
    let parameters = &generic.parameters.as_ref().unwrap().value;
    for parameter in parameters {
        assert!(matches!(
            parameter.value.annotation.as_ref().unwrap().value,
            Annotation::Domain {
                is_keyword: None,
                ..
            }
        ));
    }
    let Item::Type(thing) = &file.items[1].value else {
        panic!("type")
    };
    let TypeBody::Has(members) = &thing.body.value else {
        panic!("members")
    };
    let TypeMember::Binding(method) = &members[0].value else {
        panic!("method")
    };
    let parameters = &method.parameters.as_ref().unwrap().value;
    assert!(parameters[0].value.annotation.is_none());
    assert!(matches!(
        parameters[1].value.annotation.as_ref().unwrap().value,
        Annotation::Refined { .. }
    ));
}

#[test]
fn query_keywords_keep_exact_spelling_annotation_ancestors_and_owner() {
    let source = "--- hé 🦀\nf(x in 0 to max and < max) is >= 0 and < max: return x\n";
    let file = parser::parse_strict(source).unwrap();
    for (offset, _) in source.match_indices("is ") {
        let hit = file.node_at_offset(offset).unwrap();
        assert!(matches!(
            hit.node.kind,
            SyntaxNodeKind::AnnotationKeyword(AnnotationKeyword::Is)
        ));
        assert_eq!(hit.node.span.extract(source), "is");
        assert_eq!(hit.declarations.last().unwrap().name, "f");
        assert!(
            hit.ancestors
                .iter()
                .any(|node| matches!(node.kind, SyntaxNodeKind::Annotation(_)))
        );
    }
    for (word, expected) in [
        ("in", DomainKeyword::In),
        ("to", DomainKeyword::To),
        ("and", DomainKeyword::And),
    ] {
        for (offset, _) in source.match_indices(&format!(" {word} ")) {
            let hit = file.node_at_offset(offset + 1).unwrap();
            assert!(
                matches!(hit.node.kind, SyntaxNodeKind::DomainKeyword(found) if found == expected)
            );
            assert_eq!(hit.node.span.extract(source), word);
            assert_eq!(hit.declarations.last().unwrap().name, "f");
        }
    }
}

#[test]
fn missing_operands_with_real_closing_parentheses_preserve_later_declarations() {
    for header in [
        "f(x in): return x",
        "f(x in 0 to): return x",
        "f(x Int and <): return x",
        "f(x is >=): return x",
        "f(x is >= 0 and): return x",
    ] {
        let source = format!("{header}\nLater has\n    value Int\nlast := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        let function = binding(&output.file, 0);
        assert!(
            function.parameters.as_ref().unwrap().value[0]
                .value
                .annotation
                .is_some()
        );
        let hit = output
            .file
            .node_at_offset(source.find(')').unwrap())
            .unwrap();
        assert!(
            matches!(
                hit.node.kind,
                SyntaxNodeKind::Expression(expression) if expression.value == Expr::Missing
            ),
            "{source}: {hit:?}"
        );
        assert!(hit.node.span.is_empty());
        assert_eq!(hit.declarations.last().unwrap().name, "f");
        let names: Vec<_> = output
            .file
            .declaration_outline()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["f", "Later", "last"]);
    }
}

#[test]
fn missing_value_and_return_annotation_operands_do_not_consume_later_declarations() {
    for header in [
        "value in",
        "value in 0 to",
        "value Int and <",
        "value is >=",
        "f() in",
        "f() is >= 0 and",
    ] {
        let source = format!("{header}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert!(binding(&output.file, 0).annotation.is_some());
        let names: Vec<_> = output
            .file
            .declaration_outline()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, [header.split([' ', '(']).next().unwrap(), "later"]);
        assert!(parser::parse_strict(&source).is_err());
    }
}

#[test]
fn local_annotation_queries_keep_both_declaration_owners() {
    let source = "--- hé 🦀\nmain:\n    local is >= 0 and < max: 1\n    return local\n";
    let file = parser::parse_strict(source).unwrap();
    let hit = file.node_at_offset(source.find("is ").unwrap()).unwrap();
    assert!(matches!(
        hit.node.kind,
        SyntaxNodeKind::AnnotationKeyword(AnnotationKeyword::Is)
    ));
    let names: Vec<_> = hit.declarations.iter().map(|entry| entry.name).collect();
    assert_eq!(names, ["main", "local"]);
    assert_eq!(hit.node.span.extract(source), "is");
}

#[test]
fn unsupported_header_shapes_do_not_swallow_later_items() {
    for header in [
        "f(x Int: 1): return x",
        "Thing has value own{arena} Int",
        "Thing has method(self ref mut): return self",
        "Thing has method(self) Int: return own Int",
        "f(ref x Int): return x",
        "value in Byte\n    and < max: 1",
        "f(x is >= 0\n    and < max): return x",
    ] {
        let source = format!("{header}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        let hit = output
            .file
            .node_at_offset(source.find("later").unwrap())
            .unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("later")));
    }
}

#[test]
fn newlines_separate_parameters_but_nested_expression_groups_can_span_lines() {
    let source = concat!(
        "f(x in Byte\n    y Count and < max) Int: return x\n",
        "value Int and (value >\n    0): 1\n",
    );
    let file = parser::parse_strict(source).unwrap();
    assert_eq!(
        binding(&file, 0).parameters.as_ref().unwrap().value.len(),
        2
    );
    assert!(matches!(
        binding(&file, 1).annotation.as_ref().unwrap().value,
        Annotation::Refined { .. }
    ));
}

#[test]
fn trailing_layout_keeps_header_holes_at_eof_without_consuming_the_body_dedent() {
    for header in [
        "value is",
        "value in",
        "value in 0 to",
        "value Int and <",
        "f() in",
    ] {
        for enclosing_body in [false, true] {
            let source = if enclosing_body {
                format!("main:\n    {header}\n        -- hé 🦀\n        ")
            } else {
                format!("{header}\n    -- hé 🦀\n    ")
            };
            let output = parser::parse(&source).unwrap();
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
            let expected = if header.starts_with("f(") {
                "f"
            } else {
                "value"
            };
            assert_eq!(hit.declarations.last().unwrap().name, expected);
            if enclosing_body {
                assert_eq!(hit.declarations[0].name, "main");
            }
        }
    }
}

#[test]
fn root_disjunctions_cannot_be_silently_folded_into_a_header_component() {
    for header in [
        "value is self < max or flag: 1",
        "value in Byte or Other: 1",
        "f(x Int and x > 0 or flag): return x",
    ] {
        let source = format!("{header}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors());
        let hit = output
            .file
            .node_at_offset(source.find("later").unwrap())
            .unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("later")));
    }
    let source = "value is (self < max or flag): 1\n";
    assert!(parser::parse_strict(source).is_ok());
}

#[test]
fn header_annotations_obey_expression_and_diagnostic_budgets() {
    use parser::{ParseFailure, ParseLimit, ParseLimits, ParseOptions};

    for (source, limits, expected) in [
        (
            "f(x in - -limit to max): return x\n",
            ParseLimits {
                expression_depth_max: 2,
                ..Default::default()
            },
            ParseLimit::ExpressionDepth,
        ),
        (
            "value in 0 to and : 1\n",
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
            panic!("expected a budget refusal");
        };
        assert_eq!(limit, expected);
        assert!(source.get(span.to_range()).is_some());
    }
}

#[test]
fn eof_annotation_scans_remain_cancellable_after_lexing_and_layout() {
    use parser::{ParseFailure, ParseOptions};
    use std::cell::Cell;

    let source = format!("value in\n{}", "\n".repeat(4_000));
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
        parser::parse_with_options(&source, options)
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
fn many_parameter_refinements_remain_flat_and_source_ordered() {
    let source = format!(
        "f(value Int{}): return value\n",
        " and < limit".repeat(2_000)
    );
    let file = parser::parse_strict(&source).unwrap();
    let parameter = &binding(&file, 0).parameters.as_ref().unwrap().value[0];
    let Annotation::Refined { refinements, .. } =
        &parameter.value.annotation.as_ref().unwrap().value
    else {
        panic!("written parameter clauses");
    };
    assert_eq!(refinements.len(), 2_000);
    assert_eq!(file, file.clone());
    let hit = file.node_at_offset(source.rfind("limit").unwrap()).unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("limit")));
    assert_eq!(hit.declarations.last().unwrap().name, "f");
}
