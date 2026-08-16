use ast::{ConstValue, Pattern};
use internment::Intern;
use typecheck::analysis::pattern_matches_public;
use typecheck::reflect::{
    ReflectedRepresentation, ReflectedSemantics, reflect_ty_to_reflected_type,
};
use typecheck::ty::{Ty, UnionVariant};

#[test]
fn anonymous_integer_reflection_is_structural() {
    let reflected = reflect_ty_to_reflected_type(&Ty::anonymous_integer_for_width(8, false), None);

    assert!(matches!(
        reflected.representation,
        ReflectedRepresentation::Bits { width: 8 }
    ));
}

#[test]
fn address_and_safe_reference_have_distinct_reflection_shapes() {
    let integer = Ty::anonymous_integer_for_width(8, false);
    let address = reflect_ty_to_reflected_type(
        &Ty::Address {
            pointee: Box::new(integer.clone()),
            address_space: 3,
        },
        None,
    );
    let reference = reflect_ty_to_reflected_type(
        &Ty::Ref {
            inner: Box::new(integer),
            mutable: false,
        },
        None,
    );

    assert!(matches!(
        address.semantics,
        ReflectedSemantics::RawPointer { .. }
    ));
    assert!(matches!(
        reference.semantics,
        ReflectedSemantics::SafeReference { mutable: false, .. }
    ));
}

#[test]
fn list_patterns_match_empty_and_cons_values() {
    assert!(pattern_matches_public(
        &Pattern::ListEmpty,
        &ConstValue::List(Vec::new().into())
    ));
    let pattern = Pattern::ListCons {
        head: Box::new(ast::Spanned {
            value: Pattern::Nominal(Intern::from_ref("head"), ast::SpanId::INVALID),
            span_id: ast::SpanId::INVALID,
        }),
        tail: Box::new(ast::Spanned {
            value: Pattern::Nominal(Intern::from_ref("tail"), ast::SpanId::INVALID),
            span_id: ast::SpanId::INVALID,
        }),
    };

    assert!(pattern_matches_public(
        &pattern,
        &ConstValue::List(vec![ConstValue::Int(1.into())].into())
    ));
}

#[test]
fn boolean_union_reflection_preserves_alternatives() {
    let boolean = Ty::Union {
        name: Intern::from_ref("Bool"),
        variants: vec![
            UnionVariant::new(Intern::from_ref("True"), vec![]),
            UnionVariant::new(Intern::from_ref("False"), vec![]),
        ],
        literal_values: None,
        resolved_params: None,
    };
    let reflected = reflect_ty_to_reflected_type(&boolean, None);

    assert!(matches!(
        reflected.representation,
        ReflectedRepresentation::Sum { variants } if variants.len() == 2
    ));
}
