use interface::{
    DeclarationRef, EncodedResultAlternative, EncodedResultFamily, EncodedResultFamilyOwner,
    IntegerExpr, IntegerPredicate, SignedInteger, StructuralOperator,
};

fn truth(value: i128) -> IntegerPredicate {
    IntegerPredicate::Equal(
        IntegerExpr::Constant(SignedInteger::from_i128(value)),
        IntegerExpr::Constant(SignedInteger::from_i128(value)),
    )
}

#[test]
fn callable_result_family_encoding_is_owner_qualified() {
    let family = |owner: &str| EncodedResultFamily {
        owner: EncodedResultFamilyOwner::Callable(DeclarationRef::Subject {
            module_path: "math".to_string(),
            declaration_path: owner.to_string(),
        }),
        alternatives: vec![
            EncodedResultAlternative {
                label: "True".to_string(),
                predicate: truth(1),
            },
            EncodedResultAlternative {
                label: "False".to_string(),
                predicate: truth(0),
            },
        ],
    };

    assert_ne!(
        family("less").canonical_bytes(),
        family("greater").canonical_bytes()
    );
}

#[test]
fn structural_and_callable_owners_cannot_alias() {
    let structural = EncodedResultFamily {
        owner: EncodedResultFamilyOwner::Structural(StructuralOperator::Less),
        alternatives: vec![],
    };
    let callable = EncodedResultFamily {
        owner: EncodedResultFamilyOwner::Callable(DeclarationRef::Subject {
            module_path: "operators".to_string(),
            declaration_path: "Less".to_string(),
        }),
        alternatives: vec![],
    };

    assert_ne!(structural.canonical_bytes(), callable.canonical_bytes());
}
