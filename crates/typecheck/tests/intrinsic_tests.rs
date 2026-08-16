use ast::{OperatorRole, ResultAlternative, ResultFamilyOwner, Ty};
use internment::Intern;
use typecheck::intrinsic::{
    ComparisonPredicate, IntegerComparison, IntrinsicOp, IntrinsicValidationError,
};

fn unsigned_less() -> IntrinsicOp {
    IntrinsicOp::BitsCompare {
        interpretation: IntegerComparison::Unsigned,
        predicate: ComparisonPredicate::Less,
    }
}

#[test]
fn comparison_accepts_the_structural_truth_result_family() {
    let result = Ty::ResultFamily {
        owner: ResultFamilyOwner::Structural(OperatorRole::Less),
        alternatives: vec![
            ResultAlternative {
                label: Intern::from_ref("False"),
                proposition: None,
            },
            ResultAlternative {
                label: Intern::from_ref("True"),
                proposition: None,
            },
        ],
    };

    assert_eq!(
        unsigned_less().validate_signature(
            &[
                Ty::anonymous_integer_for_width(8, false),
                Ty::anonymous_integer_for_width(8, false),
            ],
            &result,
            None,
        ),
        Ok(())
    );
}

#[test]
fn comparison_rejects_a_result_family_without_boolean_alternatives() {
    let result = Ty::ResultFamily {
        owner: ResultFamilyOwner::Structural(OperatorRole::Less),
        alternatives: vec![ResultAlternative {
            label: Intern::from_ref("Maybe"),
            proposition: None,
        }],
    };

    assert_eq!(
        unsigned_less().validate_signature(
            &[
                Ty::anonymous_integer_for_width(8, false),
                Ty::anonymous_integer_for_width(8, false),
            ],
            &result,
            None,
        ),
        Err(IntrinsicValidationError::InvalidComparisonResult)
    );
}
