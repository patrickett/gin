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
fn parses_fixed_array_contract_structurally() {
    let ast = "#fixed_array(element, count)\nBuffer(element Type, count BigInt) has\n"
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
    allocate(ref self, l Layout) Slice(Byte) or AllocError,
    --- Free a block.
    deallocate(ref self, p Pointer(Byte), l Layout),
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

    // Check return type: Slice(Byte)
    assert!(
        allocate.return_ty.is_some(),
        "allocate should have return type"
    );
    let rt = &allocate.return_ty.as_ref().unwrap().value;
    match rt {
        Expr::TagCall(call) => {
            assert_eq!(call.name.as_str(), "Slice");
            assert_eq!(call.args.len(), 1);
        }
        other => panic!("allocate return type should be Generic, got {:?}", other),
    }

    // Check error type: AllocError
    assert!(
        allocate.error_ty.is_some(),
        "allocate should have error type"
    );
    let et = &allocate.error_ty.as_ref().unwrap().value;
    match et {
        Expr::AnonymousTag(name) => {
            assert_eq!(name.as_str(), "AllocError");
        }
        other => panic!("allocate error type should be AllocError, got {:?}", other),
    }

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
    assert!(
        deallocate.error_ty.is_none(),
        "deallocate should have no error type"
    );
}

#[test]
fn marker_copy_syntax_rejected() {
    let output = "Int is in 0...255 and is Copy".parse_source_full();

    assert!(
        output
            .symptoms
            .iter()
            .any(|d| d.code.slug() == "parse-removed-marker-syntax"),
        "symptoms: {:?}",
        output.symptoms
    );
}
