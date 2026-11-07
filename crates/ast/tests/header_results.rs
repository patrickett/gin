use ast::*;

fn node<T>(file: &mut SourceFile, text: &str, value: T) -> Spanned<T> {
    let start = file.text.find(text).unwrap();
    Spanned::with_range(&mut file.span_table, value, start..start + text.len())
}

/// Builds the `where` clause `text` whose single claim is the name `proposition` (or a
/// written-but-missing claim when `proposition` is empty).
fn contract(file: &mut SourceFile, text: &str, proposition: &str) -> Spanned<Contract> {
    let start = file.text.find(text).unwrap();
    let where_keyword = Spanned::with_range(&mut file.span_table, (), start..start + 5).span_id;
    let start = start + text.len() - proposition.len();
    let value = if proposition.is_empty() {
        Expr::Missing
    } else {
        Expr::Name(proposition.into())
    };
    let proposition = Spanned::with_range(
        &mut file.span_table,
        value,
        start..start + proposition.len(),
    );
    let end = file.span_table.get(proposition.span_id).end();
    let item = Spanned::with_range(
        &mut file.span_table,
        ClauseItem::Claim(RefinementPredicate::Expression(proposition.clone())),
        start..end,
    );
    node(
        file,
        text,
        Contract {
            where_keyword,
            open: None,
            items: vec![item],
            joiners: Vec::new(),
            close: None,
        },
    )
}

fn example(named: bool, missing: bool) -> SourceFile {
    let text = if missing {
        "f own result T where "
    } else if !named {
        "f own T where header: return local"
    } else {
        "f own result T where header: return local"
    };
    let mut file = SourceFile {
        text: text.into(),
        span_table: Default::default(),
        items: Vec::new(),
        comments: Vec::new(),
    };
    let contract = if missing {
        contract(&mut file, "where ", "")
    } else {
        contract(&mut file, "where header", "header")
    };
    let value = (!missing).then(|| body(&mut file));
    let expression = node(&mut file, "T", Expr::Name("T".into()));
    let binding = Binding {
        attributes: Vec::new(),
        name: node(&mut file, "f", "f".into()),
        parameters: None,
        convention: Some(node(&mut file, "own", BindingConvention::Own)),
        result_binder: named.then(|| node(&mut file, "result", "result".into())),
        annotation: Some(Spanned::new(
            Annotation::Expression(expression.clone()),
            expression.span_id,
        )),
        contract: Some(contract),
        alternatives: Vec::new(),
        extra_results: Vec::new(),
        value,
    };
    let item = node(&mut file, text, Item::Binding(Box::new(binding)));
    file.items.push(item);
    file
}

fn body(file: &mut SourceFile) -> BindingValue {
    let local = node(file, "local", Expr::Name("local".into()));
    let statement = node(
        file,
        "return local",
        Statement::Return(ReturnStatement { value: Some(local) }),
    );
    BindingValue {
        operator: node(file, ":", BindingOperator::Colon),
        expression: node(
            file,
            "return local",
            Expr::Block(Block {
                statements: vec![statement],
            }),
        ),
    }
}

#[test]
fn named_and_anonymous_headers_keep_roles_spans_and_owners() {
    for named in [false, true] {
        let file = example(named, false);
        let hit = file.node_at_offset(6).unwrap();
        assert_eq!(
            matches!(hit.node.kind, SyntaxNodeKind::ResultBinder(_)),
            named
        );
        if named {
            let SyntaxNodeKind::ResultBinder(name) = hit.node.kind else {
                unreachable!()
            };
            assert_eq!(name.value, "result");
            assert_eq!(hit.node.span.to_range(), 6..12);
            assert_eq!(format!("{:?}", hit.node.kind), "ResultBinder");
            assert!(matches!(
                hit.ancestors.last().unwrap().kind,
                SyntaxNodeKind::Binding(_)
            ));
            assert_eq!(hit.declarations.len(), 1);
        }
        assert_eq!(file.declaration_outline().count(), 1);
        let offset = file.text.find("where").unwrap();
        let hit = file.node_at_offset(offset).unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::WhereKeyword));
        assert_eq!(hit.node.span.to_range(), offset..offset + 5);
        let clause = hit.ancestors.last().unwrap();
        assert_eq!(clause.span.extract(&file.text), "where header");
        assert_eq!(
            format!("{:?}", hit.ancestors.iter().rev().nth(1).unwrap().kind),
            "Binding"
        );
        let SyntaxNodeKind::Contract(contract) = clause.kind else {
            unreachable!()
        };
        assert_eq!(contract.value.items.len(), 1);
        assert!(contract.value.joiners.is_empty());
        assert!(matches!(
            contract.value.items[0].value,
            ClauseItem::Claim(RefinementPredicate::Expression(_))
        ));
        let claim = file.node_at_offset(file.text.find("header").unwrap()).unwrap();
        assert_eq!(claim.node.span.extract(&file.text), "header");
        assert!(
            claim
                .ancestors
                .iter()
                .any(|ancestor| matches!(ancestor.kind, SyntaxNodeKind::Contract(_)))
        );
        assert_eq!(hit.declarations.len(), 1);
    }
}

#[test]
fn missing_header_claim_is_not_absent_clause() {
    let mut file = example(true, true);
    let hit = file.node_at_offset(file.text.len()).unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Expression(e)
        if matches!(e.value, Expr::Missing)));
    assert!(hit.node.span.is_empty());
    assert!(matches!(
        hit.ancestors.last().unwrap().kind,
        SyntaxNodeKind::Contract(_)
    ));
    let Item::Binding(binding) = &mut file.items[0].value else {
        unreachable!()
    };
    assert!(binding.contract.take().is_some());
    assert!(binding.contract.is_none());
    assert!(binding.value.is_none());
    assert!(binding.parameters.is_none());
}

#[test]
fn header_children_follow_written_source_order_without_body_binder() {
    let file = example(true, false);
    let mut end = 0;
    for (text, kind) in [
        ("f", "Name(\"f\")"),
        ("own", "BindingConvention"),
        ("result", "ResultBinder"),
        ("T", "Name(\"T\")"),
        ("where header", "WhereKeyword"),
        (":", "BindingOperator"),
        ("return", "Statement"),
    ] {
        let start = file.text.find(text).unwrap();
        let hit = file.node_at_offset(start).unwrap();
        assert_eq!(format!("{:?}", hit.node.kind), kind);
        assert!(end <= hit.node.span.start());
        end = hit.node.span.end();
    }
    let Item::Binding(binding) = &file.items[0].value else {
        unreachable!()
    };
    let Expr::Block(block) = &binding.value.as_ref().unwrap().expression.value else {
        unreachable!()
    };
    assert_eq!(block.statements.len(), 1);
    assert!(matches!(block.statements[0].value, Statement::Return(_)));
}
