use ast::{Annotation, AnnotationKeyword, Expr, Item, SourceFile, SyntaxNodeKind, TypeBody};

fn annotation(file: &SourceFile, index: usize) -> &Annotation {
    let Item::Binding(binding) = &file.items[index].value else {
        panic!("written binding");
    };
    &binding.annotation.as_ref().unwrap().value
}

#[test]
fn canonical_example_distinguishes_type_definitions_and_value_membership() {
    let source = "\
Byte is in 0 to 255
count in Byte: 42
read(value in Byte) Byte: return value
";
    let file = parser::parse_strict(source).expect("canonical domain headers");
    let Item::Type(byte) = &file.items[0].value else {
        panic!("type definition keeps is");
    };
    assert!(matches!(byte.body.value, TypeBody::Domain(_)));
    assert!(matches!(
        annotation(&file, 1),
        Annotation::Domain {
            is_keyword: None,
            ..
        }
    ));
    let Item::Binding(read) = &file.items[2].value else {
        panic!("ordinary function");
    };
    let parameter = &read.parameters.as_ref().unwrap().value[0].value;
    assert!(matches!(
        parameter.annotation.as_ref().unwrap().value,
        Annotation::Domain {
            is_keyword: None,
            ..
        }
    ));
    assert!(matches!(annotation(&file, 2), Annotation::Expression(_)));
    assert_eq!(file.text.as_ref(), source);
    assert_eq!(
        file.declaration_outline()
            .map(|entry| entry.name)
            .collect::<Vec<_>>(),
        ["Byte", "count", "read"]
    );
}

#[test]
fn canonical_in_is_consistent_across_supported_header_positions() {
    let source = "\
value in Byte: 1
read(value in Byte) in Byte:
    local in Byte: value
    return local
Generic(value in 0 to 9) is Ready
Thing has
    method(self, value in Byte) Byte: return value
";
    let file = parser::parse_strict(source).unwrap();
    for index in [0, 1] {
        assert!(matches!(
            annotation(&file, index),
            Annotation::Domain {
                is_keyword: None,
                ..
            }
        ));
    }
    for offset in 0..=source.len() {
        let hit = file.node_at_offset(offset).unwrap();
        assert!(!matches!(
            hit.node.kind,
            SyntaxNodeKind::AnnotationKeyword(_)
        ));
        assert!(source.get(hit.node.span.to_range()).is_some());
    }
}

#[test]
fn old_header_is_in_is_recovered_faithfully_but_is_not_strict_syntax() {
    for (header, errors) in [
        ("value is in Byte: 1", 1),
        ("read(value is in Byte) is in Byte: return value", 2),
        ("Generic(value is in Byte) is Ready", 1),
    ] {
        let source = format!("Byte is in 0 to 255\n{header}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert_eq!(output.diagnostics.len(), errors);
        assert_eq!(output.file.text.as_ref(), source);
        for diagnostic in &output.diagnostics {
            assert_eq!(diagnostic.span.extract(&source), "is");
            assert!(diagnostic.message.contains("in"));
            assert!(
                diagnostic
                    .help
                    .as_ref()
                    .is_some_and(|help| help.contains("in"))
            );
            let hit = output.file.node_at_offset(diagnostic.span.start()).unwrap();
            assert!(matches!(
                hit.node.kind,
                SyntaxNodeKind::AnnotationKeyword(AnnotationKeyword::Is)
            ));
        }
        let names: Vec<_> = output
            .file
            .declaration_outline()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names.last(), Some(&"later"));
        assert!(parser::parse_strict(&source).is_err());
    }
}

#[test]
fn noncanonical_header_diagnostics_do_not_discard_domains_or_later_items() {
    let source = "value is in 0 to\nNext has\n    member Int\nlater := 2\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    assert!(matches!(
        annotation(&output.file, 0),
        Annotation::Domain {
            is_keyword: Some(_),
            ..
        }
    ));
    assert_eq!(
        output
            .file
            .declaration_outline()
            .map(|entry| entry.name)
            .collect::<Vec<_>>(),
        ["value", "Next", "later"]
    );
    let hit = output
        .file
        .node_at_offset(source.find("later").unwrap())
        .unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name("later")));
}

#[test]
fn predicate_is_and_type_definition_is_in_remain_canonical() {
    let source = "\
Byte is in 0 to 255
positive is >= 0 and < limit: value
check(value is >= 0) is >= 0: return value
";
    let file = parser::parse_strict(source).unwrap();
    assert!(matches!(annotation(&file, 1), Annotation::Predicate { .. }));
    assert!(matches!(annotation(&file, 2), Annotation::Predicate { .. }));
    assert_eq!(file.items.len(), 3);
}

#[test]
fn canonical_header_holes_remain_owned_at_close_paren_and_eof() {
    for source in [
        "f(value in): return value\nlater := 2\n",
        "value in\n    -- hé 🦀\n    ",
    ] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors());
        let offset = source.find(')').unwrap_or(source.len());
        let hit = output.file.node_at_offset(offset).unwrap();
        assert!(matches!(
            hit.node.kind,
            SyntaxNodeKind::Expression(expression) if expression.value == Expr::Missing
        ));
        assert!(hit.node.span.is_empty());
        assert!(!hit.declarations.is_empty());
    }
}

#[test]
fn noncanonical_spelling_diagnostics_respect_the_diagnostic_budget() {
    let options = parser::ParseOptions {
        limits: parser::ParseLimits {
            diagnostics_max: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(matches!(
        parser::parse_with_options(
            "read(value is in Byte) is in Byte: return value\n",
            options
        ),
        Err(parser::ParseFailure::LimitExceeded {
            limit: parser::ParseLimit::Diagnostics,
            ..
        })
    ));
}
