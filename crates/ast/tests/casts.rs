use ast::*;

fn file(missing: bool) -> SourceFile {
    let text = if missing {
        "'hé' as @"
    } else {
        "'hé' as @Outer"
    };
    let mut spans = SpanTable::new();
    let value = Spanned::with_range(&mut spans, Expr::Literal(LiteralKind::String), 0..5);
    let as_keyword = spans.insert_range(6..8);
    let keyword = Spanned::with_range(&mut spans, TypePrefixKind::Pointer, 9..10);
    let target = if missing {
        Spanned::with_range(&mut spans, Expr::Missing, 10..10)
    } else {
        Spanned::with_range(&mut spans, Expr::Name("Outer".into()), 10..15)
    };
    let target = Spanned::with_range(
        &mut spans,
        Expr::PrefixedType(PrefixedType {
            keyword,
            group: None,
            target: Box::new(target),
        }),
        9..text.len(),
    );
    let expression = Spanned::with_range(
        &mut spans,
        Expr::Cast {
            value: Box::new(value),
            as_keyword,
            target: Box::new(target),
        },
        0..text.len(),
    );
    let item = Spanned::with_range(&mut spans, Item::Expression(expression), 0..text.len());
    SourceFile {
        text: text.into(),
        span_table: spans,
        items: vec![item],
        comments: Vec::new(),
    }
}

#[test]
fn casts_query_exact_keyword_pointer_and_operand_spans_without_semantics() {
    let file = file(false);
    let hit = file.node_at_offset(6).unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::AsKeyword));
    assert_eq!(hit.node.span.extract(&file.text), "as");
    assert_eq!(format!("{:?}", hit.node.kind), "AsKeyword");
    let Item::Expression(expression) = &file.items[0].value else {
        panic!("cast expression");
    };
    assert!(hit.ancestors.iter().any(|node| matches!(node.kind,
        SyntaxNodeKind::Expression(borrowed) if std::ptr::eq(borrowed, expression))));
    let hit = file.node_at_offset(9).unwrap();
    assert!(matches!(hit.node.kind,
        SyntaxNodeKind::TypePrefixKeyword(keyword) if keyword.value == TypePrefixKind::Pointer));
    assert_eq!(hit.node.span.extract(&file.text), "@");
    let hit = file.node_at_offset(10).unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("Outer")));
    assert!(hit.declarations.is_empty());
    assert_eq!(file.declaration_outline().count(), 0);
    assert!(file.node_at_offset(3).is_none());
}

#[test]
fn missing_pointer_cast_target_wins_at_eof() {
    let file = file(true);
    let hit = file.node_at_offset(file.text.len()).unwrap();
    assert!(matches!(hit.node.kind,
        SyntaxNodeKind::Expression(expression) if expression.value == Expr::Missing));
    assert_eq!(hit.node.span.to_range(), 10..10);
    assert!(hit.ancestors.iter().any(|node| matches!(node.kind,
        SyntaxNodeKind::Expression(expression) if matches!(expression.value, Expr::Cast { .. }))));
}
