use ast::*;
use std::ops::Range;

fn at<T>(spans: &mut SpanTable, value: T, range: Range<usize>) -> Spanned<T> {
    Spanned::with_range(spans, value, range)
}

fn member_file(text: String, spans: &mut SpanTable, member: Binding) -> SourceFile {
    let end = text.len();
    let member = at(spans, TypeMember::Binding(Box::new(member)), 6..end);
    let name = at(spans, "T".into(), 0..1);
    let body = at(spans, TypeBody::Has(vec![member]), 2..end);
    let declaration = TypeDeclaration {
        name,
        parameters: None,
        body,
    };
    let item = at(spans, Item::Type(Box::new(declaration)), 0..end);
    SourceFile {
        text: text.into(),
        span_table: spans.clone(),
        items: vec![item],
        comments: vec![],
    }
}

fn parameter_file(name: &str, keyword: Option<(&str, Convention)>) -> SourceFile {
    let mut spans = SpanTable::new();
    let mut text = format!("T has f({name}");
    let name = at(&mut spans, name.into(), 8..text.len());
    let convention = keyword.map(|(word, value)| {
        text.push(' ');
        let start = text.len();
        text.push_str(word);
        let keyword = at(&mut spans, value, start..text.len());
        at(
            &mut spans,
            ParameterConvention {
                keyword,
                group: None,
            },
            start..text.len(),
        )
    });
    let parameter_end = text.len();
    text.push(')');
    let parameter = Parameter {
        name,
        convention,
        annotation: None,
        default: None,
    };
    let parameter = at(&mut spans, parameter, 8..parameter_end);
    let parameters = Some(at(&mut spans, vec![parameter], 7..text.len()));
    let member = Binding {
        attributes: Vec::new(),
        name: at(&mut spans, "f".into(), 6..7),
        parameters,
        convention: None,
        result_binder: None,
        annotation: None,
        contract: None,
        alternatives: Vec::new(),
        extra_results: Vec::new(),
        value: None,
    };
    member_file(text, &mut spans, member)
}

#[test]
fn written_conventions_keep_exact_spans_and_member_owners() {
    for name in ["self", "value"] {
        for (word, value) in [
            ("ref", Convention::Ref),
            ("mut", Convention::Mut),
            ("own", Convention::Own),
        ] {
            let file = parameter_file(name, Some((word, value)));
            let start = 9 + name.len();
            for offset in start..start + word.len() {
                let hit = file.node_at_offset(offset).unwrap();
                let SyntaxNodeKind::Convention(convention) = hit.node.kind else {
                    panic!("expected convention, got {:?}", hit.node.kind);
                };
                assert_eq!(convention.value, value);
                assert_eq!(hit.node.span.to_range(), start..start + word.len());
                assert_eq!(file.span_table.get(convention.span_id), hit.node.span);
                assert_eq!(format!("{:?}", hit.node.kind), "Convention");
                assert_eq!(hit.declarations.len(), 2);
                assert_eq!(hit.declarations[0].name, "T");
                assert_eq!(hit.declarations[1].name, "f");
                assert!(hit.ancestors.iter().any(|node| {
                    matches!(node.kind, SyntaxNodeKind::Parameter(parameter)
                        if parameter.value.annotation.is_none())
                }));
                assert!(
                    !hit.ancestors
                        .iter()
                        .any(|node| { matches!(node.kind, SyntaxNodeKind::Annotation(_)) })
                );
            }
        }
    }
}

#[test]
fn absent_convention_is_not_an_implicit_receiver_or_named_default() {
    for name in ["self", "value"] {
        let file = parameter_file(name, None);
        let hit = file.node_at_offset(8).unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::Name(value) if value == name));
        assert_eq!(hit.node.span.to_range(), 8..8 + name.len());
        let parameter = hit
            .ancestors
            .iter()
            .find_map(|node| match node.kind {
                SyntaxNodeKind::Parameter(parameter) => Some(&parameter.value),
                _ => None,
            })
            .unwrap();
        assert!(parameter.convention.is_none());
        assert!(parameter.annotation.is_none());
        assert!(parameter.default.is_none());
        assert_eq!(hit.declarations[1].name, "f");
    }
}

