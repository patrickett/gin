use ast::{ResultAlternative, ResultFamilyOwner, Ty};
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
fn comparison_accepts_a_structural_proposition() {
    let result = Ty::Proposition(Box::new(ast::ProofProposition::Compare {
        left: ast::ProofTerm::Name(Intern::from_ref("left")),
        relation: ast::ProofRelation::Less,
        right: ast::ProofTerm::Name(Intern::from_ref("right")),
    }));

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
        owner: ResultFamilyOwner::LocalCallable(ast::BinderId::new(
            0,
            ast::BinderOwner::Expression(0),
        )),
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
    let source = format!(
        "{}OtherWord is in 0 to 255\nOtherWord has\n    finish(self):\n        return\nPointerToWord is @Word\nPointerToWord has\n    finish(self):\n        return\nPointerToPointer is @PointerToWord\nPointerToPointer has\n    finish(self):\n        return\nload(address PointerToWord) Word: return address.[0]\nreplace_word(previous Word, next Word) Word: return next\nstore(address PointerToWord, value Word):\n    pointer PointerToWord: address\n    pointer.[0]:: replace_word(pointer.[0], value)\n    return\nload_pointer(address PointerToPointer) PointerToWord: return address.[0]\nreplace_pointer(previous PointerToWord, next PointerToWord) PointerToWord: return next\nstore_pointer(address PointerToPointer, value PointerToWord):\n    pointer PointerToPointer: address\n    pointer.[0]:: replace_pointer(pointer.[0], value)\n    return\n",
        test_fixtures::preludes::WORD_PRELUDE
    );
    let file = TokenCursor::parse_source(&source);
    assert!(file.parse_warnings.is_empty(), "{:?}", file.parse_warnings);
    let mut context = TransformCtx::new();
    context.compile_target =
        CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").expect("target"));
    let typed = transform(&file, FileId(0), &context);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}
