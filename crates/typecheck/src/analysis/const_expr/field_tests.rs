use super::*;
use parser::query::SourceParseExt;

fn field_value(members: &str, field: &str) -> Option<ConstValue> {
    let source = format!(
        "Byte is in 0 to 255\nOther has metadata Byte\nPair has Other\n{members}\nvalue: Pair(first: 3, second: 7).{field}\n"
    );
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let BindValue::Expr(expr) = &parsed.ast.defs[&Intern::from_ref("value")].value else {
        panic!("field expression")
    };
    assert!(matches!(expr.value, Expr::RecordGet { .. }));
    CompTimeEvaluator::new(&ConstEnv::default(), &parsed.ast).eval(&expr.value)
}

#[test]
fn methods_do_not_shift_constant_payload_field_indices() {
    for members in [
        "    idle(self)\n    first Byte\n    second Byte",
        "    first Byte\n    idle(self)\n    second Byte",
        "    first Byte\n    second Byte\n    idle(self)",
    ] {
        for (field, expected) in [("first", 3), ("second", 7)] {
            assert_eq!(
                field_value(members, field),
                Some(ConstValue::Int(expected.into())),
                "{members}: {field}"
            );
        }
    }
}

#[test]
fn methods_and_absent_fields_have_no_constant_payload_slot() {
    let members = "    idle(self)\n    first Byte\n    second Byte";
    for field in ["idle", "absent"] {
        assert_eq!(field_value(members, field), None, "{field}");
    }
}

#[test]
fn explicit_record_qualifiers_preserve_stored_field_order() {
    let members = "    idle(self)\n    Pair.first Byte\n    idle_again(self)\n    Pair.second Byte";
    for (field, expected) in [("first", 3), ("second", 7)] {
        assert_eq!(
            field_value(members, field),
            Some(ConstValue::Int(expected.into())),
            "{field}"
        );
    }
}

#[test]
fn foreign_qualified_properties_have_no_constant_payload_slot() {
    let members = "    Other.metadata Byte\n    first Byte\n    second Byte";
    for (field, expected) in [("first", 3), ("second", 7)] {
        assert_eq!(
            field_value(members, field),
            Some(ConstValue::Int(expected.into())),
            "{field}"
        );
    }
    assert_eq!(field_value(members, "metadata"), None);
}
