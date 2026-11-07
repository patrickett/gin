//! Executable source-consumer examples, not a checker adapter or verified contracts.
//! Every name remains unresolved; implicit subjects and literal values are not inferred.

use ast::{
    Annotation, Argument, BinaryOperator, Convention, Expr, Expression, Item, LiteralKind,
    RefinementPredicate, SourceFile, TypeBody, TypeMember,
};

const SOURCE: &str = concat!(
    "--- hé 🦀\nState(element) has\n",
    "    capacity Count(element)\n",
    "    initialized Count(element) and <= capacity\n",
    "    inspect(self ref and self.initialized < self.capacity) Count(element):\n",
    "        return self.initialized\n",
    "probe(r mut Region(Item) and r.initialized < r.capacity) Int and >= 0: return 0\n",
);

fn member(expression: &Expression, base_name: &str, member_name: &str) {
    let Expr::Member { base, name } = &expression.value else {
        panic!("written member access");
    };
    assert_eq!(base.value, Expr::Name(base_name.into()));
    assert_eq!(name.value, member_name);
}

fn explicit_comparison(predicate: &RefinementPredicate, subject: &str) {
    let RefinementPredicate::Expression(expression) = predicate else {
        panic!("explicit written subject");
    };
    let Expr::Binary {
        left,
        operator,
        right,
    } = &expression.value
    else {
        panic!("written comparison");
    };
    assert_eq!(operator.value, BinaryOperator::Less);
    member(left, subject, "initialized");
    member(right, subject, "capacity");
}

fn keyword(file: &SourceFile, span: ast::SpanId, spelling: &str) {
    assert_eq!(file.span_table.get(span).extract(&file.text), spelling);
}

#[test]
fn parameter_and_result_conditions_are_consumable_without_reparsing_text() {
    let file = parser::parse_strict(SOURCE).unwrap();
    let Item::Binding(binding) = &file.items[1].value else {
        panic!("ordinary probe binding");
    };
    assert!(binding.convention.is_none());
    let parameter = &binding.parameters.as_ref().unwrap().value[0].value;
    let convention = &parameter.convention.as_ref().unwrap().value;
    assert_eq!(convention.keyword.value, Convention::Mut);
    keyword(&file, convention.keyword.span_id, "mut");
    let Annotation::Refined {
        base: Some(base),
        refinements,
    } = &parameter.annotation.as_ref().unwrap().value
    else {
        panic!("written parameter base and conditions");
    };
    let Expr::Call { callee, arguments } = &base.value else {
        panic!("ordinary type application");
    };
    assert_eq!(callee.value, Expr::Name("Region".into()));
    assert_eq!(arguments.value.len(), 1);
    let Argument::Positional(argument) = &arguments.value[0].value else {
        panic!("written generic argument");
    };
    assert_eq!(argument.value, Expr::Name("Item".into()));
    assert_eq!(refinements.len(), 1);
    keyword(&file, refinements[0].value.and_keyword, "and");
    explicit_comparison(&refinements[0].value.predicate.value, "r");
    let Annotation::Refined {
        base: Some(base),
        refinements,
    } = &binding.annotation.as_ref().unwrap().value
    else {
        panic!("written result annotation");
    };
    assert_eq!(base.value, Expr::Name("Int".into()));
    let RefinementPredicate::ImplicitComparison { operator, right } =
        &refinements[0].value.predicate.value
    else {
        panic!("result subject remains implicit");
    };
    assert_eq!(operator.value, BinaryOperator::GreaterOrEqual);
    assert_eq!(right.value, Expr::Literal(LiteralKind::Integer));
    keyword(&file, operator.span_id, ">=");
    keyword(&file, right.span_id, "0"); // Literal spelling, not predicate reconstruction.
}

#[test]
fn field_and_receiver_subject_forms_remain_distinct() {
    let file = parser::parse_strict(SOURCE).unwrap();
    let Item::Type(declaration) = &file.items[0].value else {
        panic!("source type occurrence");
    };
    let TypeBody::Has(members) = &declaration.body.value else {
        panic!("written members");
    };
    let TypeMember::Binding(field) = &members[1].value else {
        panic!("initialized field");
    };
    assert_eq!(field.name.value, "initialized");
    let Annotation::Refined { refinements, .. } = &field.annotation.as_ref().unwrap().value else {
        panic!("field refinement");
    };
    let RefinementPredicate::ImplicitComparison { operator, right } =
        &refinements[0].value.predicate.value
    else {
        panic!("no synthesized field subject");
    };
    assert_eq!(operator.value, BinaryOperator::LessOrEqual);
    assert_eq!(right.value, Expr::Name("capacity".into()));
    keyword(&file, operator.span_id, "<=");
    let TypeMember::Binding(method) = &members[2].value else {
        panic!("inspect method");
    };
    let receiver = &method.parameters.as_ref().unwrap().value[0].value;
    assert_eq!(receiver.name.value, "self");
    assert_eq!(
        receiver.convention.as_ref().unwrap().value.keyword.value,
        Convention::Ref
    );
    let Annotation::Refined {
        base: None,
        refinements,
    } = &receiver.annotation.as_ref().unwrap().value
    else {
        panic!("bare-and receiver has no fabricated type");
    };
    explicit_comparison(&refinements[0].value.predicate.value, "self");
    let offset = file
        .span_table
        .get(refinements[0].value.and_keyword)
        .start();
    let hit = file.node_at_offset(offset).unwrap();
    let owners: Vec<_> = hit.declarations.iter().map(|owner| owner.name).collect();
    assert_eq!(owners, ["State", "inspect"]);
}
