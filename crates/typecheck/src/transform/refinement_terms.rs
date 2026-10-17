use diagnostic::code::{MemberCode, ProofCode};
use std::collections::HashMap;
use std::ops::ControlFlow;

use ast::{ConstValue, NormalExpr, Ty, TyArg};
use diagnostic::Diagnostic;
use internment::Intern;

use super::callable_signatures::CallableSignatureScope;
use super::proof_walk::read::{ProofVisitor, walk_normal, walk_term};
use super::symbolic_values::field_symbol;
use crate::typed::TypedFileAst;

fn project(value: &NormalExpr, index: usize) -> Option<NormalExpr> {
    match value {
        NormalExpr::Var(root) => Some(field_symbol(*root, index)),
        NormalExpr::Value(ConstValue::Record { fields }) => fields
            .get(index)
            .map(|(_, value)| NormalExpr::Value(value.clone())),
        _ => None,
    }
}

fn instantiate(
    expression: &NormalExpr,
    receiver: Intern<String>,
    value: &NormalExpr,
) -> Option<NormalExpr> {
    match expression {
        NormalExpr::Var(name) if *name == receiver => Some(value.clone()),
        NormalExpr::Var(name) => {
            let suffix = name.as_str().strip_prefix(&format!("{receiver}.$field"))?;
            suffix
                .split(".$field")
                .try_fold(value.clone(), |value, index| {
                    project(&value, index.parse().ok()?)
                })
        }
        NormalExpr::Add(left, right)
        | NormalExpr::Sub(left, right)
        | NormalExpr::Mul(left, right) => {
            let left = Box::new(instantiate(left, receiver, value)?);
            let right = Box::new(instantiate(right, receiver, value)?);
            Some(match expression {
                NormalExpr::Add(_, _) => NormalExpr::Add(left, right),
                NormalExpr::Sub(_, _) => NormalExpr::Sub(left, right),
                _ => NormalExpr::Mul(left, right),
            })
        }
        NormalExpr::Value(ConstValue::Int(_)) | NormalExpr::TargetQuery { .. } => {
            Some(expression.clone())
        }
        _ => None,
    }
}

pub(super) fn resolve(
    typed: &TypedFileAst,
    signatures: &CallableSignatureScope<'_>,
    types: &HashMap<Intern<String>, Ty>,
    values: &HashMap<Intern<String>, NormalExpr>,
    name: Intern<String>,
) -> Result<(Ty, Option<NormalExpr>), Diagnostic> {
    let unknown = || {
        // type-constraint-unknown-name
        Diagnostic::new(
            ProofCode::UnknownName,
            format!("constraint refers to unknown value or member `{name}`"),
        )
        .with_arg("name", name.as_str().to_string())
    };
    if let Some(ty) = types.get(&name) {
        return Ok((
            ty.clone(),
            Some(values.get(&name).cloned().unwrap_or(NormalExpr::Var(name))),
        ));
    }
    let mut parts = name.as_str().split('.');
    let root = Intern::from_ref(parts.next().ok_or_else(unknown)?);
    let mut ty = types.get(&root).cloned().ok_or_else(unknown)?;
    let mut value = Some(values.get(&root).cloned().unwrap_or(NormalExpr::Var(root)));
    for member in parts {
        let member = Intern::from_ref(member);
        if !typed.can_access_member(&ty, member) {
            // type-private-member
            return Err(Diagnostic::new(
                MemberCode::Private,
                format!("cannot access private member `{member}` in a constraint"),
            ));
        }
        if let Ty::Record { fields, .. } = typed
            .type_registry
            .resolved_definition_for_type(ty.without_reference_wrappers())
            && let Some((index, (_, field_ty))) = fields
                .iter()
                .enumerate()
                .find(|(_, (field, _))| *field == member)
        {
            value = value.as_ref().and_then(|value| project(value, index));
            ty = field_ty.as_ref().clone();
            continue;
        }
        let Some(owner) = ty.named_instance_stripping_reference_wrappers() else {
            return Err(unknown());
        };
        let candidates = signatures.iter().filter(|(id, signature)| {
            id.0.as_str()
                .split("$specialized$")
                .next()
                .and_then(|name| name.rsplit('.').next())
                == Some(member.as_str())
                && signature.params.len() == 1
                && signature.param_conventions.first() == Some(&ast::ParamConvention::Observe)
                && signature.params[0]
                    .1
                    .named_instance_stripping_reference_wrappers()
                    .is_some_and(|instance| instance.declaration == owner.declaration)
        });
        let mut candidates: Vec<_> = candidates.collect();
        let exact = candidates.iter().any(|(_, signature)| {
            signature.params[0].1.without_reference_wrappers() == ty.without_reference_wrappers()
        });
        if exact {
            candidates.retain(|(_, signature)| {
                signature.params[0].1.without_reference_wrappers()
                    == ty.without_reference_wrappers()
            });
        } else {
            candidates.retain(|(id, _)| !id.0.contains("$specialized$"));
        }
        let (id, signature) = candidates.first().copied().ok_or_else(unknown)?;
        if candidates.len() > 1 {
            // type-constraint-ambiguous-member
            return Err(Diagnostic::new(
                ProofCode::AmbiguousMember,
                format!("constraint method `{member}` is ambiguous"),
            ));
        }
        if signature.param_refinements.iter().any(Option::is_some) {
            // type-constraint-unproven-term
            return Err(Diagnostic::new(
                ProofCode::UnprovenTerm,
                format!("constraint method `{member}` requires its own precondition"),
            ));
        }
        if !typed.can_access_method(*id) {
            // type-private-member
            return Err(Diagnostic::new(
                MemberCode::Private,
                format!("cannot access private method `{member}` in a constraint"),
            ));
        }
        if !signature.effects.writes.is_empty() || signature.effects.unknown_memory {
            // type-constraint-impure
            return Err(Diagnostic::new(
                ProofCode::Impure,
                format!("constraint method `{member}` must be pure"),
            ));
        }
        value = signature
            .effects
            .scalar_result
            .as_ref()
            .zip(value.as_ref())
            .and_then(|(scalar, value)| instantiate(scalar, signature.params[0].0, value));
        let substitution = crate::analysis::unify_type_args_with_registry(
            &[TyArg::Type(Box::new(
                signature.params[0].1.without_reference_wrappers().clone(),
            ))],
            &[TyArg::Type(Box::new(
                ty.without_reference_wrappers().clone(),
            ))],
            Some(&typed.type_registry),
        )
        .map_err(|_| unknown())?;
        ty = substitution.apply_to_ty(&signature.return_type);
    }
    Ok((ty, value))
}

