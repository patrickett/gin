use interface::{
    Compatibility, DeclarationRef, EncodedIntegerValidity, Fingerprint, IntegerExpr,
    IntegerPredicate, InterfaceDeclaration, PackageInstanceId, PublicInterface, SignedInteger,
    ValidityPosition, classify_interfaces, classify_validity,
};

fn interface(declarations: &[(&str, &[u8])]) -> PublicInterface {
    PublicInterface::from_subject_and_declarations(
        PackageInstanceId {
            package: "pkg".to_string(),
            version: "1".to_string(),
            source: "workspace".to_string(),
            instance: "root".to_string(),
        },
        declarations
            .iter()
            .map(|(name, body)| InterfaceDeclaration {
                reference: DeclarationRef::Subject {
                    module_path: "api".to_string(),
                    declaration_path: (*name).to_string(),
                },
                fingerprint: Fingerprint::from_bytes(body),
            })
            .collect(),
    )
}

fn validity(minimum: i128, maximum: i128) -> EncodedIntegerValidity {
    EncodedIntegerValidity::new(IntegerPredicate::InclusiveRange {
        value: IntegerExpr::Parameter("value".to_string()),
        minimum: IntegerExpr::Constant(SignedInteger::from_i128(minimum)),
        maximum: IntegerExpr::Constant(SignedInteger::from_i128(maximum)),
    })
}

#[test]
fn declaration_addition_is_compatible_but_removal_or_change_breaks() {
    let base = interface(&[("value", b"v1")]);
    let added = interface(&[("value", b"v1"), ("other", b"v1")]);
    let changed = interface(&[("value", b"v2")]);

    assert_eq!(classify_interfaces(&base, &base), Compatibility::Unchanged);
    assert_eq!(
        classify_interfaces(&base, &added),
        Compatibility::Compatible
    );
    assert_eq!(classify_interfaces(&added, &base), Compatibility::Breaking);
    assert_eq!(
        classify_interfaces(&base, &changed),
        Compatibility::Breaking
    );
}

#[test]
fn validity_variance_depends_on_parameter_or_return_position() {
    let narrow = validity(0, 10);
    let wide = validity(-10, 20);

    assert_eq!(
        classify_validity(&narrow, &wide, ValidityPosition::Parameter),
        Compatibility::Compatible
    );
    assert_eq!(
        classify_validity(&wide, &narrow, ValidityPosition::Parameter),
        Compatibility::Breaking
    );
    assert_eq!(
        classify_validity(&wide, &narrow, ValidityPosition::Return),
        Compatibility::Compatible
    );
    assert_eq!(
        classify_validity(&narrow, &wide, ValidityPosition::Return),
        Compatibility::Breaking
    );
}

#[test]
fn unsupported_predicate_implication_is_conservative() {
    let old = EncodedIntegerValidity::new(IntegerPredicate::IsPowerOfTwo(IntegerExpr::Parameter(
        "value".to_string(),
    )));
    let new = EncodedIntegerValidity::new(IntegerPredicate::NotEqual(
        IntegerExpr::Parameter("value".to_string()),
        IntegerExpr::Constant(SignedInteger::from_i128(0)),
    ));

    assert_eq!(
        classify_validity(&old, &new, ValidityPosition::Parameter),
        Compatibility::Unknown
    );
}
