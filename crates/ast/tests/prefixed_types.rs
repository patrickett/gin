use ast::*;
use std::ops::Range;

fn at<T>(spans: &mut SpanTable, value: T, range: Range<usize>) -> Spanned<T> {
    Spanned::with_range(spans, value, range)
}

fn group_path(spans: &mut SpanTable, text: &str, spelling: &str) -> Spanned<GroupPath> {
    let start = text.find(spelling).unwrap();
    let end = start + spelling.len();
    let components = spelling
        .char_indices()
        .filter(|(_, character)| character.is_ascii_alphabetic())
        .map(|(offset, character)| {
            at(
                spans,
                character.to_string(),
                start + offset..start + offset + 1,
            )
        })
        .collect();
    let value = GroupPath {
        open: spans.insert_range(start..start + 1),
        components,
        close: spelling
            .ends_with('}')
            .then(|| spans.insert_range(end - 1..end)),
    };
    at(spans, value, start..end)
}

fn file(convention_group: Option<&str>, borrow_group: Option<&str>) -> SourceFile {
    let text = format!(
        "f(x own{} own ref{} mut T)",
        convention_group.unwrap_or(""),
        borrow_group.unwrap_or("")
    );
    let mut spans = SpanTable::new();
    let end = text.len();
    let target_start = text.find(" T").unwrap() + 1;
    let target = at(&mut spans, Expr::Name("T".into()), target_start..end - 1);
    let inner_start = text.find("mut").unwrap();
    let keyword = at(
        &mut spans,
        TypePrefixKind::Mut,
        inner_start..inner_start + 3,
    );
    let inner = at(
        &mut spans,
        Expr::PrefixedType(PrefixedType {
            keyword,
            group: None,
            target: Box::new(target),
        }),
        inner_start..end - 1,
    );
    let outer_start = text.find("ref").unwrap();
    let keyword = at(
        &mut spans,
        TypePrefixKind::Ref,
        outer_start..outer_start + 3,
    );
    let group = borrow_group.map(|value| group_path(&mut spans, &text, value));
    let outer = at(
        &mut spans,
        Expr::PrefixedType(PrefixedType {
            keyword,
            group,
            target: Box::new(inner),
        }),
        outer_start..end - 1,
    );
    let prefix_start = outer_start - 4;
    let keyword = at(
        &mut spans,
        TypePrefixKind::Own,
        prefix_start..prefix_start + 3,
    );
    let outer = at(
        &mut spans,
        Expr::PrefixedType(PrefixedType {
            keyword,
            group: None,
            target: Box::new(outer),
        }),
        prefix_start..end - 1,
    );
    let annotation = at(
        &mut spans,
        Annotation::Expression(outer),
        prefix_start..end - 1,
    );
    parameter_file(text, spans, annotation, convention_group, prefix_start)
}

fn parameter_file(
    text: String,
    mut spans: SpanTable,
    annotation: Spanned<Annotation>,
    convention_group: Option<&str>,
    prefix_start: usize,
) -> SourceFile {
    let end = text.len();
    let keyword = at(&mut spans, Convention::Own, 4..7);
    let group = convention_group.map(|value| group_path(&mut spans, &text, value));
    let convention = at(
        &mut spans,
        ParameterConvention { keyword, group },
        4..prefix_start - 1,
    );
    let parameter = Parameter {
        name: at(&mut spans, "x".into(), 2..3),
        convention: Some(convention),
        annotation: Some(annotation),
        default: None,
    };
    let parameter = at(&mut spans, parameter, 2..end - 1);
    let binding = Binding {
        attributes: Vec::new(),
        name: at(&mut spans, "f".into(), 0..1),
        parameters: Some(at(&mut spans, vec![parameter], 1..end)),
        convention: None,
        result_binder: None,
        annotation: None,
        contract: None,
        alternatives: Vec::new(),
        extra_results: Vec::new(),
        value: None,
    };
    let item = at(&mut spans, Item::Binding(Box::new(binding)), 0..end);
    SourceFile {
        text: text.into(),
        span_table: spans,
        items: vec![item],
        comments: vec![],
    }
}