pub(super) fn predicate_names(predicate: &ast::PredicateExpr) -> Vec<Intern<String>> {
    struct Collector(Vec<Intern<String>>);

    impl ProofVisitor for Collector {
        fn visit_normal(&mut self, expr: &NormalExpr) -> ControlFlow<()> {
            if let NormalExpr::Var(name) = expr {
                self.0.push(*name);
            }
            walk_normal(self, expr)
        }

        fn visit_term(&mut self, term: &ast::ProofTerm) -> ControlFlow<()> {
            if let ast::ProofTerm::Name(name) = term {
                self.0.push(*name);
            }
            walk_term(self, term)
        }
    }

    let mut collector = Collector(Vec::new());
    let _ = collector.visit_predicate(predicate);
    let mut names = collector.0;
    names.sort();
    names.dedup();
    names
}

pub(super) fn constant_value(
    typed: &TypedFileAst,
    name: Intern<String>,
    constants: &super::constant_queries::ConstantQueries,
) -> Option<ConstValue> {
    let id = crate::typed::DefId::named(name);
    if let Some(bind) = typed.defs.get(&id)
        && bind.is_constant
        && bind.params.is_empty()
        && let crate::typed::BindBody::Expr(expression) = bind.body
        && let Some(value) = typed.exprs.const_value_of(expression).cloned().flatten()
    {
        return Some(value);
    }
    if typed
        .eval_ast
        .defs
        .get(&id)
        .is_some_and(|bind| bind.params.is_none() && bind.is_constant())
    {
        return constants.value(id);
    }
    None
}

pub(super) fn extend_values(
    typed: &TypedFileAst,
    signatures: &CallableSignatureScope<'_>,
    types: &HashMap<Intern<String>, Ty>,
    values: &mut HashMap<Intern<String>, NormalExpr>,
    predicate: &ast::PredicateExpr,
) {
    let constants = super::constant_queries::ConstantQueries::for_layout(
        std::sync::Arc::clone(&typed.eval_ast),
        typed.target_layout,
    );
    for name in predicate_names(predicate) {
        let root = Intern::from_ref(name.as_str().split('.').next().unwrap_or(name.as_str()));
        if !types.contains_key(&root)
            && let Some(value) = constant_value(typed, name, &constants)
        {
            values.insert(name, NormalExpr::Value(value));
            continue;
        }
        if !types.contains_key(&root)
            && typed
                .eval_ast
                .defs
                .get(&crate::DefId::named(name))
                .is_some_and(|bind| bind.params.is_none() && bind.is_constant())
        {
            values.entry(name).or_insert(NormalExpr::Var(name));
            continue;
        }
        if name.as_str().contains('.')
            && let Ok((_, Some(value))) = resolve(typed, signatures, types, values, name)
        {
            values.insert(name, value);
        }
    }
}

