use ast::*;

fn at<T>(spans: &mut SpanTable, value: T, range: std::ops::Range<usize>) -> Spanned<T> {
    Spanned::with_range(spans, value, range)
}

fn refinement(spans: &mut SpanTable, start: usize) -> Spanned<Refinement> {
    let and_keyword = spans.insert_range(start..start + 3);
    let right = at(spans, Expr::Missing, start + 5..start + 5);
    let operator = at(spans, BinaryOperator::Less, start + 4..start + 5);
    let predicate = at(
        spans,
        RefinementPredicate::ImplicitComparison { operator, right },
        start + 4..start + 5,
    );
    at(
        spans,
        Refinement {
            and_keyword,
            predicate,
        },
        start..start + 5,
    )
}

fn shape(spans: &mut SpanTable, kind: u8) -> (String, Spanned<Annotation>) {
    let (text, annotation) = match kind {
        0 => {
            let expression = at(spans, Expr::Name("Int".into()), 2..5);
            ("x Int", Annotation::Expression(expression))
        }
        1 | 4 => {
            let present = kind == 1;
            let start = if present { 5 } else { 2 };
            let expression = at(spans, Expr::Name("Int".into()), start + 3..start + 6);
            let base = at(
                spans,
                DomainBase::Expression(expression),
                start + 3..start + 6,
            );
            let in_keyword = Some(spans.insert_range(start..start + 2));
            let domain = at(
                spans,
                TypeDomain {
                    in_keyword,
                    base,
                    refinements: vec![],
                },
                start..start + 6,
            );
            let is_keyword = present.then(|| spans.insert_range(2..4));
            let text = if present { "x is in Int" } else { "x in Int" };
            (text, Annotation::Domain { is_keyword, domain })
        }
        2 => {
            let base = at(spans, Expr::Name("Int".into()), 2..5);
            let refinements = vec![refinement(spans, 6), refinement(spans, 12)];
            (
                "x Int and < and <",
                Annotation::Refined {
                    base: Some(base),
                    refinements,
                },
            )
        }
        3 => {
            let is_keyword = spans.insert_range(2..4);
            let expression = at(spans, Expr::Missing, 5..5);
            let predicate = at(spans, RefinementPredicate::Expression(expression), 5..5);
            let refinements = vec![refinement(spans, 6), refinement(spans, 12)];
            (
                "x is  and < and <",
                Annotation::Predicate {
                    is_keyword,
                    predicate,
                    refinements,
                },
            )
        }
        _ => unreachable!(),
    };
    let end = text.len();
    (text.into(), at(spans, annotation, 2..end))
}

fn file(kind: u8, parameter: bool) -> SourceFile {
    let mut spans = SpanTable::new();
    let (text, annotation) = shape(&mut spans, kind);
    let name: Name = at(&mut spans, "x".into(), 0..1);
    let mut binding = Binding {
        attributes: Vec::new(),
        name: name.clone(),
        parameters: None,
        convention: None,
        result_binder: None,
        annotation: None,
        contract: None,
        alternatives: Vec::new(),
        extra_results: Vec::new(),
        value: None,
    };
    if parameter {
        let value = Parameter {
            name,
            convention: None,
            annotation: Some(annotation),
            default: None,
        };
        let value = at(&mut spans, value, 0..text.len());
        binding.parameters = Some(at(&mut spans, vec![value], 0..text.len()));
    } else {
        binding.annotation = Some(annotation);
    }
    let item = at(&mut spans, Item::Binding(Box::new(binding)), 0..text.len());
    SourceFile {
        text: text.into(),
        span_table: spans,
        items: vec![item],
        comments: vec![],
    }
}

