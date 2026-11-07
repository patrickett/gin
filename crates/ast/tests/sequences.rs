use ast::*;

#[test]
fn bare_return_sequence_queries_keep_commas_without_synthetic_delimiters() {
    let text = "return a,b";
    let mut spans = SpanTable::new();
    let first = Spanned::with_range(&mut spans, Expr::Name("a".into()), 7..8);
    let second = Spanned::with_range(&mut spans, Expr::Name("b".into()), 9..10);
    let separator = spans.insert(Span::new(8, 9));
    let sequence = SequenceExpression {
        delimiters: SequenceDelimiters::Bare,
        elements: vec![first, second],
        separators: vec![separator],
    };
    let value = Spanned::with_range(&mut spans, Expr::Sequence(Box::new(sequence)), 7..10);
    let statement = Spanned::with_range(
        &mut spans,
        Statement::Return(ReturnStatement {
            value: Some(value),
        }),
        0..text.len(),
    );
    let expression = Spanned::with_range(
        &mut spans,
        Expr::Block(Block {
            statements: vec![statement],
        }),
        0..text.len(),
    );
    let item = Spanned::with_range(&mut spans, Item::Expression(expression), 0..text.len());
    let file = SourceFile {
        text: text.into(),
        span_table: spans,
        items: vec![item],
        comments: Vec::new(),
    };
    let hit = file.node_at_offset(8).unwrap();
    assert!(matches!(
        hit.node.kind,
        SyntaxNodeKind::CollectionPunctuation(CollectionPunctuation::Comma)
    ));
    assert_eq!(hit.node.span.extract(text), ",");
    assert!(hit.ancestors.iter().any(|node| matches!(node.kind,
        SyntaxNodeKind::Sequence(sequence) if sequence.delimiters == SequenceDelimiters::Bare)));
    for offset in 7..10 {
        let hit = file.node_at_offset(offset).unwrap();
        assert!(!matches!(
            hit.node.kind,
            SyntaxNodeKind::CollectionPunctuation(
                CollectionPunctuation::Open | CollectionPunctuation::Close
            )
        ));
    }
}

#[test]
fn raw_sequence_and_numeric_select_views_borrow_nodes_without_brackets() {
    let text = "(x,).0";
    let mut spans = SpanTable::new();
    let element = Spanned::with_range(&mut spans, Expr::Name("x".into()), 1..2);
    let sequence = SequenceExpression {
        delimiters: SequenceDelimiters::Parenthesized {
            open: spans.insert(Span::new(0, 1)),
            close: spans.insert(Span::new(3, 4)),
        },
        elements: vec![element],
        separators: vec![spans.insert(Span::new(2, 3))],
    };
    let base = Spanned::with_range(&mut spans, Expr::Sequence(Box::new(sequence)), 0..4);
    let select = NumericSelectExpression {
        base,
        dot: spans.insert(Span::new(4, 5)),
        index: Spanned::with_range(&mut spans, Expr::Literal(LiteralKind::Integer), 5..6),
    };
    let expression = Spanned::with_range(
        &mut spans,
        Expr::NumericSelect(Box::new(select)),
        0..text.len(),
    );
    let item = Spanned::with_range(&mut spans, Item::Expression(expression), 0..text.len());
    let file = SourceFile {
        text: text.into(),
        span_table: spans,
        items: vec![item],
        comments: Vec::new(),
    };
    let Item::Expression(expression) = &file.items[0].value else {
        panic!("expression")
    };
    let Expr::NumericSelect(select) = &expression.value else {
        panic!("numeric select")
    };
    let hit = file.node_at_offset(4).unwrap();
    assert!(matches!(
        hit.node.kind,
        SyntaxNodeKind::CollectionPunctuation(CollectionPunctuation::Dot)
    ));
    assert_eq!(hit.node.span.extract(text), ".");
    assert!(hit.ancestors.iter().any(|node| matches!(
        node.kind, SyntaxNodeKind::NumericSelect(found) if std::ptr::eq(found, select.as_ref())
    )));
    let hit = file.node_at_offset(2).unwrap();
    assert!(matches!(
        hit.node.kind,
        SyntaxNodeKind::CollectionPunctuation(CollectionPunctuation::Comma)
    ));
    assert!(
        hit.ancestors
            .iter()
            .any(|node| matches!(node.kind, SyntaxNodeKind::Sequence(_)))
    );
    let hit = file.node_at_offset(5).unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Expression(index)
        if index.value == Expr::Literal(LiteralKind::Integer)));
    assert!(
        hit.ancestors
            .iter()
            .any(|node| format!("{:?}", node.kind) == "NumericSelect")
    );
    assert_eq!(file.declaration_outline().count(), 0);
}
