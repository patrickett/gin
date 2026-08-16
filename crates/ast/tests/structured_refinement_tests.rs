use ast::integer::{CanonicalPredicate, IntegerDomain};
use ast::{NormalExpr, PredicateExpr, TargetQueryKind, TypeReference};
use internment::Intern;

#[test]
fn target_query_refinement_remains_structured_in_integer_domain() {
    let refinement = PredicateExpr::Ge(NormalExpr::TargetQuery {
        kind: TargetQueryKind::Alignment,
        operand: TypeReference::Nominal(Intern::from_ref("element")),
    });
    let domain = IntegerDomain::from_refinement(&refinement);
    assert_eq!(
        domain.predicate(),
        &CanonicalPredicate::Structured(refinement)
    );
    assert!(domain.storage_hull().is_none());
}
