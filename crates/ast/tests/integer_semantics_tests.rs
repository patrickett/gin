use std::num::NonZeroU32;

use ast::integer::{
    CanonicalIntegerExpr, CanonicalNaturalExpr, CanonicalPredicate, ContainmentProof,
    IntegerDomain, IntegerKnowledge, IntegerRepresentation, IntegerRepresentationError,
    IntegerRepresentationRule, IntegerValidity, prove_containment, proves_lossless_storage,
    resolve_representation, resolve_runtime_representation,
};
use i256::I256;

fn validity(min: i128, max: i128) -> IntegerValidity {
    IntegerValidity::new(IntegerDomain::bounded(I256::from(min), I256::from(max)).unwrap())
}

fn inferred_width(min: i128, max: i128) -> u32 {
    resolve_representation(
        &validity(min, max),
        &IntegerRepresentationRule::InferFromValidity,
    )
    .unwrap()
    .width()
    .get()
}

#[test]
fn representation_uses_endpoints_instead_of_interval_span() {
    assert_eq!(inferred_width(250, 260), 9);
}

#[test]
fn unsigned_endpoint_width_matrix() {
    assert_eq!(inferred_width(0, 1), 1);
    assert_eq!(inferred_width(0, 2), 2);
    assert_eq!(inferred_width(0, 255), 8);
    assert_eq!(inferred_width(0, 256), 9);
}

#[test]
fn signed_twos_complement_width_matrix() {
    assert_eq!(inferred_width(-1, 0), 1);
    assert_eq!(inferred_width(-1, 1), 2);
    assert_eq!(inferred_width(-1, 127), 8);
    assert_eq!(inferred_width(-1, 128), 9);
    assert_eq!(inferred_width(-1, 255), 9);
    assert_eq!(inferred_width(-1, 256), 10);
}

#[test]
fn hole_predicate_preserves_storage_hull() {
    let domain = IntegerDomain::excluding(
        I256::from(0),
        I256::from(10),
        vec![I256::from(5), I256::from(5), I256::from(20)],
    )
    .unwrap();
    assert!(matches!(
        domain.predicate(),
        CanonicalPredicate::HullExcluding { excluded, .. } if excluded == &[I256::from(5)]
    ));
    assert!(!domain.contains(I256::from(5)));
    assert_eq!(domain.storage_hull().unwrap().min(), I256::from(0));
    assert_eq!(domain.storage_hull().unwrap().max(), I256::from(10));
}

#[test]
fn exact_bits_is_separate_from_validity() {
    let validity = validity(0, 256);
    let representation = resolve_representation(
        &validity,
        &IntegerRepresentationRule::ExactBits(CanonicalNaturalExpr::Value(
            NonZeroU32::new(16).unwrap(),
        )),
    )
    .unwrap();
    assert_eq!(representation.width().get(), 16);
    assert_eq!(
        validity.domain().storage_hull().unwrap().max(),
        I256::from(256)
    );
}

#[test]
fn exact_bits_rejects_a_representation_smaller_than_validity() {
    let error = resolve_representation(
        &validity(0, 256),
        &IntegerRepresentationRule::ExactBits(CanonicalNaturalExpr::Value(
            NonZeroU32::new(8).unwrap(),
        )),
    )
    .unwrap_err();
    assert!(matches!(
        &error,
        IntegerRepresentationError::RepresentationTooSmall {
            requested,
            required
        } if requested.width().get() == 8 && required.width().get() == 9
    ));
    assert_eq!(
        error.diagnostic_code(),
        "type-integer-representation-too-small"
    );
}

#[test]
fn empty_domain_is_rejected_for_runtime_materialization() {
    let validity = IntegerValidity::new(IntegerDomain::empty());
    let error =
        resolve_runtime_representation(&validity, &IntegerRepresentationRule::InferFromValidity)
            .unwrap_err();
    assert_eq!(error, IntegerRepresentationError::EmptyDomain);
    assert_eq!(
        error.diagnostic_code(),
        "type-integer-representation-width-invalid"
    );
}

#[test]
fn unbounded_domain_requires_finite_runtime_bounds() {
    let validity = IntegerValidity::new(IntegerDomain::unbounded());
    let error =
        resolve_runtime_representation(&validity, &IntegerRepresentationRule::InferFromValidity)
            .unwrap_err();

    assert_eq!(error, IntegerRepresentationError::UnboundedDomain);
    assert_eq!(
        error.diagnostic_code(),
        "type-integer-refinement-requires-finite-bounds"
    );
}

#[test]
fn containment_proves_bounded_subset() {
    let actual = IntegerDomain::bounded(I256::from(2), I256::from(8)).unwrap();
    let expected = IntegerDomain::bounded(I256::from(0), I256::from(10)).unwrap();

    assert_eq!(
        prove_containment(&actual, &expected),
        ContainmentProof::Proven
    );
}

#[test]
fn containment_disproves_value_outside_expected_domain() {
    let actual = IntegerDomain::bounded(I256::from(2), I256::from(12)).unwrap();
    let expected = IntegerDomain::bounded(I256::from(0), I256::from(10)).unwrap();
    let proof = prove_containment(&actual, &expected);

    assert_eq!(proof, ContainmentProof::Disproven);
    assert_eq!(
        proof.diagnostic_code(),
        Some("type-integer-narrowing-failed")
    );
}

#[test]
fn containment_reports_symbolic_proof_as_unknown() {
    let actual = IntegerDomain::symbolic(ast::NormalExpr::Var(internment::Intern::from_ref("n")));
    let expected = IntegerDomain::bounded(I256::from(0), I256::from(10)).unwrap();
    let proof = prove_containment(&actual, &expected);

    assert_eq!(proof, ContainmentProof::Unknown);
    assert_eq!(
        proof.diagnostic_code(),
        Some("type-integer-narrowing-unproven")
    );
}

#[test]
fn exact_knowledge_proves_lossless_storage_reduction() {
    let knowledge = IntegerKnowledge::Exact(CanonicalIntegerExpr::Value(I256::from(15)));
    let target = IntegerRepresentation::new(NonZeroU32::new(4).unwrap());

    assert!(proves_lossless_storage(&knowledge, target));
}

#[test]
fn unknown_knowledge_cannot_prove_lossless_storage_reduction() {
    let target = IntegerRepresentation::new(NonZeroU32::new(8).unwrap());

    assert!(!proves_lossless_storage(&IntegerKnowledge::Unknown, target));
}

#[test]
fn wide_compile_time_domain_retains_i256_endpoints() {
    let max = (I256::from(1) << 199) - I256::from(1);
    let validity = IntegerValidity::new(IntegerDomain::bounded(I256::from(0), max).unwrap());
    let representation =
        resolve_representation(&validity, &IntegerRepresentationRule::InferFromValidity).unwrap();
    assert_eq!(representation.width().get(), 199);
    assert!(!representation.is_runtime_supported());
}

#[test]
fn wide_compile_time_domain_is_rejected_for_runtime_materialization() {
    let max = (I256::from(1) << 199) - I256::from(1);
    let validity = IntegerValidity::new(IntegerDomain::bounded(I256::from(0), max).unwrap());
    let error =
        resolve_runtime_representation(&validity, &IntegerRepresentationRule::InferFromValidity)
            .unwrap_err();
    assert!(matches!(
        &error,
        IntegerRepresentationError::RuntimeWidthUnsupported(representation)
            if representation.width().get() == 199
    ));
    assert_eq!(
        error.diagnostic_code(),
        "type-integer-runtime-width-unsupported"
    );
}