#[test]
fn four_shapes_keep_annotation_views_and_declaration_owners() {
    for kind in 0..4 {
        for parameter in [false, true] {
            let file = file(kind, parameter);
            let offset = if kind == 3 { 5 } else { file.text.len() - 1 };
            let hit = file.node_at_offset(offset).unwrap();
            assert_eq!(hit.declarations.len(), 1);
            assert_eq!(hit.declarations[0].name, "x");
            assert_eq!(
                hit.ancestors
                    .iter()
                    .any(|node| matches!(node.kind, SyntaxNodeKind::Parameter(_))),
                parameter
            );
            let annotation = hit
                .ancestors
                .iter()
                .find_map(|node| match node.kind {
                    SyntaxNodeKind::Annotation(annotation) => Some(annotation),
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                format!("{:?}", SyntaxNodeKind::Annotation(annotation)),
                "Annotation"
            );
            if kind == 3 {
                assert!(matches!(hit.node.kind,
                    SyntaxNodeKind::Expression(expr) if expr.value == Expr::Missing));
                assert!(hit.node.span.is_empty());
            }
        }
    }
}

#[test]
fn written_keywords_have_exact_roles_and_no_synthetic_is() {
    for (kind, parameter) in [(1, false), (1, true), (4, true), (3, false)] {
        let file = file(kind, parameter);
        for (offset, _) in file.text.match_indices("is") {
            let hit = file.node_at_offset(offset).unwrap();
            assert!(matches!(
                hit.node.kind,
                SyntaxNodeKind::AnnotationKeyword(AnnotationKeyword::Is)
            ));
            assert_eq!(hit.node.span.to_range(), offset..offset + 2);
            assert_eq!(format!("{:?}", hit.node.kind), "AnnotationKeyword");
        }
        for (offset, _) in file.text.match_indices("in") {
            let hit = file.node_at_offset(offset).unwrap();
            assert!(matches!(
                hit.node.kind,
                SyntaxNodeKind::DomainKeyword(DomainKeyword::In)
            ));
            assert_eq!(hit.node.span.to_range(), offset..offset + 2);
        }
        if kind == 4 {
            let hit = file.node_at_offset(2).unwrap();
            assert!(
                !hit.ancestors
                    .iter()
                    .any(|node| matches!(node.kind, SyntaxNodeKind::AnnotationKeyword(_)))
            );
        }
    }
}

#[test]
fn duplicate_refinements_retain_occurrence_order() {
    for kind in [2, 3] {
        let file = file(kind, false);
        for (offset, _) in file.text.match_indices("and") {
            let hit = file.node_at_offset(offset).unwrap();
            assert!(matches!(
                hit.node.kind,
                SyntaxNodeKind::DomainKeyword(DomainKeyword::And)
            ));
            assert_eq!(hit.node.span.to_range(), offset..offset + 3);
            let annotation = hit
                .ancestors
                .iter()
                .find_map(|node| match node.kind {
                    SyntaxNodeKind::Annotation(annotation) => Some(annotation),
                    _ => None,
                })
                .unwrap();
            let refinements = match &annotation.value {
                Annotation::Refined { refinements, .. }
                | Annotation::Predicate { refinements, .. } => refinements,
                _ => unreachable!(),
            };
            let ranges: Vec<_> = refinements
                .iter()
                .map(|value| file.span_table.get(value.span_id).to_range())
                .collect();
            assert_eq!(ranges, [6..11, 12..17]);
        }
        for offset in [11, 17] {
            let hit = file.node_at_offset(offset).unwrap();
            assert!(matches!(hit.node.kind,
                SyntaxNodeKind::Expression(expr) if expr.value == Expr::Missing));
        }
    }
}

#[test]
fn duplicate_annotations_keep_distinct_source_occurrences() {
    let mut file = SourceFile {
        text: "x Int\nx Int".into(),
        span_table: SpanTable::new(),
        items: vec![],
        comments: vec![],
    };
    for start in [0, 6] {
        let name = at(&mut file.span_table, "x".into(), start..start + 1);
        let expression = at(
            &mut file.span_table,
            Expr::Name("Int".into()),
            start + 2..start + 5,
        );
        let annotation = Spanned::new(
            Annotation::Expression(expression.clone()),
            expression.span_id,
        );
        let binding = Binding {
            attributes: Vec::new(),
            name,
            parameters: None,
            convention: None,
            result_binder: None,
            annotation: Some(annotation),
            contract: None,
            alternatives: Vec::new(),
            extra_results: Vec::new(),
            value: None,
        };
        file.items.push(at(
            &mut file.span_table,
            Item::Binding(Box::new(binding)),
            start..start + 5,
        ));
    }
    let owners: Vec<_> = file
        .declaration_outline()
        .map(|owner| owner.span.to_range())
        .collect();
    assert_eq!(owners, [0..5, 6..11]);
    for start in [0, 6] {
        let hit = file.node_at_offset(start + 2).unwrap();
        assert_eq!(hit.declarations[0].span.to_range(), start..start + 5);
        assert!(hit.ancestors.iter().any(|node| matches!(
            node.kind,
            SyntaxNodeKind::Annotation(_)
        ) && node.span.to_range()
            == (start + 2..start + 5)));
    }
}