#[test]
fn conventions_and_nested_type_prefixes_are_distinct_written_layers() {
    let file = file(Some("{a . b}"), Some("{b.a}"));
    let own = file.node_at_offset(4).unwrap();
    assert!(matches!(own.node.kind,
        SyntaxNodeKind::Convention(keyword) if keyword.value == Convention::Own));
    assert_eq!(own.node.span.to_range(), 4..7);
    assert!(matches!(own.ancestors.last().unwrap().kind,
        SyntaxNodeKind::ParameterConvention(convention)
            if convention.value.group.is_some()));
    for (word, kind, layers) in [
        ("own ref", TypePrefixKind::Own, 1),
        ("ref", TypePrefixKind::Ref, 2),
        ("mut", TypePrefixKind::Mut, 3),
    ] {
        let start = file.text.find(word).unwrap();
        let hit = file.node_at_offset(start).unwrap();
        assert!(matches!(hit.node.kind,
            SyntaxNodeKind::TypePrefixKeyword(keyword) if keyword.value == kind));
        let SyntaxNodeKind::TypePrefixKeyword(keyword) = hit.node.kind else {
            unreachable!()
        };
        assert_eq!(file.span_table.get(keyword.span_id), hit.node.span);
        assert_eq!(hit.node.span.to_range(), start..start + 3);
        assert_eq!(format!("{:?}", hit.node.kind), "TypePrefixKeyword");
        assert_eq!(hit.declarations[0].name, "f");
        assert!(hit.ancestors.iter().any(|node| matches!(node.kind,
            SyntaxNodeKind::Parameter(parameter) if parameter.value.default.is_none())));
        assert!(hit.ancestors.iter().any(|node| matches!(node.kind,
            SyntaxNodeKind::Binding(binding)
                if binding.annotation.is_none() && binding.value.is_none())));
        assert_eq!(
            hit.ancestors
                .iter()
                .filter(|node| matches!(node.kind,
                SyntaxNodeKind::Expression(expression)
                    if matches!(expression.value, Expr::PrefixedType(_))))
                .count(),
            layers
        );
        assert!(
            !hit.ancestors
                .iter()
                .any(|node| matches!(node.kind, SyntaxNodeKind::ParameterConvention(_)))
        );
    }
}

