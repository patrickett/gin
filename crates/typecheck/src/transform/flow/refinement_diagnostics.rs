use ast::{PredicateExpr, ProofProposition};
use diagnostic::Diagnostic;
use diagnostic::code::ProofCode;

fn condition(subject: &str, refinement: &PredicateExpr) -> String {
    const NODES_MAX: usize = 128;
    let mut pending = vec![refinement];
    let mut conditions = Vec::new();
    for _ in 0..NODES_MAX {
        let Some(predicate) = pending.pop() else {
            break;
        };
        let (relation, right) = match predicate {
            PredicateExpr::Lt(right) => ("<", right),
            PredicateExpr::Gt(right) => (">", right),
            PredicateExpr::Le(right) => ("<=", right),
            PredicateExpr::Ge(right) => (">=", right),
            PredicateExpr::Eq(right) => ("=", right),
            PredicateExpr::Ne(right) => ("!=", right),
            PredicateExpr::And(predicates) => {
                pending.extend(predicates.iter().rev());
                continue;
            }
            PredicateExpr::Proposition(proposition) => {
                conditions.push(format!("({proposition})"));
                continue;
            }
        };
        conditions.push(format!("{subject} {relation} {right}"));
    }
    if !pending.is_empty() {
        conditions.push("...".to_string());
    }
    conditions.join(" and ")
}

pub(super) fn argument(name: &str, refinement: &PredicateExpr) -> Diagnostic {
    let condition = condition(name, refinement);
    // type-parameter-refinement-unproven
    Diagnostic::new(
        ProofCode::ParameterRefinementUnproven,
        format!("cannot prove refinement `{condition}` for argument `{name}`"),
    )
    .with_arg("name", name)
    .with_help(
        "check this condition before the call, or add it to the caller's parameter refinement; \
         check again after changing any value the condition depends on",
    )
}

pub(super) fn construction(name: &str, refinement: &PredicateExpr) -> Diagnostic {
    let condition = condition(name, refinement);
    // dep-refinement-unproven
    Diagnostic::new(
        ProofCode::DependentRefinementUnproven,
        format!("cannot prove refinement `{condition}` for field `{name}`"),
    )
    .with_help(
        "check this condition on the field values before constructing the record, \
         or add it to the caller's parameter refinements",
    )
}

pub(super) fn replacement(changed: &str, name: &str, refinement: &PredicateExpr) -> Diagnostic {
    let condition = condition(name, refinement);
    // dep-refinement-unproven
    Diagnostic::new(
        ProofCode::DependentRefinementUnproven,
        format!("cannot prove refinement `{condition}` after replacement of `{changed}`"),
    )
    .with_arg("name", name)
    .with_help(
        "check this condition using the replacement value and the current record fields \
         before replacing the field; check again after changing any dependency",
    )
}

pub(super) fn postcondition(proposition: &ProofProposition) -> Diagnostic {
    // type-postcondition-unproven
    Diagnostic::new(
        ProofCode::PostconditionUnproven,
        format!("cannot prove return postcondition `{proposition}`"),
    )
    .with_help(
        "establish this condition before returning, or add a required input condition to the \
         function's parameter refinements; check the current values after any mutation",
    )
}
