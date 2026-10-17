use ast::{DeclareValue, Expr, HasMember};
use parser::query::SourceParseExt;

#[test]
fn binary_expression_is_preserved_in_type_application_argument() {
    let output =
        "Array(x, n Int) has size Int\nHolder has values Array(Int, n + 1)\n".parse_source_full();
    assert!(
        output.symptoms.is_empty(),
        "parse symptoms: {:?}",
        output.symptoms
    );

    let holder = output
        .ast
        .tags
        .get(&internment::Intern::from_ref("Holder"))
        .expect("Holder declaration");
    let DeclareValue::Has(members) = &holder.value else {
        panic!("Holder should be a record declaration");
    };
    let HasMember::Property(property) = &members[0] else {
        panic!("Holder should have a property");
    };
    let Expr::TagCall(call) = &property.ty.as_ref().expect("property type").value else {
        panic!("property should use a generic type application");
    };
    assert!(matches!(
        call.args.get(1).map(|arg| &arg.value),
        Some(Expr::Binary { .. })
    ));
}

#[test]
fn named_return_type_arguments_preserve_their_individual_values() {
    let output = "forward(value Container(element: Byte, allocator: Arena)) Container(allocator: Arena, element: Byte): return value\n".parse_source_full();
    assert!(output.symptoms.is_empty(), "{:?}", output.symptoms);
    let bind = output
        .ast
        .defs
        .values()
        .find(|bind| bind.name.as_str() == "forward")
        .unwrap();
    let (_, arguments) = bind.type_annotation.as_ref().expect("type application");
    assert_eq!(arguments.len(), 2);
    for (argument, expected_name, expected_value) in [
        (&arguments[0], "allocator", "Arena"),
        (&arguments[1], "element", "Byte"),
    ] {
        let Expr::Bind(bind) = &argument.value else {
            panic!("named argument");
        };
        assert_eq!(bind.name.as_str(), expected_name);
        let ast::BindValue::Expr(value) = &bind.value else {
            panic!("argument value");
        };
        assert!(
            matches!(&value.value, Expr::AnonymousTag(name) if name.as_str() == expected_value)
        );
    }
}
