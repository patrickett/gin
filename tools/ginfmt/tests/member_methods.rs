use parser::query::SourceParseExt;

mod common;

use common::assert_roundtrip;

#[test]
fn formatting_preserves_nested_receiver_calls() {
    let source = "Thing has\n    read(self ref) Word: return self.bytes.read(7)\n    copy(self ref) Word: return self.bytes.copy()\n";
    let formatted = ginfmt::format(source);
    assert!(formatted.contains("self.bytes.read(7)"), "{formatted}");
    assert!(formatted.contains("self.bytes.copy()"), "{formatted}");
    let parsed = formatted.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    assert_roundtrip(&formatted);
}

#[test]
fn formatting_preserves_private_field_and_method_queries() {
    let source = "Thing(x) has\n    #private\n    address own x\n    capacity Word\n    #private\n    inspect(self ref) Word: return self.capacity\n";
    let formatted = ginfmt::format(source);
    let parsed = formatted.parse_source_full();
    assert!(
        parsed.symptoms.is_empty(),
        "{:?}\n{formatted}",
        parsed.symptoms
    );
    let declaration = parsed.ast.tags.values().next().unwrap();
    let ast::DeclareValue::Has(members) = &declaration.value else {
        panic!("expected members")
    };
    let ast::HasMember::Property(field) = &members[0] else {
        panic!("expected field")
    };
    assert!(field.attributes.is_private());
    let ast::HasMember::Function(method) = &members[2] else {
        panic!("expected method")
    };
    assert!(method.attributes.is_private());
    assert_roundtrip(&formatted);
}

#[test]
fn formatting_preserves_private_fields_without_method_bodies() {
    let source = "Thing(x) has\n    #private\n    address own x\n    capacity Word\n";
    let formatted = ginfmt::format(source);
    assert!(formatted.contains("#private"), "{formatted}");
    let parsed = formatted.parse_source_full();
    assert!(
        parsed.symptoms.is_empty(),
        "{:?}\n{formatted}",
        parsed.symptoms
    );
    assert_roundtrip(&formatted);
}

#[test]
fn formatting_preserves_private_blocks_and_grants_without_falling_back() {
    for grants in ["", "(client, other.module)"] {
        let source = format!(
            r#"Record has
    visible Word
    #private{grants}
        a Word
        longer Word
        #inline
        hidden(self ref) Word:
            return self.a
    read(self ref) Word: return self.a
"#
        );
        let formatted = ginfmt::format(&source);
        assert!(formatted.contains("        a      Word"), "{formatted}");
        assert!(
            formatted.contains(&format!("    #private{grants}\n")),
            "{formatted}"
        );
        let parsed = formatted.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{:?}\n{formatted}",
            parsed.symptoms
        );
        let ast::DeclareValue::Has(members) = &parsed.ast.tags.values().next().unwrap().value
        else {
            panic!("expected members")
        };
        let privacy: Vec<_> = members
            .iter()
            .map(|member| match member {
                ast::HasMember::Property(field) => field.attributes.is_private(),
                ast::HasMember::Function(method) => method.attributes.is_private(),
            })
            .collect();
        assert_eq!(privacy, [false, true, true, true, false]);
        assert_roundtrip(&formatted);
    }
}

#[test]
fn formatting_preserves_private_blocks_containing_only_fields() {
    let source =
        "Record has\n    #private(client)\n        a Word\n        longer Word\n    visible Word\n";
    let formatted = ginfmt::format(source);
    assert!(formatted.contains("        a      Word"), "{formatted}");
    assert!(formatted.contains("    visible Word"), "{formatted}");
    assert_roundtrip(&formatted);
}
