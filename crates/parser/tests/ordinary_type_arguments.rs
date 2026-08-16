use ast::{DeclareValue, Expr, HasMember};
use parser::query::SourceParseExt;

#[test]
fn binary_expression_is_preserved_in_type_application_argument() {
    let output = "Array(x Type, n Int) has size Int\nHolder has values Array(Int, n + 1)\n"
        .parse_source_full();
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
