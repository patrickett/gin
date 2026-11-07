use ast::{FieldAttribute, Item, SyntaxNodeKind, TypeBody, TypeMember};

#[test]
fn private_grouped_annotations_are_fields_not_parameter_lists() {
    for annotation in ["(T,U)", "(T,)", "()", "(T)"] {
        let source = format!("Holder has\n    #private\n    field {annotation}\nlater: 2\n");
        let file = parser::parse_strict(&source).unwrap();
        let Item::Type(declaration) = &file.items[0].value else {
            panic!("type")
        };
        let TypeBody::Has(members) = &declaration.body.value else {
            panic!("members")
        };
        let TypeMember::Binding(field) = &members[0].value else {
            panic!("field")
        };
        assert!(field.parameters.is_none());
        assert_eq!(field.attributes.len(), 1);
        assert_eq!(
            file.span_table
                .get(field.annotation.as_ref().unwrap().span_id)
                .extract(&source),
            annotation,
        );
        assert_eq!(file.declaration_outline().last().unwrap().name, "later");
    }
}

#[test]
fn standalone_private_fields_preserve_order_and_punctuation_spans() {
    let source =
        "Thing has\n    #private\n    #private\n    value own Int\n    next Int\nprivate\n";
    let file = parser::parse_strict(source).unwrap();
    let Item::Type(declaration) = &file.items[0].value else {
        panic!("type")
    };
    let TypeBody::Has(members) = &declaration.body.value else {
        panic!("members")
    };
    assert_eq!(members.len(), 2);
    let TypeMember::Binding(field) = &members[0].value else {
        panic!("field")
    };
    assert_eq!(field.attributes.len(), 2);
    let mut previous = 0;
    for attribute in &field.attributes {
        let span = file.span_table.get(attribute.span_id);
        assert_eq!(span.extract(source), "#private");
        assert!(span.start() >= previous);
        previous = span.end();
        let FieldAttribute::Private {
            hash,
            private_keyword,
        } = attribute.value
        else {
            panic!("private attribute");
        };
        assert_eq!(file.span_table.get(hash).extract(source), "#");
        assert_eq!(
            file.span_table.get(private_keyword).extract(source),
            "private"
        );
    }
    assert_eq!(
        file.span_table.get(members[0].span_id).extract(source),
        "#private\n    #private\n    value own Int"
    );
    let TypeMember::Binding(next) = &members[1].value else {
        panic!("field")
    };
    assert!(next.attributes.is_empty());
    assert!(matches!(file.items[1].value, Item::Private));
}

#[test]
fn unsupported_attachments_are_diagnosed() {
    for source in [
        "#private\nvalue Int\n",
        "Thing has #private value Int\n",
        "Thing has\n    #private value Int\n",
        "Thing has\n    #private\n    Other is Int\n",
        "Thing has\n    #private\n",
        "Thing has\n    #private(arg)\n    value Int\n",
        "Thing has\n    #phantom\n    value Int\n",
        "run():\n    #private\n    local Int\n",
        "Thing has\n    #private\n    receiver.field Int\n",
        "Thing has\n    #private\n    value Int:\n        return 1\n",
    ] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert!(parser::parse_strict(source).is_err(), "{source}");
    }
}

#[test]
fn dangling_attribute_does_not_attach_across_a_dedent() {
    let source = "Thing has\n    #private\nlater := 1\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    let Item::Binding(later) = &output.file.items[1].value else {
        panic!("later survives")
    };
    assert_eq!(later.name.value, "later");
    assert!(later.attributes.is_empty());
}

#[test]
fn private_field_recovers_after_an_unfinished_type_application() {
    let source = "Thing has\n    broken Int(\n    #private\n    value Int\nlater := 2\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    let Item::Type(declaration) = &output.file.items[0].value else {
        panic!("type");
    };
    let TypeBody::Has(members) = &declaration.body.value else {
        panic!("members");
    };
    let TypeMember::Binding(field) = &members[1].value else {
        panic!("field after broken type");
    };
    assert_eq!(field.name.value, "value");
    assert_eq!(field.attributes.len(), 1);
    let hit = output
        .file
        .node_at_offset(source.find("#private").unwrap() + 1)
        .unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::PrivateKeyword));
    assert_eq!(hit.declarations[0].name, "Thing");
    assert_eq!(hit.declarations[1].name, "value");
    let Item::Binding(later) = &output.file.items[1].value else {
        panic!("later");
    };
    assert_eq!(later.name.value, "later");
}

#[test]
fn private_attribute_edit_prefixes_remain_recoverable() {
    let source = "Thing has\n    -- hé 🦀\n    #private\n    value Int\nlater := 1\n";
    for end in 0..=source.len() {
        if !source.is_char_boundary(end) {
            continue;
        }
        let prefix = &source[..end];
        let output = parser::parse(prefix).unwrap();
        for diagnostic in &output.diagnostics {
            assert!(prefix.get(diagnostic.span.to_range()).is_some());
        }
    }
}

#[test]
fn commas_separate_field_attributes_like_newlines() {
    let comma = "Thing has\n    #private, #private\n    value Int\n";
    let lines = "Thing has\n    #private\n    #private\n    value Int\n";
    for source in [comma, lines] {
        let file = parser::parse_strict(source).unwrap();
        let Item::Type(declaration) = &file.items[0].value else {
            panic!("type")
        };
        let TypeBody::Has(members) = &declaration.body.value else {
            panic!("members")
        };
        let TypeMember::Binding(field) = &members[0].value else {
            panic!("field")
        };
        assert_eq!(field.attributes.len(), 2);
        for attribute in &field.attributes {
            assert_eq!(
                file.span_table.get(attribute.span_id).extract(source),
                "#private"
            );
        }
    }
}

#[test]
fn dangling_attribute_commas_are_rejected() {
    for source in [
        "Thing has\n    #private,\n    value Int\n",
        "Thing has\n    #private,, #private\n    value Int\n",
    ] {
        assert!(parser::parse_strict(source).is_err(), "{source:?}");
    }
}
