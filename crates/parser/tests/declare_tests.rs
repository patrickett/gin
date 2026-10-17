use ast::ParamConvention;
use ast::prelude::Intern;
use ast::{DeclareValue, Expr, HasMember};
use parser::query::SourceParseExt;

#[test]
fn preserves_removed_condition_attribute_as_raw_syntax() {
    let ast = "#condition\nTruth(value) has witness value\n"
        .parse_source_full()
        .ast;
    let truth = ast.tags.get(&Intern::from_ref("Truth")).expect("Truth");

    let attributes = truth
        .attributes
        .raw_attributes
        .as_ref()
        .expect("attributes");
    assert!(matches!(
        attributes.as_slice(),
        [ast::AttributeItem::Flag { name, .. }] if name.as_str() == "condition"
    ));
}

#[test]
fn rejects_removed_default_attribute() {
    let output = "#default(IntegerLiteral)\nInt is in 0 to 255\n".parse_source_full();
    assert!(
        output
            .symptoms
            .iter()
            .any(|diagnostic| { diagnostic.code.slug() == "parse-removed-default-attribute" })
    );
}

#[test]
fn rejects_generic_pointer_former() {
    let output = "Pointer(x) is @x\n".parse_source_full();
    assert!(
        output
            .symptoms
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "parse-removed-pointer-former")
    );
}

#[test]
fn parses_fixed_array_contract_structurally() {
    let ast = "#fixed_array(element, count)\nBuffer(element, count BigInt) has\n"
        .parse_source_full()
        .ast;
    let buffer = ast.tags.get(&Intern::from_ref("Buffer")).expect("Buffer");
    let contract = buffer
        .attributes
        .fixed_array
        .as_ref()
        .expect("fixed-array contract");
    assert_eq!(contract.element_parameter.as_str(), "element");
    assert_eq!(contract.length_parameter.as_str(), "count");
}

#[test]
fn parses_stacked_declare_attributes() {
    let ast = "#custom\n#target(\"wasm32\")\nExample has value Bool\n"
        .parse_source_full()
        .ast;
    let declaration = ast
        .tags
        .get(&Intern::new("Example".to_string()))
        .expect("Example tag should exist");
    let attrs = declaration
        .attributes
        .raw_attributes
        .as_ref()
        .expect("raw attributes should be present");

    assert_eq!(attrs.len(), 2);
}

#[test]
fn parses_same_line_declare_attributes() {
    let ast = "#custom, #target(\"wasm32\")\nExample has value Bool\n"
        .parse_source_full()
        .ast;
    let declaration = ast
        .tags
        .get(&Intern::new("Example".to_string()))
        .expect("Example tag should exist");
    let attrs = declaration
        .attributes
        .raw_attributes
        .as_ref()
        .expect("raw attributes should be present");

    assert_eq!(attrs.len(), 2);
}

#[test]
fn rejects_removed_assembly_attributes() {
    for source in [
        "#asm\nwrite(fd Int) Int\n",
        "#asm('spec')\nwrite(fd Int) Int\n",
        "#asm('a', 'b')\nwrite(fd Int) Int: return 0\n",
    ] {
        let output = source.parse_source_full();
        assert!(
            output.symptoms.iter().any(|diagnostic| {
                diagnostic.code.slug() == "parse-unknown-compiler-query"
                    && diagnostic.message.contains("asm")
            }),
            "{:?}",
            output.symptoms
        );
    }
}

#[test]
fn detached_declare_attribute_reports_diagnostic() {
    let output = "#custom\n\nExample has value Bool\n".parse_source_full();

    assert!(
        output
            .symptoms
            .iter()
            .any(|d| d.code.slug() == "parse-detached-attribute"),
        "symptoms: {:?}",
        output.symptoms
    );
}

