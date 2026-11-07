use ast::*;
use std::ops::Range;

fn at<T>(spans: &mut SpanTable, value: T, range: Range<usize>) -> Spanned<T> {
    Spanned::with_range(spans, value, range)
}

fn file(text: &str, spans: &mut SpanTable, binding: Binding) -> SourceFile {
    let item = at(spans, Item::Binding(Box::new(binding)), 0..text.len());
    SourceFile {
        text: text.into(),
        span_table: spans.clone(),
        items: vec![item],
        comments: vec![],
    }
}

fn result_file(text: &str, explicit: bool, missing: bool) -> SourceFile {
    let mut spans = SpanTable::new();
    let convention = explicit.then(|| at(&mut spans, BindingConvention::Own, 2..5));
    let annotation = missing.then(|| {
        let expression = at(&mut spans, Expr::Missing, text.len()..text.len());
        at(
            &mut spans,
            Annotation::Expression(expression),
            text.len()..text.len(),
        )
    });
    let binding = Binding {
        attributes: Vec::new(),
        name: at(&mut spans, "f".into(), 0..1),
        parameters: None,
        convention,
        result_binder: None,
        annotation,
        contract: None,
        alternatives: Vec::new(),
        extra_results: Vec::new(),
        value: None,
    };
    file(text, &mut spans, binding)
}

#[test]
fn explicit_keyword_has_exact_span_owner_and_bounded_debug() {
    let file = result_file("f own", true, false);
    for offset in 2..5 {
        let hit = file.node_at_offset(offset).unwrap();
        let SyntaxNodeKind::BindingConvention(convention) = hit.node.kind else {
            panic!("expected binding convention");
        };
        assert_eq!(convention.value, BindingConvention::Own);
        assert_eq!(hit.node.span.to_range(), 2..5);
        assert_eq!(file.span_table.get(convention.span_id), hit.node.span);
        assert_eq!(format!("{:?}", hit.node.kind), "BindingConvention");
        assert_eq!(hit.declarations.len(), 1);
        assert_eq!(hit.declarations[0].name, "f");
        assert!(hit.ancestors.iter().any(|node| matches!(node.kind,
            SyntaxNodeKind::Binding(binding)
                if binding.annotation.is_none() && binding.parameters.is_none())));
        assert!(!hit.ancestors.iter().any(|node| matches!(
            node.kind,
            SyntaxNodeKind::ParameterConvention(_) | SyntaxNodeKind::GroupPath(_)
        )));
    }
}

#[test]
fn unwritten_convention_and_qualifier_only_header_fabricate_nothing() {
    for (text, explicit) in [("f", false), ("f own", true)] {
        let file = result_file(text, explicit, false);
        let hit = file.node_at_offset(0).unwrap();
        let binding = hit
            .ancestors
            .iter()
            .find_map(|node| match node.kind {
                SyntaxNodeKind::Binding(binding) => Some(binding),
                _ => None,
            })
            .unwrap();
        assert_eq!(binding.convention.is_some(), explicit);
        assert!(binding.annotation.is_none());
        assert!(binding.value.is_none());
    }
}

#[test]
fn end_uses_existing_binding_query_kind_without_classifying_header() {
    let mut file = result_file("f end", true, false);
    let Item::Binding(binding) = &mut file.items[0].value else {
        panic!("binding");
    };
    binding.convention.as_mut().unwrap().value = BindingConvention::End;
    for offset in 2..=5 {
        let hit = file.node_at_offset(offset).unwrap();
        assert!(
            matches!(hit.node.kind, SyntaxNodeKind::BindingConvention(keyword)
            if keyword.value == BindingConvention::End)
        );
        assert_eq!(hit.node.span.to_range(), 2..5);
        assert_eq!(hit.declarations[0].name, "f");
    }
}

