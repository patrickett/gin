//! Single-result source guarantees; no resolution, proof truth, or export inference.

use ast::{
    Annotation, Binding, ClauseItem, Expr, Item, RefinementPredicate, SourceFile, Statement,
    SyntaxNodeKind, TypeBody, TypeMember,
};

fn binding(file: &SourceFile, index: usize) -> &Binding {
    let Item::Binding(binding) = &file.items[index].value else {
        panic!("source binding");
    };
    binding
}

#[test]
fn named_header_guarantee_is_a_where_claim_and_return_carries_none() {
    let source = concat!(
        "make(value Int and value >= 0) result Int where result >= value:\n",
        "    return value\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let function = binding(&file, 0);
    assert_eq!(function.result_binder.as_ref().unwrap().value, "result");
    let Annotation::Expression(annotation) = &function.annotation.as_ref().unwrap().value else {
        panic!("written result type");
    };
    assert_eq!(annotation.value, Expr::Name("Int".into()));
    let header = function.contract.as_ref().unwrap();
    assert_eq!(
        file.span_table.get(header.span_id).extract(source),
        "where result >= value"
    );
    assert_eq!(
        file.span_table
            .get(header.value.where_keyword)
            .extract(source),
        "where"
    );
    assert_eq!(header.value.items.len(), 1);
    assert!(header.value.joiners.is_empty());
    let ClauseItem::Claim(RefinementPredicate::Expression(claim)) = &header.value.items[0].value
    else {
        panic!("written claim");
    };
    assert_eq!(
        file.span_table.get(claim.span_id).extract(source),
        "result >= value"
    );
    let Expr::Block(body) = &function.value.as_ref().unwrap().expression.value else {
        panic!("body");
    };
    assert_eq!(body.statements.len(), 1);
    let Statement::Return(returned) = &body.statements[0].value else {
        panic!("return");
    };
    assert_eq!(
        returned.value.as_ref().unwrap().value,
        Expr::Name("value".into())
    );
}

#[test]
fn claims_and_transitions_share_one_where_clause_in_written_order() {
    let source = "f(x Int) result Int where result >= 0, x :: x + 1, x >= 0: return x\n";
    let file = parser::parse_strict(source).unwrap();
    let contract = binding(&file, 0).contract.as_ref().unwrap();
    assert_eq!(contract.value.items.len(), 3);
    assert_eq!(contract.value.joiners.len(), 2);
    for joiner in &contract.value.joiners {
        assert_eq!(file.span_table.get(*joiner).extract(source), ",");
    }
    assert!(matches!(
        contract.value.items[0].value,
        ClauseItem::Claim(RefinementPredicate::Expression(_))
    ));
    assert!(matches!(
        contract.value.items[1].value,
        ClauseItem::Transition(_)
    ));
    assert!(matches!(
        contract.value.items[2].value,
        ClauseItem::Claim(RefinementPredicate::Expression(_))
    ));
    assert_eq!(
        file.span_table
            .get(contract.value.items[1].span_id)
            .extract(source),
        "x :: x + 1"
    );
}

#[test]
fn unnamed_result_may_open_a_claim_with_a_comparison() {
    let source = "f(x Int) Int where >= 10: return x\n";
    let file = parser::parse_strict(source).unwrap();
    let contract = binding(&file, 0).contract.as_ref().unwrap();
    let ClauseItem::Claim(RefinementPredicate::ImplicitComparison { operator, right }) =
        &contract.value.items[0].value
    else {
        panic!("implicit comparison");
    };
    assert_eq!(file.span_table.get(operator.span_id).extract(source), ">=");
    assert_eq!(file.span_table.get(right.span_id).extract(source), "10");
}

#[test]
fn return_postcondition_is_a_diagnostic_and_later_items_still_parse() {
    for source in ["f(x Int) Int: return x thus x >= 0\nlater := 2\n"] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors());
        assert!(parser::parse_strict(source).is_err());
        let hit = output
            .file
            .node_at_offset(source.find("later").unwrap())
            .unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("later")));
    }
}

#[test]
fn anonymous_and_generic_results_keep_written_names_without_label_classification() {
    let source = concat!(
        "plain(value Int) Int where value >= 0: return value\n",
        "label(value Int) True where value > 0: return True\n",
        "generic(element Type, value element) result element where result = value: return value\n",
        "collision(result Int) result Int where result = result: return result\n",
    );
    let file = parser::parse_strict(source).unwrap();
    for index in [0, 1] {
        assert!(binding(&file, index).result_binder.is_none());
        assert!(binding(&file, index).contract.is_some());
    }
    let Annotation::Expression(label) = &binding(&file, 1).annotation.as_ref().unwrap().value
    else {
        panic!("unresolved result occurrence");
    };
    assert_eq!(label.value, Expr::Name("True".into()));
    let generic = binding(&file, 2);
    assert_eq!(generic.result_binder.as_ref().unwrap().value, "result");
    let Annotation::Expression(annotation) = &generic.annotation.as_ref().unwrap().value else {
        panic!("generic annotation");
    };
    assert_eq!(annotation.value, Expr::Name("element".into()));
    let collision = binding(&file, 3);
    assert_eq!(collision.result_binder.as_ref().unwrap().value, "result");
    assert_eq!(
        collision.parameters.as_ref().unwrap().value[0]
            .value
            .name
            .value,
        "result"
    );
}

