use super::*;
use crate::ty::PredicateExpr;
use internment::Intern;

fn interval(min: i128, max: i128) -> IntegerDomain {
    IntegerDomain::bounded(min.into(), max.into()).unwrap()
}

#[test]
fn containment_does_not_fill_union_gaps() {
    let target = IntegerDomain::union([interval(-4, -2), interval(2, 4)]);
    assert_eq!(
        interval(-1, 1).prove_containment(&target),
        ContainmentProof::Disproven
    );
    assert_eq!(
        interval(2, 3).prove_containment(&target),
        ContainmentProof::Proven
    );
}

#[test]
fn adjacent_union_branches_cover_a_continuous_source() {
    let target = IntegerDomain::union([interval(3, 4), interval(1, 2)]);
    assert_eq!(
        interval(1, 4).prove_containment(&target),
        ContainmentProof::Proven
    );
}

#[test]
fn unknown_source_membership_does_not_prove_an_exclusion() {
    let uncertain = IntegerDomain::from_refinement(&PredicateExpr::And(vec![
        PredicateExpr::Ge(1_i128.into()),
        PredicateExpr::Le(4_i128.into()),
        PredicateExpr::Ne(NormalExpr::Var(Intern::from_ref("unknown"))),
    ]));
    let actual = IntegerDomain::union([uncertain, interval(8, 8)]);
    let expected = IntegerDomain::excluding(1.into(), 8.into(), vec![2.into()]).unwrap();
    assert_eq!(actual.membership(2.into()), ContainmentProof::Unknown);
    assert_eq!(
        actual.prove_containment(&expected),
        ContainmentProof::Unknown
    );
}

#[test]
fn uncertain_singletons_do_not_supply_a_definite_counterexample() {
    let actual = IntegerDomain::from_refinement(&PredicateExpr::And(vec![
        PredicateExpr::Ge(1_i128.into()),
        PredicateExpr::Le(1_i128.into()),
        PredicateExpr::Ne(NormalExpr::Var(Intern::from_ref("unknown"))),
    ]));
    assert_eq!(
        actual.prove_containment(&interval(2, 2)),
        ContainmentProof::Unknown
    );
}

#[test]
fn excluded_endpoints_do_not_prevent_valid_narrowing() {
    let actual = IntegerDomain::excluding(1.into(), 4.into(), vec![1.into(), 4.into()]).unwrap();
    assert_eq!(
        actual.prove_containment(&interval(2, 3)),
        ContainmentProof::Proven
    );
}

#[test]
fn excluding_every_value_makes_containment_vacuous() {
    let actual = IntegerDomain::excluding(1.into(), 2.into(), vec![1.into(), 2.into()]).unwrap();
    assert_eq!(
        actual.prove_containment(&IntegerDomain::empty()),
        ContainmentProof::Proven
    );
}

#[test]
fn finite_subset_proofs_match_exhaustive_membership() {
    let domain = |mask: u32| {
        IntegerDomain::union(
            (-2_i128..=2)
                .enumerate()
                .filter(|(index, _)| mask & (1 << index) != 0)
                .map(|(_, value)| interval(value, value)),
        )
    };
    for actual in 0_u32..32 {
        for expected in 0_u32..32 {
            let proof = if actual & !expected == 0 {
                ContainmentProof::Proven
            } else {
                ContainmentProof::Disproven
            };
            assert_eq!(
                domain(actual).prove_containment(&domain(expected)),
                proof,
                "actual={actual:05b}, expected={expected:05b}"
            );
        }
    }
}

#[test]
fn exclusions_at_integer_limits_are_safe() {
    let actual =
        IntegerDomain::excluding(I256::MIN, I256::MAX, vec![I256::MIN, I256::MAX]).unwrap();
    let expected =
        IntegerDomain::bounded(I256::MIN + I256::from(1), I256::MAX - I256::from(1)).unwrap();
    assert_eq!(
        actual.prove_containment(&expected),
        ContainmentProof::Proven
    );
}

#[test]
fn unbounded_domains_are_not_capped_at_i256_limits() {
    let bounded = IntegerDomain::bounded(I256::MIN, I256::MAX).unwrap();
    assert_eq!(
        IntegerDomain::unbounded().prove_containment(&bounded),
        ContainmentProof::Disproven
    );
    let above = IntegerDomain::from_refinement(&PredicateExpr::Gt(I256::MAX.into()));
    let below = IntegerDomain::from_refinement(&PredicateExpr::Lt(I256::MIN.into()));
    assert_eq!(
        above.prove_containment(&bounded),
        ContainmentProof::Disproven
    );
    assert_eq!(
        below.prove_containment(&bounded),
        ContainmentProof::Disproven
    );
}

#[test]
fn opaque_clauses_do_not_hide_known_later_bounds() {
    let domain = IntegerDomain::from_refinement(&PredicateExpr::And(vec![
        PredicateExpr::Ne(NormalExpr::Var(Intern::from_ref("unknown"))),
        PredicateExpr::Ge(1_i128.into()),
        PredicateExpr::Le(4_i128.into()),
    ]));
    let hull = domain.storage_hull().expect("known finite bounds");
    assert_eq!((hull.min(), hull.max()), (1.into(), 4.into()));
    assert_eq!(
        domain.prove_containment(&interval(0, 5)),
        ContainmentProof::Proven
    );
}
