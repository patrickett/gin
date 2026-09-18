use ast::{BindValue, Expr};
use internment::Intern;
use parser::query::SourceParseExt;

#[test]
fn empty_parentheses_are_an_empty_result_value() {
    let parsed = "value := ()\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let BindValue::Expr(value) = &parsed.ast.defs[&Intern::from_ref("value")].value else {
        panic!("expression")
    };
    assert!(matches!(&value.value, Expr::ResultValues(values) if values.is_empty()));
}
