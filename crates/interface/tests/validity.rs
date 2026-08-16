use interface::{EncodedIntegerValidity, IntegerExpr, IntegerPredicate, SignedInteger};

fn integer(value: i128) -> IntegerExpr {
    IntegerExpr::Constant(SignedInteger::from_i128(value))
}

#[test]
fn equivalent_conjunctions_have_identical_validity_bytes() {
    let lower = IntegerPredicate::GreaterOrEqual(
        IntegerExpr::Parameter("value".to_string()),
        integer(-128),
    );
    let upper =
        IntegerPredicate::LessOrEqual(IntegerExpr::Parameter("value".to_string()), integer(255));
    let left = EncodedIntegerValidity::new(IntegerPredicate::And(vec![
        lower.clone(),
        upper.clone(),
        lower.clone(),
    ]));
    let right = EncodedIntegerValidity::new(IntegerPredicate::And(vec![upper, lower]));

    assert_eq!(left.canonical_bytes(), right.canonical_bytes());
}

#[test]
fn specialized_predicates_and_large_integers_are_canonical() {
    let magnitude = vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let large = SignedInteger::from_sign_and_magnitude(false, magnitude).unwrap();
    let predicate = IntegerPredicate::And(vec![
        IntegerPredicate::IsPowerOfTwo(IntegerExpr::Parameter("alignment".to_string())),
        IntegerPredicate::Less(
            IntegerExpr::NonnegativeProduct(vec![
                IntegerExpr::Parameter("count".to_string()),
                IntegerExpr::Size(Box::new(IntegerExpr::Parameter("element".to_string()))),
            ]),
            IntegerExpr::Constant(large),
        ),
    ]);
    let bytes = EncodedIntegerValidity::new(predicate).canonical_bytes();

    assert!(bytes.contains(&10), "missing IsPowerOfTwo tag: {bytes:?}");
    assert!(
        bytes.contains(&17),
        "missing arbitrary-width magnitude length: {bytes:?}"
    );
}

#[test]
fn signed_integer_rejects_nonminimal_and_negative_zero_forms() {
    assert!(SignedInteger::from_sign_and_magnitude(false, vec![0, 1]).is_none());
    assert!(SignedInteger::from_sign_and_magnitude(true, Vec::new()).is_none());
}

#[test]
fn storage_hull_is_derived_from_the_complete_predicate() {
    let value = IntegerExpr::Parameter("value".to_string());
    let validity = EncodedIntegerValidity::new(IntegerPredicate::And(vec![
        IntegerPredicate::GreaterOrEqual(value.clone(), integer(-7)),
        IntegerPredicate::Less(value, integer(250)),
    ]));

    assert_eq!(validity.storage_hull_i128(), Some((-7, 249)));
}
