use ast::{
    Annotation, Binding, DeclarationKind, Expr, Item, SourceFile, SpanTable, Spanned,
    SyntaxNodeKind, TypeBody, TypeDeclaration,
};

fn type_occurrences() -> SourceFile {
    let source = "Thing is Int\nprivate\nThing has\n    member Int\n";
    let mut spans = SpanTable::new();
    let first_name = Spanned::with_range(&mut spans, "Thing".into(), 0..5);
    let annotation = Spanned::with_range(&mut spans, Expr::Name("Int".into()), 9..12);
    let body = Spanned::with_range(&mut spans, TypeBody::Is(annotation), 6..12);
    let first = TypeDeclaration {
        name: first_name,
        parameters: None,
        body,
    };
    let first = Spanned::with_range(&mut spans, Item::Type(Box::new(first)), 0..12);
    let private = Spanned::with_range(&mut spans, Item::Private, 13..20);
    let second_name = Spanned::with_range(&mut spans, "Thing".into(), 21..26);
    let member_name = Spanned::with_range(&mut spans, "member".into(), 35..41);
    let annotation = Spanned::with_range(&mut spans, Expr::Name("Int".into()), 42..45);
    let member = Binding {
        attributes: Vec::new(),
        name: member_name,
        parameters: None,
        convention: None,
        result_binder: None,
        annotation: Some(Spanned::new(
            Annotation::Expression(annotation.clone()),
            annotation.span_id,
        )),
        contract: None,
        alternatives: Vec::new(),
        extra_results: Vec::new(),
        value: None,
    };
    let member = Spanned::with_range(
        &mut spans,
        ast::TypeMember::Binding(Box::new(member)),
        35..45,
    );
    let body = Spanned::with_range(&mut spans, TypeBody::Has(vec![member]), 27..45);
    let second = TypeDeclaration {
        name: second_name,
        parameters: None,
        body,
    };
    let second = Spanned::with_range(&mut spans, Item::Type(Box::new(second)), 21..45);
    SourceFile {
        text: source.into(),
        span_table: spans,
        items: vec![first, private, second],
        comments: vec![],
    }
}

#[test]
fn type_definitions_and_extensions_remain_separate_without_member_declarations() {
    let file = type_occurrences();
    let source = file.text.as_ref();
    let declarations: Vec<_> = file.declaration_outline().collect();
    assert_eq!(declarations.len(), 2);
    assert_eq!(declarations[0].span.extract(source), "Thing is Int");
    assert_eq!(
        declarations[1].span.extract(source),
        "Thing has\n    member Int"
    );
    for declaration in declarations {
        assert_eq!(declaration.kind, DeclarationKind::Type);
        assert_eq!(declaration.name, "Thing");
        assert_eq!(declaration.name_span.extract(source), "Thing");
    }
}

#[test]
fn cursor_queries_distinguish_type_owners_and_written_members() {
    let file = type_occurrences();
    let alias = file.node_at_offset(10).unwrap();
    assert!(matches!(alias.node.kind, SyntaxNodeKind::Name("Int")));
    assert_eq!(alias.declarations.len(), 1);
    assert_eq!(alias.declarations[0].kind, DeclarationKind::Type);
    assert_eq!(alias.declarations[0].span.to_range(), 0..12);
    let member = file.node_at_offset(36).unwrap();
    assert!(matches!(member.node.kind, SyntaxNodeKind::Name("member")));
    assert_eq!(member.declarations.len(), 2);
    assert_eq!(member.declarations[0].name, "Thing");
    assert_eq!(member.declarations[0].span.to_range(), 21..45);
    assert_eq!(member.declarations[1].name, "member");
    assert_eq!(member.declarations[1].kind, DeclarationKind::Binding);
    let keyword = file.node_at_offset(27).unwrap();
    assert!(matches!(
        keyword.node.kind, SyntaxNodeKind::TypeBody(body) if matches!(body.value, TypeBody::Has(_))
    ));
}