#[test]
fn parses_has_function_signatures() {
    let source = r#"
Allocator has
    --- Attempt to allocate.
    allocate(self ref, l Layout) Slice(Byte) or AllocError,
    --- Free a block.
    deallocate(self ref, p @Byte, l Layout),
"#;
    let ast = source.parse_source_full().ast;
    let allocator = ast
        .tags
        .get(&Intern::new("Allocator".to_string()))
        .expect("Allocator tag should exist");

    let DeclareValue::Has(members) = &allocator.value else {
        panic!(
            "Allocator should be parsed as Has, got {:?}",
            allocator.value
        );
    };

    assert_eq!(members.len(), 2, "expected 2 has members");

    let HasMember::Function(allocate) = &members[0] else {
        panic!("allocate should be a function member");
    };
    assert_eq!(allocate.name.as_str(), "allocate");
    assert!(
        allocate.doc_comment.is_some(),
        "allocate should have doc comment"
    );

    // Check params
    assert_eq!(allocate.params.len(), 2);
    assert!(
        allocate
            .conventions
            .contains_key(&Intern::new("self".to_string()))
    );
    assert_eq!(
        allocate.conventions.get(&Intern::new("self".to_string())),
        Some(&ParamConvention::Observe),
    );

    let Expr::Binary(result) = &allocate.return_ty.as_ref().unwrap().value else {
        panic!("expected a direct union return type");
    };
    assert_eq!(result.op, ast::BinOp::LogicalOr);
    assert!(
        matches!(&result.lhs.value, Expr::TagCall(call) if call.name.as_str() == "Slice" && call.args.len() == 1)
    );
    assert!(matches!(&result.rhs.value, Expr::AnonymousTag(name) if name.as_str() == "AllocError"));

    let HasMember::Function(deallocate) = &members[1] else {
        panic!("deallocate should be a function member");
    };
    assert_eq!(deallocate.name.as_str(), "deallocate");
    assert!(
        deallocate.doc_comment.is_some(),
        "deallocate should have doc comment"
    );
    assert_eq!(deallocate.params.len(), 3);
    assert!(
        deallocate.return_ty.is_none(),
        "deallocate should have no return type"
    );
}

#[test]
fn marker_copy_syntax_rejected() {
    let output = "Int is in 0 to 255 and is Copy".parse_source_full();

    assert!(
        output
            .symptoms
            .iter()
            .any(|d| d.code.slug() == "parse-removed-marker-syntax"),
        "symptoms: {:?}",
        output.symptoms
    );
}

#[test]
fn provided_traits_preserve_generic_arguments_separately_from_fields() {
    let parsed = "Div(result) has\n    divide(self, rhs Self) result\nWord has Div(Outcome)\n"
        .parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let provided = &parsed.ast.tags[&Intern::from_ref("Word")].provided_traits[0];
    assert_eq!(provided.trait_name.as_str(), "Div");
    assert!(provided.fields.is_empty());
    assert!(
        matches!(&provided.args[0].value, ast::Expr::AnonymousTag(name) if name.as_str() == "Outcome")
    );
}

#[test]
fn private_queries_attach_to_fields_and_methods() {
    let source = "Thing(x) has\n    #private\n    address own x\n    visible x\n    #private\n    inspect(self ref) x: return deref self.address\n";
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let declaration = &parsed.ast.tags[&Intern::from_ref("Thing")];
    let DeclareValue::Has(members) = &declaration.value else {
        panic!("expected members")
    };
    let HasMember::Property(field) = &members[0] else {
        panic!("expected field")
    };
    assert!(field.owned_storage);
    assert!(field.attributes.is_private());
    let HasMember::Property(field) = &members[1] else {
        panic!("expected field")
    };
    assert!(!field.attributes.is_private());
    let HasMember::Function(method) = &members[2] else {
        panic!("expected method")
    };
    assert!(method.attributes.is_private());
}

#[test]
fn sealed_is_no_longer_a_compiler_query() {
    let parsed = "#sealed\nThing has value x\n".parse_source_full();
    assert!(
        parsed
            .symptoms
            .iter()
            .any(|flaw| flaw.code.slug() == "parse-unknown-compiler-query")
    );
}

#[test]
fn sequence_view_is_no_longer_a_compiler_query() {
    for source in [
        "#sequence_view(element)\nView(element) has value element\n",
        "#sequence_view(element, data, length)\nView(element) has value element\n",
    ] {
        let parsed = source.parse_source_full();
        assert!(
            parsed
                .symptoms
                .iter()
                .any(|diagnostic| diagnostic.code.slug() == "parse-unknown-compiler-query"),
            "{:?}",
            parsed.symptoms
        );
    }
}