#[test]
fn result_binder_and_header_keyword_queries_do_not_create_body_declarations() {
    let source = "--- hé 🦀\nf(value Int) result Int where result = value: return value\n";
    let file = parser::parse_strict(source).unwrap();
    let function = binding(&file, 0);
    let name = function.result_binder.as_ref().unwrap();
    let span = file.span_table.get(name.span_id);
    let hit = file.node_at_offset(span.start()).unwrap();
    assert!(
        matches!(hit.node.kind, SyntaxNodeKind::ResultBinder(found) if found.value == "result")
    );
    assert_eq!(hit.node.span, span);
    assert_eq!(format!("{:?}", hit.node.kind), "ResultBinder");
    assert_eq!(hit.declarations.last().unwrap().name, "f");
    let reference = file
        .node_at_offset(source.rfind("result").unwrap())
        .unwrap();
    assert!(matches!(
        reference.node.kind,
        SyntaxNodeKind::Name("result")
    ));
    let clause = function.contract.as_ref().unwrap();
    let hit = file
        .node_at_offset(file.span_table.get(clause.value.where_keyword).start())
        .unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::WhereKeyword));
    assert!(
        hit.ancestors
            .iter()
            .any(|node| matches!(node.kind, SyntaxNodeKind::Contract(_)))
    );
    assert_eq!(
        file.declaration_outline()
            .map(|declaration| declaration.name)
            .collect::<Vec<_>>(),
        ["f"]
    );
}

#[test]
fn missing_header_claims_and_method_separators_preserve_their_owners() {
    for source in ["f() result Int where", "f() result Int where result >"] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors());
        let function = binding(&output.file, 0);
        assert_eq!(function.result_binder.as_ref().unwrap().value, "result");
        assert!(function.contract.is_some());
        let hit = output.file.node_at_offset(source.len()).unwrap();
        assert!(
            matches!(hit.node.kind, SyntaxNodeKind::Expression(expression)
            if expression.value == Expr::Missing)
        );
    }
    let source = "Thing has\n    first() result Int where result > 0\n    next Int\nlater := 2\n";
    let file = parser::parse_strict(source).unwrap();
    let Item::Type(thing) = &file.items[0].value else {
        panic!("type");
    };
    let TypeBody::Has(members) = &thing.body.value else {
        panic!("members");
    };
    assert_eq!(members.len(), 2);
    let TypeMember::Binding(method) = &members[0].value else {
        panic!("method");
    };
    assert!(method.result_binder.is_some() && method.contract.is_some());
    let TypeMember::Binding(field) = &members[1].value else {
        panic!("field");
    };
    assert!(field.result_binder.is_none() && field.contract.is_none());
}

#[test]
fn logical_or_requires_grouping_and_outcome_rows_are_not_flattened() {
    let source = "f(x Int) result Int where (result = x or result > x) and x >= 0: return x\n";
    assert!(parser::parse_strict(source).is_ok());
    for header in [
        "f(x Int) result Int where result = x or Error",
        "f(x Int) True where x > 0 or False where x <= 0",
        "f(x Int) first Int, second Int",
        "f(x Int) result Int where not result < x",
        "Thing has field Int where field >= 0",
    ] {
        let source = format!("{header}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert!(parser::parse_strict(&source).is_err(), "{source}");
        let hit = output
            .file
            .node_at_offset(source.find("later").unwrap())
            .unwrap();
        assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("later")));
    }
}

#[test]
fn unannotated_header_guarantees_work_at_top_level_and_in_nested_bodies() {
    let source = concat!(
        "plain() where 1 = 1: return 1\n",
        "outer():\n    inner() where 2 = 2: return 2\n",
        "Thing has method() where 3 = 3: return 3\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let Expr::Block(body) = &binding(&file, 1).value.as_ref().unwrap().expression.value else {
        panic!("outer body");
    };
    let Statement::Binding(inner) = &body.statements[0].value else {
        panic!("nested binding");
    };
    let Item::Type(thing) = &file.items[2].value else {
        panic!("type");
    };
    let TypeBody::Has(members) = &thing.body.value else {
        panic!("members");
    };
    let TypeMember::Binding(method) = &members[0].value else {
        panic!("method");
    };
    for function in [binding(&file, 0), inner.as_ref(), method.as_ref()] {
        assert!(function.parameters.as_ref().unwrap().value.is_empty());
        assert!(function.result_binder.is_none());
        assert!(function.annotation.is_none());
        assert!(function.contract.is_some());
    }
}

#[test]
fn explicit_result_end_is_a_flaw_not_a_named_result_binder() {
    for source in ["make() end Transaction", "Thing has make() end Transaction"] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors());
        let diagnostic = output
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message.contains("not a result"))
            .unwrap();
        assert_eq!(diagnostic.span.extract(source), "end");
        assert_eq!(diagnostic.category, diagnostic::Category::Flaw);
        assert!(parser::parse_strict(source).is_err());
    }
    // Contextual rejection does not reserve the identifier throughout the source.
    assert!(parser::parse_strict("end(value Int) Int: return value\n").is_ok());
}
