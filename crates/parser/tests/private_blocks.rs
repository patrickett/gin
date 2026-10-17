use ast::{BindAttributes, DeclareValue, HasMember};
use internment::Intern;
use parser::query::SourceParseExt;

fn attributes(member: &HasMember) -> &BindAttributes {
    match member {
        HasMember::Property(field) => &field.attributes,
        HasMember::Function(method) => &method.attributes,
    }
}

#[test]
fn private_blocks_flatten_fields_and_methods_without_leaking() {
    let source = r#"Record has
    visible Word
    #private
        secret Word
        #inline
        hidden(self ref) Word:
            when self.secret = 0 then
                return 1
            return self.secret
        make(value Word) Self: return Self(value, value)
    read(self ref) Word: return self.secret
    #private(client)
        shared Word
    last Word
Other has value Word
"#;
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let declaration = &parsed.ast.tags[&Intern::from_ref("Record")];
    let DeclareValue::Has(members) = &declaration.value else {
        panic!("expected members")
    };
    assert_eq!(members.len(), 7);
    assert_eq!(
        members
            .iter()
            .map(|member| attributes(member).is_private())
            .collect::<Vec<_>>(),
        [false, true, true, true, false, true, false],
    );
    assert!(attributes(&members[2]).inline_always);
    assert!(matches!(
        attributes(&members[5]).raw_attributes.as_deref(),
        Some([ast::AttributeItem::Call { name, args, .. }])
            if name.as_str() == "private" && args.len() == 1
    ));
    assert!(parsed.ast.tags.contains_key(&Intern::from_ref("Other")));
}

#[test]
fn private_blocks_end_at_eof_or_before_another_declaration() {
    for ending in ["", "\n", "\nOther has value Word\n"] {
        for separator in ["", ","] {
            let source = format!(
                "Record has\n    #private\n        first Word,\n        last Word{separator}{ending}"
            );
            let parsed = source.parse_source_full();
            assert!(
                parsed.symptoms.is_empty(),
                "{source}: {:?}",
                parsed.symptoms
            );
            let DeclareValue::Has(members) = &parsed.ast.tags[&Intern::from_ref("Record")].value
            else {
                panic!("expected members")
            };
            assert_eq!(members.len(), 2);
            assert!(members.iter().all(|member| attributes(member).is_private()));
            assert_eq!(
                parsed.ast.tags.len(),
                1 + usize::from(ending.contains("Other"))
            );
        }
    }
}

#[test]
fn docs_and_other_attributes_belong_to_private_members() {
    let source = r#"Record has
    #private(client, other.module)
        --- Stored value
        secret Word
        --- Read the value
        #inline
        hidden(self ref) Word: return self.secret
"#;
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let DeclareValue::Has(members) = &parsed.ast.tags[&Intern::from_ref("Record")].value else {
        panic!("expected members")
    };
    let HasMember::Property(field) = &members[0] else {
        panic!("expected field")
    };
    let HasMember::Function(method) = &members[1] else {
        panic!("expected method")
    };
    assert!(field.doc_comment.is_some());
    assert!(method.doc_comment.is_some());
    assert!(method.attributes.inline_always);
    assert!(members.iter().all(|member| attributes(member).is_private()));
}

#[test]
fn private_blocks_support_generic_and_qualified_members_in_composed_has_bodies() {
    let source = r#"Record(element) has Copy(element)
    #private(other.module)
        storage own element
        Copy.inspect(self ref) element
    read(self ref) element: return self.storage
Other has value Word
"#;
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let DeclareValue::Has(members) = &parsed.ast.tags[&Intern::from_ref("Record")].value else {
        panic!("expected members")
    };
    assert_eq!(members.len(), 3);
    let HasMember::Property(field) = &members[0] else {
        panic!("expected field")
    };
    let HasMember::Function(method) = &members[1] else {
        panic!("expected method")
    };
    assert!(field.owned_storage);
    assert_eq!(method.qualifier.as_ref().unwrap().name.as_str(), "Copy");
    assert!(field.attributes.is_private());
    assert!(method.attributes.is_private());
    assert!(!attributes(&members[2]).is_private());
    assert_eq!(parsed.ast.tags.len(), 2);
}

#[test]
fn private_blocks_reject_nested_or_conflicting_permissions_and_other_attribute_blocks() {
    for (source, code) in [
        (
            "Record has\n    #private\n        #private(client)\n            secret Word\n",
            "parse-nested-private-block",
        ),
        (
            "Record has\n    #private\n        #private(client)\n        secret Word\n",
            "parse-private-block-override",
        ),
        (
            "Record has\n    #inline\n        hidden(self): return\n",
            "parse-invalid-private-block",
        ),
        (
            "Record has\n    #private, #inline\n        hidden(self): return\n",
            "parse-invalid-private-block",
        ),
    ] {
        let parsed = source.parse_source_full();
        assert!(
            parsed
                .symptoms
                .iter()
                .any(|symptom| symptom.code.slug() == code),
            "{source}: {:?}",
            parsed.symptoms
        );
    }
}
