use ast::{BindValue, Expr, NormalExpr, PredicateExpr, TargetQueryKind, TypeReference};
use internment::Intern;
use parser::query::SourceParseExt;

fn parsed_binding(source: &str, name: &str) -> ast::Bind {
    let output = source.parse_source_full();
    assert!(output.symptoms.is_empty(), "{:?}", output.symptoms);
    output
        .ast
        .defs
        .get(&Intern::from_ref(name))
        .expect("binding")
        .clone()
}

#[test]
fn index_bits_query_preserves_pointer_to_unit_operand() {
    let binding = parsed_binding("width := #index_bits(@())\n", "width");
    let BindValue::Expr(expression) = binding.value else {
        panic!("expression body");
    };
    assert_eq!(
        expression.value,
        Expr::TargetQuery {
            kind: TargetQueryKind::IndexBits,
            operand: TypeReference::Pointer(Box::new(TypeReference::Unit)),
        }
    );
}

#[test]
fn target_query_predicate_is_not_encoded_as_a_variable() {
    let binding = parsed_binding(
        "layout(element, alignment Alignment and >= #alignment(element)): alignment\n",
        "layout",
    );
    assert_eq!(
        binding.param_refinements[&Intern::from_ref("alignment")],
        PredicateExpr::Ge(NormalExpr::TargetQuery {
            kind: TargetQueryKind::Alignment,
            operand: TypeReference::Nominal(Intern::from_ref("element")),
        })
    );
}

#[test]
fn applied_nominal_cast_preserves_type_arguments() {
    let binding = parsed_binding(
        "convert(value, element): value as Count(element)\n",
        "convert",
    );
    let BindValue::Expr(expression) = binding.value else {
        panic!("expression body");
    };
    assert!(matches!(
        expression.value,
        Expr::Cast {
            ty: TypeReference::Application { name, arguments },
            ..
        } if name.as_str() == "Count"
            && arguments == vec![TypeReference::Nominal(Intern::from_ref("element"))]
    ));
}