#[test]
fn missing_target_at_header_end_is_contained_and_wins() {
    let file = result_file("f own ", true, true);
    let hit = file.node_at_offset(file.text.len()).unwrap();
    assert!(
        matches!(hit.node.kind, SyntaxNodeKind::Expression(expression)
        if matches!(expression.value, Expr::Missing))
    );
    assert_eq!(hit.node.span.to_range(), 6..6);
    for ancestor in &hit.ancestors {
        assert!(ancestor.span.start() <= hit.node.span.start());
        assert!(ancestor.span.end() >= hit.node.span.end());
    }
    assert!(matches!(
        file.node_at_offset(2).unwrap().node.kind,
        SyntaxNodeKind::BindingConvention(_)
    ));
}

fn three_roles_file() -> SourceFile {
    let text = "f(x own T) own own T";
    let mut spans = SpanTable::new();
    let keyword = at(&mut spans, Convention::Own, 4..7);
    let convention = at(
        &mut spans,
        ParameterConvention {
            keyword,
            group: None,
        },
        4..7,
    );
    let target = at(&mut spans, Expr::Name("T".into()), 8..9);
    let annotation = at(&mut spans, Annotation::Expression(target), 8..9);
    let parameter = Parameter {
        name: at(&mut spans, "x".into(), 2..3),
        convention: Some(convention),
        annotation: Some(annotation),
        default: None,
    };
    let parameter = at(&mut spans, parameter, 2..9);
    let target = at(&mut spans, Expr::Name("T".into()), 19..20);
    let keyword = at(&mut spans, TypePrefixKind::Own, 15..18);
    let expression = at(
        &mut spans,
        Expr::PrefixedType(PrefixedType {
            keyword,
            group: None,
            target: Box::new(target),
        }),
        15..20,
    );
    let binding = Binding {
        attributes: Vec::new(),
        name: at(&mut spans, "f".into(), 0..1),
        parameters: Some(at(&mut spans, vec![parameter], 1..10)),
        convention: Some(at(&mut spans, BindingConvention::Own, 11..14)),
        result_binder: None,
        annotation: Some(at(&mut spans, Annotation::Expression(expression), 15..20)),
        contract: None,
        alternatives: Vec::new(),
        extra_results: Vec::new(),
        value: None,
    };
    file(text, &mut spans, binding)
}

#[test]
fn parameter_result_and_storage_own_are_distinct_written_roles() {
    let file = three_roles_file();
    let parameter = file.node_at_offset(4).unwrap();
    assert!(
        matches!(parameter.node.kind, SyntaxNodeKind::Convention(keyword)
        if keyword.value == Convention::Own)
    );
    assert!(
        parameter
            .ancestors
            .iter()
            .any(|node| matches!(node.kind, SyntaxNodeKind::ParameterConvention(_)))
    );
    let result = file.node_at_offset(11).unwrap();
    assert!(
        matches!(result.node.kind, SyntaxNodeKind::BindingConvention(keyword)
        if keyword.value == BindingConvention::Own)
    );
    assert_eq!(result.node.span.to_range(), 11..14);
    let storage = file.node_at_offset(15).unwrap();
    assert!(
        matches!(storage.node.kind, SyntaxNodeKind::TypePrefixKeyword(keyword)
        if keyword.value == TypePrefixKind::Own)
    );
    assert_eq!(storage.node.span.to_range(), 15..18);
    assert!(
        storage
            .ancestors
            .iter()
            .any(|node| matches!(node.kind, SyntaxNodeKind::Annotation(_)))
    );
}

#[test]
fn end_parameter_keyword_and_aggregate_share_exact_span() {
    let mut file = three_roles_file();
    let Item::Binding(binding) = &mut file.items[0].value else {
        panic!("binding");
    };
    let parameter = &mut binding.parameters.as_mut().unwrap().value[0].value;
    parameter.convention.as_mut().unwrap().value.keyword.value = Convention::End;
    file.text = "f(x end T) own own T".into();
    for offset in 4..7 {
        let hit = file.node_at_offset(offset).unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::Convention(keyword)
            if keyword.value == Convention::End));
        assert_eq!(hit.node.span.to_range(), 4..7);
        assert!(hit.ancestors.iter().any(|node| matches!(
            node.kind,
            SyntaxNodeKind::ParameterConvention(_)
        ) && node.span.to_range() == (4..7)));
    }
}
