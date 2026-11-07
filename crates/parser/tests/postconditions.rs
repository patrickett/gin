//! Header `where` claims only; this does not verify or export their truth.
//! A claim is never written on a `return`.

use ast::{
    BinaryOperator, ClauseItem, Expr, Item, RefinementPredicate, SourceFile, SpanId, Statement,
    SyntaxNodeKind,
};

fn claims(file: &SourceFile, item: usize) -> Vec<&ast::Expression> {
    let Item::Binding(binding) = &file.items[item].value else {
        panic!("source binding");
    };
    binding
        .contract
        .as_ref()
        .expect("written where clause")
        .value
        .items
        .iter()
        .filter_map(|item| match &item.value {
            ClauseItem::Claim(RefinementPredicate::Expression(claim)) => Some(claim),
            _ => None,
        })
        .collect()
}

fn text(file: &SourceFile, span: SpanId) -> &str {
    file.span_table.get(span).extract(&file.text)
}

#[test]
fn existing_preconditions_and_header_claims_remain_separate() {
    let source = concat!(
        "identity(value Int and value >= 0) Int where value >= 0: return value\n",
        "plain: return\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let claim = claims(&file, 0);
    assert_eq!(claim.len(), 1);
    assert_eq!(text(&file, claim[0].span_id), "value >= 0");
    let Item::Binding(plain) = &file.items[1].value else {
        panic!("plain");
    };
    assert!(plain.contract.is_none());
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("identity");
    };
    assert!(
        binding.parameters.as_ref().unwrap().value[0]
            .value
            .annotation
            .is_some()
    );
    let Expr::Block(block) = &binding.value.as_ref().unwrap().expression.value else {
        panic!("body");
    };
    assert!(matches!(block.statements[0].value, Statement::Return(_)));
}

#[test]
fn claims_preserve_grouping_operators_and_names_without_binding_a_result() {
    let source = "f(a Int, b Int) Int where (a < b or a = b) and a >= 0: return a\n";
    let file = parser::parse_strict(source).unwrap();
    let claim = claims(&file, 0)[0];
    let Expr::Binary {
        left,
        operator,
        right,
    } = &claim.value
    else {
        panic!("written conjunction");
    };
    assert_eq!(operator.value, BinaryOperator::And);
    assert_eq!(text(&file, operator.span_id), "and");
    let Expr::Group(group) = &left.value else {
        panic!("parentheses retained");
    };
    assert!(matches!(&group.value, Expr::Binary { operator, .. }
        if operator.value == BinaryOperator::Or));
    assert_eq!(text(&file, right.span_id), "a >= 0");
    assert_eq!(text(&file, claim.span_id), "(a < b or a = b) and a >= 0");
}

#[test]
fn return_claims_are_rejected_and_later_items_still_parse() {
    for source in [
        "main: return thus value >= 0\nlater := 2\n",
        "main: return value thus value >= 0\nlater := 2\n",
        "main:\n    return value thus value = value\nlater := 2\n",
    ] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert!(parser::parse_strict(source).is_err(), "{source}");
        let hit = output
            .file
            .node_at_offset(source.find("later").unwrap())
            .unwrap();
        assert!(
            matches!(hit.node.kind, SyntaxNodeKind::Name("later")),
            "{source}"
        );
    }
}

#[test]
fn claim_keyword_and_expressions_keep_written_order_and_owners() {
    let source = concat!(
        "--- hé 🦀\nouter(value Int) Int where value >= 0:\n",
        "    inner(other Int) Int where other = other: return other\n",
        "    return inner(value)\n",
    );
    let file = parser::parse_strict(source).unwrap();
    for (offset, expected) in source
        .match_indices("where")
        .zip([vec!["outer"], vec!["outer", "inner"]])
    {
        let hit = file.node_at_offset(offset.0).unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::WhereKeyword));
        assert_eq!(format!("{:?}", hit.node.kind), "WhereKeyword");
        assert!(
            hit.ancestors
                .iter()
                .any(|node| matches!(node.kind, SyntaxNodeKind::Contract(_)))
        );
        let owners: Vec<_> = hit.declarations.iter().map(|owner| owner.name).collect();
        assert_eq!(owners, expected);
    }
}