#[test]
fn field_own_is_a_written_type_prefix_without_initialization() {
    let text = "T has x own Int";
    let mut spans = SpanTable::new();
    let expression = at(&mut spans, Expr::Name("Int".into()), 12..15);
    let keyword = at(&mut spans, TypePrefixKind::Own, 8..11);
    let expression = at(
        &mut spans,
        Expr::PrefixedType(PrefixedType {
            keyword,
            group: None,
            target: Box::new(expression),
        }),
        8..15,
    );
    let annotation = at(&mut spans, Annotation::Expression(expression), 8..15);
    let member = Binding {
        attributes: Vec::new(),
        name: at(&mut spans, "x".into(), 6..7),
        parameters: None,
        convention: None,
        result_binder: None,
        annotation: Some(annotation),
        contract: None,
        alternatives: Vec::new(),
        extra_results: Vec::new(),
        value: None,
    };
    let file = member_file(text.into(), &mut spans, member);
    for offset in 8..11 {
        let hit = file.node_at_offset(offset).unwrap();
        assert!(matches!(hit.node.kind,
            SyntaxNodeKind::TypePrefixKeyword(keyword) if keyword.value == TypePrefixKind::Own));
        assert_eq!(hit.node.span.to_range(), 8..11);
        assert_eq!(format!("{:?}", hit.node.kind), "TypePrefixKeyword");
        assert_eq!(hit.declarations.len(), 2);
        assert_eq!(hit.declarations[0].name, "T");
        assert_eq!(hit.declarations[1].name, "x");
        assert!(hit.ancestors.iter().any(|node| matches!(node.kind,
            SyntaxNodeKind::Binding(binding)
                if binding.parameters.is_none() && binding.value.is_none())));
    }
    let hit = file.node_at_offset(12).unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("Int")));
    assert!(
        hit.ancestors
            .iter()
            .any(|node| { matches!(node.kind, SyntaxNodeKind::Annotation(_)) })
    );
}

#[test]
fn receiver_bare_and_keeps_no_nominal_base_or_is_keyword() {
    let text = "T has f(self mut and ready)";
    let mut spans = SpanTable::new();
    let expression = at(&mut spans, Expr::Name("ready".into()), 21..26);
    let predicate = at(
        &mut spans,
        RefinementPredicate::Expression(expression),
        21..26,
    );
    let refinement = Refinement {
        and_keyword: spans.insert_range(17..20),
        predicate,
    };
    let refinement = at(&mut spans, refinement, 17..26);
    let annotation = at(
        &mut spans,
        Annotation::Refined {
            base: None,
            refinements: vec![refinement],
        },
        17..26,
    );
    let keyword = at(&mut spans, Convention::Mut, 13..16);
    let parameter = Parameter {
        name: at(&mut spans, "self".into(), 8..12),
        convention: Some(at(
            &mut spans,
            ParameterConvention {
                keyword,
                group: None,
            },
            13..16,
        )),
        annotation: Some(annotation),
        default: None,
    };
    let parameter = at(&mut spans, parameter, 8..26);
    let member = Binding {
        attributes: Vec::new(),
        name: at(&mut spans, "f".into(), 6..7),
        parameters: Some(at(&mut spans, vec![parameter], 7..27)),
        convention: None,
        result_binder: None,
        annotation: None,
        contract: None,
        alternatives: Vec::new(),
        extra_results: Vec::new(),
        value: None,
    };
    let file = member_file(text.into(), &mut spans, member);
    let hit = file.node_at_offset(17).unwrap();
    assert!(matches!(
        hit.node.kind,
        SyntaxNodeKind::DomainKeyword(DomainKeyword::And)
    ));
    assert_eq!(hit.node.span.to_range(), 17..20);
    assert_eq!(hit.declarations[1].name, "f");
    assert!(hit.ancestors.iter().any(|node| matches!(
        node.kind,
        SyntaxNodeKind::Annotation(annotation)
            if matches!(&annotation.value, Annotation::Refined { base: None, refinements }
                if refinements.len() == 1)
    )));
    for offset in 0..text.len() {
        let hit = file.node_at_offset(offset).unwrap();
        assert!(!matches!(
            hit.node.kind,
            SyntaxNodeKind::AnnotationKeyword(_) | SyntaxNodeKind::Name("Self")
        ));
    }
}
