//! `#register` and `#instruction`: the only link between Gin and the machine.
use ast::{FieldAttribute, Item, TypeBody, TypeMember};

const SOURCE: &str = "\
Inputs has
    #register('x0')
    character in 0 to 255

#instruction('bl')
branch_link(target, input Type, output Type, inputs input) output

put(inputs Inputs) Inputs: return branch_link('putchar', Inputs, Inputs, inputs)
";

fn spelled(file: &ast::SourceFile, span: ast::SpanId) -> &str {
    file.span_table.get(span).extract(&file.text)
}

#[test]
fn instruction_is_a_marker_that_retains_its_text() {
    let file = parser::parse_strict(SOURCE).unwrap();
    let instruction = file
        .items
        .iter()
        .find(|item| matches!(item.value, Item::Instruction { .. }))
        .unwrap();
    let Item::Instruction { text } = instruction.value else {
        panic!("{:?}", instruction.value);
    };
    assert_eq!(spelled(&file, text), "'bl'");
    assert_eq!(spelled(&file, instruction.span_id), "#instruction('bl')");
}

#[test]
fn symbol_is_not_a_directive() {
    let diagnostics = parser::parse_strict("#symbol('a')\nS has\n").unwrap_err();
    assert!(format!("{diagnostics:?}").contains("UnknownCompilerQuery"));
}

#[test]
fn an_instruction_function_has_no_body() {
    let file = parser::parse_strict(SOURCE).unwrap();
    let binding = file
        .items
        .iter()
        .find_map(|item| match &item.value {
            Item::Binding(binding) if binding.name.value == "branch_link" => Some(binding),
            _ => None,
        })
        .unwrap();
    assert!(binding.value.is_none());
    let parameters = &binding.parameters.as_ref().unwrap().value;
    assert!(
        parameters[0].value.annotation.is_none(),
        "the literal parameter"
    );
}

#[test]
fn register_is_a_field_attribute_that_retains_its_name() {
    let file = parser::parse_strict(SOURCE).unwrap();
    let declaration = file
        .items
        .iter()
        .find_map(|item| match &item.value {
            Item::Type(declaration) if declaration.name.value == "Inputs" => Some(declaration),
            _ => None,
        })
        .unwrap();
    let TypeBody::Has(members) = &declaration.body.value else {
        panic!("a record");
    };
    let TypeMember::Binding(field) = &members[0].value else {
        panic!("a field");
    };
    let FieldAttribute::Register { name, .. } = field.attributes[0].value else {
        panic!("{:?}", field.attributes);
    };
    assert_eq!(spelled(&file, name), "'x0'");
    assert_eq!(
        spelled(&file, field.attributes[0].span_id),
        "#register('x0')"
    );
}

#[test]
fn each_needs_one_string() {
    for source in [
        "#instruction\nf() in 0 to 1\n",
        "#instruction(bl)\nf() in 0 to 1\n",
        "#instruction()\nf() in 0 to 1\n",
        "#instruction('a'\nf() in 0 to 1\n",
        "S has\n    #register(x0)\n    a in 0 to 1\n",
        "S has\n    #register\n    a in 0 to 1\n",
    ] {
        assert!(parser::parse_strict(source).is_err(), "{source}");
    }
}