#[test]
fn missing_claims_recover_without_dropping_where_or_absorbing_a_later_item() {
    for source in ["main where", "main Int where", "main Int where value >"] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors(), "{source}");
        let Item::Binding(binding) = &output.file.items[0].value else {
            panic!("binding");
        };
        assert!(binding.contract.is_some(), "{source}");
        let hit = output.file.node_at_offset(source.len()).unwrap();
        assert!(
            matches!(hit.node.kind, SyntaxNodeKind::Expression(expression)
            if expression.value == Expr::Missing),
            "{source}"
        );
        assert_eq!(hit.node.span.start(), source.len());
        assert_eq!(hit.node.span.end(), source.len());
    }
    let source = "main Int where\nlater := 2\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    let Item::Binding(binding) = &output.file.items[0].value else {
        panic!("binding");
    };
    assert!(binding.contract.is_some());
    let hit = output
        .file
        .node_at_offset(source.find("later").unwrap())
        .unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("later")));
}

#[test]
fn unsupported_proof_and_outcome_forms_fail_closed_in_this_slice() {
    for source in [
        "f(value Int) Int where value in 0 to 4 or value > 9: return value",
        "f(value Int) True where value > 0 or False where value <= 0: return True",
        "f(value Int) Int where value > 0 where value < 4: return value",
    ] {
        let source = format!("{source}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert!(parser::parse_strict(&source).is_err(), "{source}");
        let hit = output
            .file
            .node_at_offset(source.find("later").unwrap())
            .unwrap();
        assert!(
            matches!(hit.node.kind, SyntaxNodeKind::Name("later")),
            "{source}"
        );
    }
}

#[test]
fn claim_expressions_obey_depth_and_diagnostic_budgets() {
    use parser::{ParseFailure, ParseLimit, ParseLimits, ParseOptions};
    for (source, limits, expected) in [
        (
            "main Int where - - -value > 0: return value",
            ParseLimits {
                expression_depth_max: 2,
                ..Default::default()
            },
            ParseLimit::ExpressionDepth,
        ),
        (
            "main Int where\nfirst Int where\n",
            ParseLimits {
                diagnostics_max: 1,
                ..Default::default()
            },
            ParseLimit::Diagnostics,
        ),
    ] {
        let options = ParseOptions {
            limits,
            ..Default::default()
        };
        let ParseFailure::LimitExceeded { limit, span, .. } =
            parser::parse_with_options(source, options).unwrap_err()
        else {
            panic!("budget refusal");
        };
        assert_eq!(limit, expected);
        assert!(source.get(span.to_range()).is_some());
    }
}

#[test]
fn cancellation_remains_live_in_a_long_claim() {
    use parser::{ParseFailure, ParseOptions};
    use std::cell::Cell;

    let source = format!(
        "main Int where check({}value): return value\n",
        "value,".repeat(2_000)
    );
    let baseline = Cell::new(0usize);
    let count = || {
        baseline.set(baseline.get() + 1);
        false
    };
    let output = parser::parse_with_options(
        &source,
        ParseOptions {
            is_cancelled: Some(&count),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!output.has_errors());
    assert!(baseline.get() > 100);
    let cutoff = baseline.get() - 100;
    let visits = Cell::new(0usize);
    let cancelled = || {
        visits.set(visits.get() + 1);
        visits.get() >= cutoff
    };
    let failure = parser::parse_with_options(
        &source,
        ParseOptions {
            is_cancelled: Some(&cancelled),
            ..Default::default()
        },
    )
    .unwrap_err();
    let ParseFailure::Cancelled { span } = failure else {
        panic!("expected cancellation, not a syntax diagnostic");
    };
    assert!(source.get(span.to_range()).is_some());
    assert!(span.start() > source.len() / 2);
}
