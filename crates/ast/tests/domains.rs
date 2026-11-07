use ast::*;

fn located<T>(spans: &mut SpanTable, value: T, source: &str, text: &str) -> Spanned<T> {
    let start = source.find(text).unwrap();
    Spanned::with_range(spans, value, start..start + text.len())
}

fn range_file(with_in: bool) -> SourceFile {
    let source = if with_in {
        "T is in -1 to 9"
    } else {
        "T is -1 to 9"
    };
    let mut spans = SpanTable::new();
    let operator = located(&mut spans, UnaryOperator::Negate, source, "-");
    let operand = located(&mut spans, Expr::Literal(LiteralKind::Integer), source, "1");
    let lower = located(
        &mut spans,
        Expr::Unary {
            operator,
            operand: Box::new(operand),
        },
        source,
        "-1",
    );
    let upper = located(&mut spans, Expr::Literal(LiteralKind::Integer), source, "9");
    let to_keyword = located(&mut spans, (), source, "to").span_id;
    let base = located(
        &mut spans,
        DomainBase::Range {
            lower,
            to_keyword,
            upper,
        },
        source,
        "-1 to 9",
    );
    let in_keyword = with_in.then(|| located(&mut spans, (), source, "in").span_id);
    let start = if with_in { "in -1 to 9" } else { "-1 to 9" };
    let domain = located(
        &mut spans,
        TypeDomain {
            in_keyword,
            base,
            refinements: vec![],
        },
        source,
        start,
    );
    wrap(source, spans, domain)
}

fn wrap(source: &str, mut spans: SpanTable, domain: Spanned<TypeDomain>) -> SourceFile {
    let name = located(&mut spans, "T".into(), source, "T");
    let body = Spanned::with_range(&mut spans, TypeBody::Domain(domain), 2..source.len());
    let item = Spanned::with_range(
        &mut spans,
        Item::Type(Box::new(TypeDeclaration {
            name,
            parameters: None,
            body,
        })),
        0..source.len(),
    );
    SourceFile {
        text: source.into(),
        span_table: spans,
        items: vec![item],
        comments: vec![],
    }
}

#[test]
fn range_keywords_and_unary_preserve_spans_and_owners() {
    for with_in in [false, true] {
        let file = range_file(with_in);
        let source = file.text.as_ref();
        let to = file.node_at_offset(source.find("to").unwrap()).unwrap();
        assert!(matches!(
            to.node.kind,
            SyntaxNodeKind::DomainKeyword(DomainKeyword::To)
        ));
        assert_eq!(to.node.span.extract(source), "to");
        assert_eq!(to.declarations.len(), 1);
        assert_eq!(to.declarations[0].name, "T");
        assert!(to.ancestors.iter().any(|node| matches!(node.kind,
            SyntaxNodeKind::TypeDomain(domain) if domain.in_keyword.is_some() == with_in)));
        assert!(
            to.ancestors
                .iter()
                .any(|node| matches!(node.kind, SyntaxNodeKind::DomainBase(_)))
        );
        let sign = file.node_at_offset(source.find('-').unwrap()).unwrap();
        assert!(
            matches!(sign.node.kind, SyntaxNodeKind::UnaryOperator(operator)
            if operator.value == UnaryOperator::Negate)
        );
        assert_eq!(sign.node.span.extract(source), "-");
        if with_in {
            let keyword = file.node_at_offset(source.find("in").unwrap()).unwrap();
            assert!(matches!(
                keyword.node.kind,
                SyntaxNodeKind::DomainKeyword(DomainKeyword::In)
            ));
            assert_eq!(keyword.node.span.extract(source), "in");
        }
        assert_eq!(format!("{:?}", to.node.kind), "DomainKeyword");
    }
}

fn refined_file() -> SourceFile {
    let source = "T is in Int and < limit and self > floor";
    let mut spans = SpanTable::new();
    let expression = located(&mut spans, Expr::Name("Int".into()), source, "Int");
    let base = located(
        &mut spans,
        DomainBase::Expression(expression),
        source,
        "Int",
    );
    let operator = located(&mut spans, BinaryOperator::Less, source, "<");
    let right = located(&mut spans, Expr::Name("limit".into()), source, "limit");
    let implicit = located(
        &mut spans,
        RefinementPredicate::ImplicitComparison { operator, right },
        source,
        "< limit",
    );
    let and_keyword = located(&mut spans, (), source, "and").span_id;
    let first = located(
        &mut spans,
        Refinement {
            and_keyword,
            predicate: implicit,
        },
        source,
        "and < limit",
    );
    let second = explicit_refinement(&mut spans, source);
    let in_keyword = Some(located(&mut spans, (), source, "in").span_id);
    let domain = located(
        &mut spans,
        TypeDomain {
            in_keyword,
            base,
            refinements: vec![first, second],
        },
        source,
        "in Int and < limit and self > floor",
    );
    wrap(source, spans, domain)
}

fn explicit_refinement(spans: &mut SpanTable, source: &str) -> Spanned<Refinement> {
    let left = located(spans, Expr::Name("self".into()), source, "self");
    let operator = located(spans, BinaryOperator::Greater, source, ">");
    let right = located(spans, Expr::Name("floor".into()), source, "floor");
    let explicit = located(
        spans,
        Expr::Binary {
            left: Box::new(left),
            operator,
            right: Box::new(right),
        },
        source,
        "self > floor",
    );
    let predicate = located(
        spans,
        RefinementPredicate::Expression(explicit),
        source,
        "self > floor",
    );
    let second_start = source.rfind("and").unwrap();
    let and_keyword = spans.insert_range(second_start..second_start + 3);
    located(
        spans,
        Refinement {
            and_keyword,
            predicate,
        },
        source,
        "and self > floor",
    )
}

#[test]
fn ordered_refinements_keep_implicit_and_explicit_subjects_distinct() {
    let file = refined_file();
    let source = file.text.as_ref();
    for text in ["<", "self", ">"] {
        let hit = file.node_at_offset(source.find(text).unwrap()).unwrap();
        assert_eq!(hit.node.span.extract(source), text);
        assert_eq!(hit.declarations.len(), 1);
        let predicate = hit
            .ancestors
            .iter()
            .find_map(|node| match node.kind {
                SyntaxNodeKind::RefinementPredicate(predicate) => Some(predicate),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            matches!(
                predicate.value,
                RefinementPredicate::ImplicitComparison { .. }
            ),
            text == "<"
        );
        let domain = hit
            .ancestors
            .iter()
            .find_map(|node| match node.kind {
                SyntaxNodeKind::TypeDomain(domain) => Some(domain),
                _ => None,
            })
            .unwrap();
        let texts: Vec<_> = domain
            .refinements
            .iter()
            .map(|clause| clause.extract_source(&file.span_table, source))
            .collect();
        assert_eq!(texts, ["and < limit", "and self > floor"]);
        assert_eq!(
            format!("{:?}", SyntaxNodeKind::TypeDomain(domain)),
            "TypeDomain"
        );
    }
    for (offset, _) in source.match_indices("and") {
        let hit = file.node_at_offset(offset).unwrap();
        assert!(matches!(
            hit.node.kind,
            SyntaxNodeKind::DomainKeyword(DomainKeyword::And)
        ));
        assert_eq!(hit.node.span.to_range(), offset..offset + 3);
        assert!(
            hit.ancestors
                .iter()
                .any(|node| matches!(node.kind, SyntaxNodeKind::Refinement(_)))
        );
    }
}