pub(super) fn constant_bounds(
    typed: &TypedFileAst,
    name: Intern<String>,
) -> Option<(i256::I256, i256::I256)> {
    use crate::layout::{LayoutTarget, TargetLayout};
    let mut bounds: Option<(i256::I256, i256::I256)> = None;
    for layout in [
        LayoutTarget::X86_64,
        LayoutTarget::Arm64,
        LayoutTarget::Wasm32,
    ] {
        let constants = super::constant_queries::ConstantQueries::for_layout(
            std::sync::Arc::clone(&typed.eval_ast),
            Some(TargetLayout::new(layout)),
        );
        let ConstValue::Int(value) = constant_value(typed, name, &constants)? else {
            return None;
        };
        bounds = Some(bounds.map_or((value, value), |(low, high)| {
            (low.min(value), high.max(value))
        }));
    }
    bounds
}

pub(super) fn values_complete(
    predicate: &ast::PredicateExpr,
    values: &HashMap<Intern<String>, NormalExpr>,
) -> bool {
    predicate_names(predicate)
        .iter()
        .all(|name| !name.as_str().contains('.') || values.contains_key(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(value: &str) -> Intern<String> {
        Intern::from_ref(value)
    }

    fn record(fields: Vec<(&str, NormalExpr)>) -> NormalExpr {
        NormalExpr::Value(ConstValue::Record {
            fields: fields
                .into_iter()
                .map(|(field, value)| {
                    let NormalExpr::Value(value) = value else {
                        panic!("constant field")
                    };
                    (name(field), value)
                })
                .collect(),
        })
    }

    #[test]
    fn receiver_substitution_projects_nested_constants_and_symbolic_fields() {
        let value = record(vec![(
            "nested",
            record(vec![("first", 7_i128.into()), ("second", 11_i128.into())]),
        )]);
        let term = NormalExpr::Var(name("self.$field0.$field1"));
        assert_eq!(
            instantiate(&term, name("self"), &value),
            Some(11_i128.into())
        );
        assert_eq!(
            instantiate(&term, name("self"), &NormalExpr::Var(name("actual"))),
            Some(field_symbol(name("actual.$field0"), 1))
        );
        assert_eq!(
            instantiate(&NormalExpr::Var(name("self")), name("self"), &value),
            Some(value)
        );
    }

    #[test]
    fn unsupported_receiver_paths_remain_unknown() {
        let value = record(vec![("only", 7_i128.into())]);
        for variable in [
            "other",
            "self.$field1",
            "self.$field0.$field0",
            "self.$fieldbad",
            "self.$field",
        ] {
            assert_eq!(
                instantiate(&NormalExpr::Var(name(variable)), name("self"), &value),
                None,
                "{variable}"
            );
        }
        assert_eq!(project(&3_i128.into(), 0), None);
    }

    #[test]
    fn arithmetic_substitution_requires_every_operand() {
        let receiver = name("self");
        let field = NormalExpr::Var(name("self.$field0"));
        let value = record(vec![("only", 7_i128.into())]);
        for (term, expected) in [
            (
                NormalExpr::Add(Box::new(field.clone()), Box::new(2_i128.into())),
                NormalExpr::Add(Box::new(7_i128.into()), Box::new(2_i128.into())),
            ),
            (
                NormalExpr::Sub(Box::new(field.clone()), Box::new(2_i128.into())),
                NormalExpr::Sub(Box::new(7_i128.into()), Box::new(2_i128.into())),
            ),
            (
                NormalExpr::Mul(Box::new(field), Box::new(2_i128.into())),
                NormalExpr::Mul(Box::new(7_i128.into()), Box::new(2_i128.into())),
            ),
        ] {
            assert_eq!(instantiate(&term, receiver, &value), Some(expected));
        }
        let term = NormalExpr::Add(
            Box::new(NormalExpr::Var(receiver)),
            Box::new(NormalExpr::Var(name("missing"))),
        );
        assert_eq!(instantiate(&term, receiver, &value), None);
    }

    #[test]
    fn predicate_dependencies_include_nested_terms_and_are_unique() {
        use ast::{PredicateExpr, ProofProposition, ProofRelation, ProofTerm};
        let predicate = PredicateExpr::And(vec![
            PredicateExpr::Eq(NormalExpr::Add(
                Box::new(NormalExpr::Var(name("z"))),
                Box::new(NormalExpr::Var(name("a"))),
            )),
            PredicateExpr::Proposition(Box::new(ProofProposition::Not(Box::new(
                ProofProposition::Compare {
                    relation: ProofRelation::Equal,
                    left: ProofTerm::Remainder(
                        Box::new(ProofTerm::Name(name("a"))),
                        Box::new(ProofTerm::Name(name("receiver.count"))),
                    ),
                    right: ProofTerm::PowerOfTwo(Box::new(ProofTerm::Name(name("exponent")))),
                },
            )))),
        ]);
        let mut expected = vec![
            name("a"),
            name("exponent"),
            name("receiver.count"),
            name("z"),
        ];
        expected.sort();
        assert_eq!(predicate_names(&predicate), expected);
        let mut values = HashMap::new();
        assert!(!values_complete(&predicate, &values));
        values.insert(name("receiver.count"), 7_i128.into());
        assert!(values_complete(&predicate, &values));
    }
}
