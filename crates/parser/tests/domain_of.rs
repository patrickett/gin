use ast::{BindValue, Expr};
use internment::Intern;
use parser::query::SourceParseExt;

#[test]
fn domain_of_binds_more_tightly_than_comparison() {
    let parsed = "Word is in -8 to 7\nequal: domain of Word = domain of Word\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let bind = &parsed.ast.defs[&Intern::from_ref("equal")];
    let BindValue::Expr(expr) = &bind.value else {
        panic!("body")
    };
    let Expr::Binary(binary) = &expr.value else {
        panic!("binary: {expr:?}")
    };
    assert!(matches!(binary.lhs.value, Expr::DomainOf(_)));
    assert!(matches!(binary.rhs.value, Expr::DomainOf(_)));
}

#[test]
fn range_parameters_bind_two_distinct_names_in_one_argument() {
    let parsed = "Bounds(low to high) is low to high\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let params = parsed.ast.tags[&Intern::from_ref("Bounds")]
        .params
        .as_ref()
        .unwrap();
    assert_eq!(params.len(), 1);
    let pattern = params
        .values()
        .next()
        .unwrap()
        .range_pattern
        .as_ref()
        .unwrap();
    assert_eq!(pattern.start.as_str(), "low");
    assert_eq!(pattern.end.as_str(), "high");
}

#[test]
fn repeated_endpoint_bindings_are_rejected() {
    let parsed = "Bounds(x to x) is x to x\n".parse_source_full();
    assert!(
        parsed
            .symptoms
            .iter()
            .any(|d| d.code.slug() == "parse-range-parameter")
    );
}
