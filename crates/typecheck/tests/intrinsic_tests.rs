use ast::{OperatorRole, ResultAlternative, ResultFamilyOwner, Ty};
use flask::{CompileTarget, TargetTriple};
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::FileId;
use typecheck::intrinsic::{
    ComparisonPredicate, IntegerComparison, IntrinsicOp, IntrinsicValidationError,
};
use typecheck::transform::{TransformCtx, transform};

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

#[test]
fn address_store_requires_unit_result() {
    let pointee = Ty::anonymous_integer_for_width(8, false);
    let address = Ty::Address {
        pointee: Box::new(pointee.clone()),
        address_space: 0,
    };
    assert_eq!(
        IntrinsicOp::AddressStore.validate_signature(
            &[address, pointee],
            &Ty::anonymous_integer_for_width(8, false),
            None
        ),
        Err(IntrinsicValidationError::InvalidAddressStoreResult)
    );
}

#[test]
fn address_store_rejects_product_result() {
    let pointee = Ty::anonymous_integer_for_width(8, false);
    let address = Ty::Address {
        pointee: Box::new(pointee.clone()),
        address_space: 0,
    };
    let product = Ty::Record {
        name: Intern::from_ref("Pair"),
        fields: vec![(Intern::from_ref("value"), Box::new(pointee.clone()))],
        resolved_params: None,
    };
    assert_eq!(
        IntrinsicOp::AddressStore.validate_signature(&[address, pointee], &product, None),
        Err(IntrinsicValidationError::InvalidAddressStoreResult)
    );
}

#[test]
fn address_store_accepts_unit_and_rejects_pointee_mismatch() {
    let pointee = Ty::anonymous_integer_for_width(8, false);
    let address = Ty::Address {
        pointee: Box::new(pointee.clone()),
        address_space: 0,
    };
    assert_eq!(
        IntrinsicOp::AddressStore.validate_signature(&[address.clone(), pointee], &Ty::Unit, None),
        Ok(())
    );
    assert_eq!(
        IntrinsicOp::AddressStore.validate_signature(
            &[address, Ty::anonymous_integer_for_width(16, false)],
            &Ty::Unit,
            None
        ),
        Err(IntrinsicValidationError::PointeeMismatch)
    );
}

#[test]
fn ordinary_pointer_loads_and_stores_accept_named_pointees() {
    let source = "Word is in 0 to 255\nOtherWord is in 0 to 255\nPointerToWord is @Word\nPointerToPointer is @PointerToWord\nload(address PointerToWord) Word: address.(0)\nreplace_word(previous Word, next Word) Word: next\nstore(address PointerToWord, value Word):\n    pointer PointerToWord: address\n    pointer.(0):: replace_word(pointer.(0), value)\nreturn\nload_pointer(address PointerToPointer) PointerToWord: address.(0)\nreplace_pointer(previous PointerToWord, next PointerToWord) PointerToWord: next\nstore_pointer(address PointerToPointer, value PointerToWord):\n    pointer PointerToPointer: address\n    pointer.(0):: replace_pointer(pointer.(0), value)\nreturn\n";
    let file = TokenCursor::parse_source(source);
    assert!(file.parse_warnings.is_empty(), "{:?}", file.parse_warnings);
    let mut context = TransformCtx::new();
    context.compile_target =
        CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").expect("target"));
    let typed = transform(&file, FileId(0), &context);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}
