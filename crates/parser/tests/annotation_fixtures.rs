use ast::{Annotation, Expr, Item, SourceFile, SyntaxNodeKind};

const CORE_BOX: &str = include_str!("../../test-fixtures/fixtures/box_storage_core_box.gin");
const MEMORY: &str = include_str!("../../test-fixtures/fixtures/machine_memory_binary_copy.gin");

fn original_line<'a>(fixture: &'a str, expected: &str) -> &'a str {
    let line = fixture
        .split_inclusive('\n')
        .find(|line| line.trim_end() == expected)
        .expect("claimed original fixture line");
    assert_eq!(line.trim_end(), expected);
    line
}

fn check_expression_annotations(file: &SourceFile, parameter_type: &str) {
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("original function binding");
    };
    let parameters = &binding.parameters.as_ref().unwrap().value;
    assert_eq!(parameters.len(), 2);
    for parameter in parameters {
        let annotation = parameter.value.annotation.as_ref().unwrap();
        let Annotation::Expression(expression) = &annotation.value else {
            panic!("ordinary named parameter annotation");
        };
        assert_eq!(expression.value, Expr::Name(parameter_type.into()));
        assert_eq!(
            file.span_table.get(annotation.span_id).extract(&file.text),
            parameter_type
        );
    }
    assert!(matches!(
        &binding.annotation.as_ref().unwrap().value,
        Annotation::Expression(expression) if expression.value == Expr::Name(parameter_type.into())
    ));
}

#[test]
fn original_replace_function_lines_keep_parameter_and_return_annotations() {
    // These are exact standalone lines, not claims that the complete storage/ownership
    // fixtures are supported by the source parser.
    for (fixture, expected, name) in [
        (
            CORE_BOX,
            "replace(previous Byte, next Byte) Byte: return next",
            "replace",
        ),
        (
            MEMORY,
            "replace_byte(previous Byte, next Byte) Byte: return next",
            "replace_byte",
        ),
    ] {
        let source = original_line(fixture, expected);
        let file = parser::parse_strict(source).unwrap();
        assert_eq!(file.text.as_ref(), source);
        assert_eq!(file.items.len(), 1);
        check_expression_annotations(&file, "Byte");
        for (offset, _) in source.match_indices("Byte") {
            let hit = file.node_at_offset(offset).unwrap();
            assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("Byte")));
            assert_eq!(hit.declarations.last().unwrap().name, name);
            assert!(
                hit.ancestors
                    .iter()
                    .any(|node| matches!(node.kind, SyntaxNodeKind::Annotation(_)))
            );
        }
    }
}

#[test]
fn original_noncanonical_local_line_recovers_a_domain_annotation() {
    // Parse only this original local binding, removing its enclosing body's indentation.
    // This is not a claim that the complete ownership/storage fixture parses.
    let source = original_line(CORE_BOX, "    recovered is in Byte: failed_allocation")
        .strip_prefix("    ")
        .unwrap();
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    assert!(parser::parse_strict(source).is_err());
    let diagnostic = output
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.span.extract(source) == "is")
        .expect("noncanonical header diagnostic at the written is");
    assert!(diagnostic.help.as_deref().unwrap().contains("in"));
    let file = output.file;
    assert_eq!(file.text.as_ref(), source);
    assert_eq!(file.items.len(), 1);
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("recovered binding")
    };
    let annotation = binding.annotation.as_ref().unwrap();
    let Annotation::Domain { is_keyword, domain } = &annotation.value else {
        panic!("written local domain");
    };
    assert_eq!(
        file.span_table.get(is_keyword.unwrap()).extract(source),
        "is"
    );
    assert!(domain.value.refinements.is_empty());
    assert_eq!(
        file.span_table.get(annotation.span_id).extract(source),
        "is in Byte"
    );
    let hit = file.node_at_offset(source.find("Byte").unwrap()).unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("Byte")));
    assert_eq!(hit.declarations.last().unwrap().name, "recovered");
}
