use ast::{AttributeItem, DeclareValue, NormalExpr, PredicateExpr};
use parser::query::SourceParseExt;

fn assert_call_directive(name: &str) {
    let source = format!("#{name}(8)\nValue is in 0...255\n");
    let output = source.as_str().parse_source_full();
    assert!(
        output
            .symptoms
            .iter()
            .all(|diagnostic| diagnostic.category != diagnostic::Category::Flaw),
        "{:?}",
        output.symptoms
    );
    let declaration = output
        .ast
        .tags
        .get(&internment::Intern::from_ref("Value"))
        .expect("Value");
    let attributes = declaration
        .attributes
        .raw_attributes
        .as_ref()
        .expect("attributes");
    assert!(matches!(
        attributes.as_slice(),
        [AttributeItem::Call { name: actual, args, .. }]
            if actual.as_str() == name && args.len() == 1
    ));
}

#[test]
fn bits_directive_is_structured() {
    assert_call_directive("bits");
}

#[test]
fn size_directive_is_structured() {
    assert_call_directive("size");
}

#[test]
fn alignment_directive_is_structured() {
    assert_call_directive("alignment");
}

#[test]
fn index_bits_directive_is_structured() {
    assert_call_directive("index_bits");
}

#[test]
fn phantom_directive_is_structured() {
    assert_call_directive("phantom");
}

#[test]
fn inclusive_refinement_is_structured_as_a_range() {
    let output = "Window is in 250...260\n".parse_source_full();
    assert!(output.symptoms.is_empty(), "{:?}", output.symptoms);
    let declaration = output
        .ast
        .tags
        .get(&internment::Intern::from_ref("Window"))
        .expect("Window");
    assert!(
        matches!(declaration.value, DeclareValue::InRange(min, max) if min == 250.into() && max == 260.into())
    );
}

#[test]
fn binding_range_refinement_expands_to_inclusive_bounds() {
    let output = "index is in 0...255: 0\n".parse_source_full();
    assert!(
        output
            .symptoms
            .iter()
            .all(|diagnostic| diagnostic.category != diagnostic::Category::Flaw),
        "{:?}",
        output.symptoms
    );
    let binding = output
        .ast
        .defs
        .get(&internment::Intern::from_ref("index"))
        .expect("index");
    assert_eq!(
        binding.return_refinement,
        Some(PredicateExpr::And(vec![
            PredicateExpr::Ge(NormalExpr::from(0)),
            PredicateExpr::Le(NormalExpr::from(255)),
        ]))
    );
}

#[test]
fn named_binding_refinement_follows_the_nominal_type() {
    let output = "value Int and > 0: 2\n".parse_source_full();
    assert!(output.symptoms.is_empty(), "{:?}", output.symptoms);
    let binding = output
        .ast
        .defs
        .get(&internment::Intern::from_ref("value"))
        .expect("value");
    assert_eq!(
        binding.return_refinement,
        Some(PredicateExpr::Gt(NormalExpr::from(0)))
    );
}

#[test]
fn not_equal_refinement_is_structured_as_inequality() {
    let output = "value is >= 0 and not = 5 and < 100: 2\n".parse_source_full();
    assert!(
        output
            .symptoms
            .iter()
            .all(|diagnostic| diagnostic.category != diagnostic::Category::Flaw),
        "{:?}",
        output.symptoms
    );
    let binding = output
        .ast
        .defs
        .get(&internment::Intern::from_ref("value"))
        .expect("value");
    assert_eq!(
        binding.return_refinement,
        Some(PredicateExpr::And(vec![
            PredicateExpr::Ge(NormalExpr::from(0)),
            PredicateExpr::Ne(NormalExpr::from(5)),
            PredicateExpr::Lt(NormalExpr::from(100)),
        ]))
    );
}

#[test]
fn declaration_level_self_is_an_expression() {
    let output = "Identity is self\n".parse_source_full();
    assert!(output.symptoms.is_empty(), "{:?}", output.symptoms);
    let declaration = output
        .ast
        .tags
        .get(&internment::Intern::from_ref("Identity"))
        .expect("Identity");
    assert!(matches!(
        declaration.value,
        DeclareValue::Alias(ref expression) if matches!(expression.value, ast::Expr::SelfRef)
    ));
}
