use interface::{
    DeclarationRef, EncodedIntegerValidity, EncodedResultAlternative, EncodedResultFamily,
    EncodedResultFamilyOwner, IntegerExpr, IntegerPredicate, SemanticDecodeError, SignedInteger,
    StructuralOperator,
};

fn constant(value: i128) -> IntegerExpr {
    IntegerExpr::Constant(SignedInteger::from_i128(value))
}

fn expressions() -> Vec<IntegerExpr> {
    let parameter = IntegerExpr::Parameter("p".to_string());
    vec![
        constant(0),
        constant(-129),
        parameter.clone(),
        IntegerExpr::Declaration(DeclarationRef::Subject {
            module_path: "m".to_string(),
            declaration_path: "D".to_string(),
        }),
        IntegerExpr::Size(Box::new(parameter.clone())),
        IntegerExpr::Alignment(Box::new(parameter.clone())),
        IntegerExpr::IndexBits(Box::new(parameter.clone())),
        IntegerExpr::Negate(Box::new(parameter.clone())),
        IntegerExpr::Add(vec![parameter.clone(), constant(1)]),
        IntegerExpr::MultiplyConstant(SignedInteger::from_i128(-2), Box::new(parameter.clone())),
        IntegerExpr::PowerOfTwo(Box::new(parameter.clone())),
        IntegerExpr::NonnegativeProduct(vec![parameter, constant(4)]),
    ]
}

#[test]
fn every_integer_expression_and_predicate_tag_round_trips() {
    let expressions = expressions();
    let predicates = vec![
        IntegerPredicate::Equal(expressions[0].clone(), expressions[1].clone()),
        IntegerPredicate::NotEqual(expressions[1].clone(), expressions[2].clone()),
        IntegerPredicate::Less(expressions[2].clone(), expressions[3].clone()),
        IntegerPredicate::LessOrEqual(expressions[3].clone(), expressions[4].clone()),
        IntegerPredicate::Greater(expressions[4].clone(), expressions[5].clone()),
        IntegerPredicate::GreaterOrEqual(expressions[5].clone(), expressions[6].clone()),
        IntegerPredicate::InclusiveRange {
            value: expressions[6].clone(),
            minimum: expressions[7].clone(),
            maximum: expressions[8].clone(),
        },
        IntegerPredicate::And(vec![IntegerPredicate::IsPowerOfTwo(expressions[9].clone())]),
        IntegerPredicate::Or(vec![IntegerPredicate::Equal(
            expressions[10].clone(),
            expressions[11].clone(),
        )]),
        IntegerPredicate::Not(Box::new(IntegerPredicate::Equal(
            expressions[0].clone(),
            expressions[2].clone(),
        ))),
        IntegerPredicate::IsPowerOfTwo(expressions[11].clone()),
    ];

    for predicate in predicates {
        let validity = EncodedIntegerValidity::new(predicate);
        let bytes = validity.canonical_bytes();
        let decoded = EncodedIntegerValidity::decode(&bytes).unwrap();
        assert_eq!(decoded.canonical_bytes(), bytes);
    }
}

#[test]
fn result_family_owner_tags_and_alternative_order_round_trip() {
    for owner in [
        EncodedResultFamilyOwner::Callable(DeclarationRef::Subject {
            module_path: "m".to_string(),
            declaration_path: "f".to_string(),
        }),
        EncodedResultFamilyOwner::Structural(StructuralOperator::GreaterOrEqual),
    ] {
        let family = EncodedResultFamily {
            owner,
            alternatives: vec![EncodedResultAlternative {
                label: "True".to_string(),
                predicate: IntegerPredicate::Equal(constant(1), constant(1)),
            }],
        };
        let bytes = family.canonical_bytes();
        assert_eq!(EncodedResultFamily::decode(&bytes), Ok(family));
    }
}

#[test]
fn semantic_decoder_rejects_unknown_truncated_and_nonminimal_bytes() {
    assert_eq!(
        EncodedIntegerValidity::decode(&[255]),
        Err(SemanticDecodeError::InvalidTag)
    );
    assert_eq!(
        EncodedIntegerValidity::decode(&[0]),
        Err(SemanticDecodeError::Truncated)
    );
    assert_eq!(
        EncodedIntegerValidity::decode(&[0, 1, 0x80, 0, 0, 0]),
        Err(SemanticDecodeError::Noncanonical)
    );
}

#[test]
fn signed_integer_boundary_has_stable_golden_bytes() {
    let validity =
        EncodedIntegerValidity::new(IntegerPredicate::Equal(constant(-129), constant(128)));

    assert_eq!(
        validity.canonical_bytes(),
        vec![0, 0, 2, 1, 129, 0, 1, 1, 128]
    );
}