#[test]
fn groups_query_components_braces_and_raw_punctuation_with_contained_owners() {
    let file = file(Some("{a . b}"), Some("{b.a}"));
    for spelling in ["{a . b}", "{b.a}"] {
        let start = file.text.find(spelling).unwrap();
        for (relative, character) in spelling.char_indices() {
            let hit = file.node_at_offset(start + relative).unwrap();
            match character {
                '{' => assert!(matches!(
                    hit.node.kind,
                    SyntaxNodeKind::GroupDelimiter(GroupDelimiter::Open)
                )),
                '}' => assert!(matches!(
                    hit.node.kind,
                    SyntaxNodeKind::GroupDelimiter(GroupDelimiter::Close)
                )),
                'a' | 'b' => assert!(matches!(hit.node.kind,
                    SyntaxNodeKind::Name(name) if name == character.to_string())),
                '.' => assert!(matches!(hit.node.kind, SyntaxNodeKind::GroupPath(_))),
                ' ' => continue,
                _ => unreachable!(),
            }
            assert_eq!(hit.declarations.len(), 1);
            assert_eq!(hit.declarations[0].name, "f");
            let mut enclosing = hit.declarations[0].span;
            for node in hit.ancestors.iter().chain(std::iter::once(&hit.node)) {
                assert!(node.span.start() >= enclosing.start());
                assert!(node.span.end() <= enclosing.end());
                enclosing = node.span;
            }
            assert!(format!("{:?}", hit.node.kind).len() < 32);
            let group = hit
                .ancestors
                .iter()
                .chain(std::iter::once(&hit.node))
                .find_map(|node| match node.kind {
                    SyntaxNodeKind::GroupPath(group) => Some(group),
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                format!("{:?}", SyntaxNodeKind::GroupPath(group)),
                "GroupPath"
            );
            assert_eq!(
                file.span_table.get(group.span_id).to_range(),
                start..start + spelling.len()
            );
        }
    }
}

#[test]
fn absent_groups_and_empty_recovered_groups_remain_different() {
    for group in [None, Some("{"), Some("{}")] {
        let file = file(group, group.map(|_| "{  "));
        let hit = file.node_at_offset(4).unwrap();
        let convention = hit
            .ancestors
            .iter()
            .find_map(|node| match node.kind {
                SyntaxNodeKind::ParameterConvention(value) => Some(&value.value),
                _ => None,
            })
            .unwrap();
        assert_eq!(convention.group.is_some(), group.is_some());
        if let Some(path) = &convention.group {
            assert!(path.value.components.is_empty());
            assert_eq!(path.value.close.is_some(), group == Some("{}"));
        }
        let offset = file.text.find("ref").unwrap();
        let hit = file.node_at_offset(offset).unwrap();
        let expression = hit.ancestors.last().unwrap();
        let SyntaxNodeKind::Expression(expression) = expression.kind else {
            panic!()
        };
        let Expr::PrefixedType(borrowed) = &expression.value else {
            panic!()
        };
        assert_eq!(borrowed.group.is_some(), group.is_some());
        if let Some(path) = &borrowed.group {
            assert!(path.value.components.is_empty());
            assert!(path.value.close.is_none());
            let open = file.span_table.get(path.value.open).start();
            assert!(matches!(
                file.node_at_offset(open).unwrap().node.kind,
                SyntaxNodeKind::GroupDelimiter(GroupDelimiter::Open)
            ));
        }
    }
}

#[test]
fn deep_prefix_queries_use_iterative_traversal_and_bounded_debug() {
    const PREFIXES: usize = 512;
    let text = format!("{}T", "ref ".repeat(PREFIXES));
    let mut spans = SpanTable::new();
    let mut expression = at(
        &mut spans,
        Expr::Name("T".into()),
        text.len() - 1..text.len(),
    );
    for index in (0..PREFIXES).rev() {
        let start = index * 4;
        let keyword = at(&mut spans, TypePrefixKind::Ref, start..start + 3);
        expression = at(
            &mut spans,
            Expr::PrefixedType(PrefixedType {
                keyword,
                group: None,
                target: Box::new(expression),
            }),
            start..text.len(),
        );
    }
    let item = at(&mut spans, Item::Expression(expression), 0..text.len());
    let file = SourceFile {
        text: text.into(),
        span_table: spans,
        items: vec![item],
        comments: vec![],
    };
    let hit = file.node_at_offset(file.text.len() - 1).unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("T")));
    assert_eq!(hit.ancestors.len(), PREFIXES + 3);
    assert!(hit.declarations.is_empty());
    for node in &hit.ancestors {
        assert!(format!("{:?}", node.kind).len() < 32);
    }
}

#[test]
fn missing_targets_and_own_groups_are_retained_without_invented_syntax() {
    for kind in [
        TypePrefixKind::Own,
        TypePrefixKind::Ref,
        TypePrefixKind::Mut,
        TypePrefixKind::Pointer,
    ] {
        let text = match kind {
            TypePrefixKind::Own => "own{g}",
            TypePrefixKind::Ref => "ref{g}",
            TypePrefixKind::Mut => "mut{g}",
            TypePrefixKind::Pointer => "@{g}",
        };
        let mut spans = SpanTable::new();
        let target = at(&mut spans, Expr::Missing, text.len()..text.len());
        let keyword = at(&mut spans, kind, 0..text.find('{').unwrap());
        let group = Some(group_path(&mut spans, text, "{g}"));
        let expression = at(
            &mut spans,
            Expr::PrefixedType(PrefixedType {
                keyword,
                group,
                target: Box::new(target),
            }),
            0..text.len(),
        );
        let item = at(&mut spans, Item::Expression(expression), 0..text.len());
        let file = SourceFile {
            text: text.into(),
            span_table: spans,
            items: vec![item],
            comments: vec![],
        };
        let hit = file.node_at_offset(text.len()).unwrap();
        assert!(matches!(hit.node.kind,
            SyntaxNodeKind::Expression(expression) if expression.value == Expr::Missing));
        assert_eq!(hit.node.span.to_range(), text.len()..text.len());
        for ancestor in &hit.ancestors {
            assert!(ancestor.span.start() <= hit.node.span.start());
            assert!(hit.node.span.end() <= ancestor.span.end());
        }
        assert!(matches!(
            file.node_at_offset(text.find('{').unwrap())
                .unwrap()
                .node
                .kind,
            SyntaxNodeKind::GroupDelimiter(GroupDelimiter::Open)
        ));
    }
}
