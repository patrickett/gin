use ast::*;

fn located<T>(spans: &mut SpanTable, value: T, start: usize, end: usize) -> Spanned<T> {
    Spanned::with_range(spans, value, start..end)
}

#[test]
fn collection_views_borrow_original_nodes_and_keep_written_recovery() {
    let text = "['hé'].[0;]";
    let mut spans = SpanTable::new();
    let literal = located(&mut spans, Expr::Literal(LiteralKind::String), 1, 6);
    let array = ArrayExpression {
        open: spans.insert(Span::new(0, 1)),
        elements: vec![literal],
        labels: vec![None],
        separators: Vec::new(),
        length: None,
        close: spans.insert(Span::new(6, 7)),
    };
    let base = located(&mut spans, Expr::Array(Box::new(array)), 0, 7);
    let index = located(&mut spans, Expr::Literal(LiteralKind::Integer), 9, 10);
    let missing = located(&mut spans, Expr::Missing, 11, 11);
    let select = SelectExpression {
        base,
        dot: spans.insert(Span::new(7, 8)),
        open: spans.insert(Span::new(8, 9)),
        index,
        length: Some(ArrayLength {
            separator: spans.insert(Span::new(10, 11)),
            value: missing,
        }),
        close: spans.insert(Span::new(11, 12)),
    };
    let expression = located(&mut spans, Expr::Select(Box::new(select)), 0, text.len());
    let item = located(&mut spans, Item::Expression(expression), 0, text.len());
    let file = SourceFile {
        text: text.into(),
        span_table: spans,
        items: vec![item],
        comments: Vec::new(),
    };
    let Item::Expression(expression) = &file.items[0].value else {
        panic!("expression")
    };
    let Expr::Select(select) = &expression.value else {
        panic!("selection")
    };
    let hit = file.node_at_offset(10).unwrap();
    assert!(matches!(
        hit.node.kind,
        SyntaxNodeKind::CollectionPunctuation(CollectionPunctuation::Semicolon)
    ));
    let borrowed = hit
        .ancestors
        .iter()
        .find_map(|node| match node.kind {
            SyntaxNodeKind::Select(select) => Some(select),
            _ => None,
        })
        .unwrap();
    assert!(std::ptr::eq(borrowed, select.as_ref()));
    let hit = file.node_at_offset(11).unwrap();
    assert!(matches!(
        hit.node.kind,
        SyntaxNodeKind::Expression(expression) if expression.value == Expr::Missing
    ));
    assert!(hit.node.span.is_empty());
    assert!(file.node_at_offset(4).is_none(), "UTF-8 continuation byte");
    assert_eq!(
        file.node_at_offset(3).unwrap().node.span.extract(text),
        "'hé'"
    );
}

#[test]
fn raw_loop_binder_and_range_queries_do_not_create_outline_declarations() {
    let text = "for i in 0 to 1 do\n    x";
    let mut spans = SpanTable::new();
    let start = located(&mut spans, Expr::Literal(LiteralKind::Integer), 9, 10);
    let end = located(&mut spans, Expr::Literal(LiteralKind::Integer), 14, 15);
    let to_keyword = spans.insert(Span::new(11, 13));
    let iterable = located(
        &mut spans,
        Expr::Range {
            start: Box::new(start),
            to_keyword,
            end: Box::new(end),
        },
        9,
        15,
    );
    let body = located(&mut spans, Expr::Name("x".into()), 23, 24);
    let loop_expression = LoopExpression::For {
        for_keyword: spans.insert(Span::new(0, 3)),
        binder: located(&mut spans, "i".into(), 4, 5),
        in_keyword: spans.insert(Span::new(6, 8)),
        iterable,
        invariant: None,
        do_keyword: spans.insert(Span::new(16, 18)),
        body,
    };
    let expression = located(
        &mut spans,
        Expr::Loop(Box::new(loop_expression)),
        0,
        text.len(),
    );
    let item = located(&mut spans, Item::Expression(expression), 0, text.len());
    let file = SourceFile {
        text: text.into(),
        span_table: spans,
        items: vec![item],
        comments: Vec::new(),
    };
    assert_eq!(file.declaration_outline().count(), 0);
    let hit = file.node_at_offset(4).unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::LoopBinder(name) if name.value == "i"));
    assert!(hit.declarations.is_empty());
    let hit = file.node_at_offset(11).unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::RangeKeyword));
    assert_eq!(hit.node.span.extract(text), "to");
    assert_eq!(format!("{:?}", hit.node.kind), "RangeKeyword");
    let hit = file.node_at_offset(16).unwrap();
    assert!(matches!(
        hit.node.kind,
        SyntaxNodeKind::LoopKeyword(LoopKeyword::Do)
    ));
    assert_eq!(hit.node.span.extract(text), "do");
}

#[test]
fn raw_missing_for_do_has_a_borrowed_zero_width_keyword_view() {
    let text = "for i in xs\n    x";
    let mut spans = SpanTable::new();
    let iterable = located(&mut spans, Expr::Name("xs".into()), 9, 11);
    let body = located(&mut spans, Expr::Name("x".into()), 16, 17);
    let do_keyword = spans.insert(Span::new(11, 11));
    let loop_expression = LoopExpression::For {
        for_keyword: spans.insert(Span::new(0, 3)),
        binder: located(&mut spans, "i".into(), 4, 5),
        in_keyword: spans.insert(Span::new(6, 8)),
        iterable,
        invariant: None,
        do_keyword,
        body,
    };
    let expression = located(
        &mut spans,
        Expr::Loop(Box::new(loop_expression)),
        0,
        text.len(),
    );
    let item = located(&mut spans, Item::Expression(expression), 0, text.len());
    let file = SourceFile {
        text: text.into(),
        span_table: spans,
        items: vec![item],
        comments: Vec::new(),
    };
    let hit = file.node_at_offset(11).unwrap();
    assert!(matches!(
        hit.node.kind,
        SyntaxNodeKind::LoopKeyword(LoopKeyword::Do)
    ));
    assert!(hit.node.span.is_empty());
    assert!(hit.ancestors.iter().any(|node| {
        matches!(node.kind, SyntaxNodeKind::Loop(LoopExpression::For {
            do_keyword: keyword, ..
        }) if *keyword == do_keyword)
    }));
}
