use ast::{DeclareValue, Expr, HasMember};
use internment::Intern;
use parser::query::SourceParseExt;

#[test]
fn method_results_can_project_a_named_integer_domain() {
    for body in ["", ": self"] {
        let parsed = format!("Numeric has\n    as_num(self ref) is in Self{body}\n    to_num(self) is in Self{body}\n").parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let declaration = &parsed.ast.tags[&Intern::from_ref("Numeric")];
        let DeclareValue::Has(members) = &declaration.value else {
            panic!("expected a has declaration");
        };
        for member in members {
            let HasMember::Function(method) = member else {
                panic!("expected a method");
            };
            assert!(matches!(
                method.return_ty.as_ref().map(|ty| &ty.value),
                Some(Expr::IntegerDomain(_))
            ));
        }
    }
}
