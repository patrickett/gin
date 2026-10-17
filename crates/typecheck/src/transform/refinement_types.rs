use diagnostic::code::ProofCode;
use std::collections::{HashMap, HashSet};

use ast::{ConstValue, FileAst, NormalExpr, PredicateExpr, ProofProposition, ProofTerm, Ty};
use diagnostic::Diagnostic;
use internment::Intern;

use super::callable_signatures::CallableSignatureScope;
use crate::typed::{DefId, TagId, TypedCallableSignature, TypedFileAst};

struct OperandTypes<'a, 'scope> {
    typed: &'a TypedFileAst,
    signatures: &'a CallableSignatureScope<'scope>,
    names: HashMap<Intern<String>, Ty>,
}

fn array_length_names(names: &mut HashMap<Intern<String>, Ty>) {
    let mut pending: Vec<_> = names.values().cloned().collect();
    while let Some(ty) = pending.pop() {
        match ty {
            Ty::Array { elem, size } => {
                let mut variables = HashSet::new();
                super::scalar_mutations::variables(&size, &mut variables);
                for name in variables {
                    names
                        .entry(name)
                        .or_insert(Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer));
                }
                pending.push(*elem);
            }
            Ty::Ref { inner, .. } => pending.push(*inner),
            _ => {}
        }
    }
}

impl OperandTypes<'_, '_> {
    fn numeric(&self, ty: &Ty) -> Result<(), Diagnostic> {
        let ty = ty.without_reference_wrappers();
        if matches!(
            ty,
            Ty::Literal(ConstValue::Int(_)) | Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer)
        ) || self
            .typed
            .type_registry
            .integer_validity_for_type(ty)
            .is_some()
        {
            Ok(())
        } else {
            // type-constraint-noninteger
            Err(Diagnostic::new(
                ProofCode::NonInteger,
                "numeric constraints require integer operands",
            ))
        }
    }

    fn name(&self, name: Intern<String>) -> Result<Option<Ty>, Diagnostic> {
        let (ty, _) = super::refinement_terms::resolve(
            self.typed,
            self.signatures,
            &self.names,
            &HashMap::new(),
            name,
        )?;
        self.numeric(&ty)?;
        Ok(Some(ty.without_reference_wrappers().clone()))
    }

    fn compatible(&self, left: Option<Ty>, right: Option<Ty>) -> Result<Option<Ty>, Diagnostic> {
        if let (Some(left), Some(right)) = (&left, &right)
            && let (Some(left), Some(right)) = (left.named_instance(), right.named_instance())
            && (left.declaration != right.declaration || left.arguments != right.arguments)
        {
            // type-constraint-type-mismatch
            return Err(Diagnostic::new(
                ProofCode::TypeMismatch,
                "constraint operands have incompatible nominal types or type arguments",
            ));
        }
        Ok(left.or(right))
    }

    fn normal(&self, expr: &NormalExpr) -> Result<Option<Ty>, Diagnostic> {
        match expr {
            NormalExpr::Value(ConstValue::Int(_))
            | NormalExpr::Inferred(_)
            | NormalExpr::TargetQuery { .. } => Ok(None),
            NormalExpr::Var(name) => self.name(*name),
            NormalExpr::Add(left, right)
            | NormalExpr::Sub(left, right)
            | NormalExpr::Mul(left, right) => {
                self.compatible(self.normal(left)?, self.normal(right)?)
            }
            // type-constraint-noninteger
            NormalExpr::Value(_) => Err(Diagnostic::new(
                ProofCode::NonInteger,
                "numeric constraints require integer operands",
            )),
        }
    }

    fn term(&self, term: &ProofTerm) -> Result<Option<Ty>, Diagnostic> {
        match term {
            ProofTerm::Value(_) | ProofTerm::TargetQuery { .. } => Ok(None),
            ProofTerm::Name(name) => self.name(*name),
            ProofTerm::Add(left, right)
            | ProofTerm::Sub(left, right)
            | ProofTerm::Mul(left, right)
            | ProofTerm::Remainder(left, right) => {
                self.compatible(self.term(left)?, self.term(right)?)
            }
            ProofTerm::PowerOfTwo(exponent) => self.term(exponent),
        }
    }

    fn proposition(&self, proposition: &ProofProposition) -> Result<(), Diagnostic> {
        match proposition {
            ProofProposition::Compare { left, right, .. } => {
                self.compatible(self.term(left)?, self.term(right)?)?;
            }
            ProofProposition::InRange { value, start, end } => {
                let value = self.term(value)?;
                self.compatible(value.clone(), self.term(start)?)?;
                self.compatible(value, self.term(end)?)?;
            }
            ProofProposition::Not(inner) => self.proposition(inner)?,
            ProofProposition::And(left, right) | ProofProposition::Or(left, right) => {
                self.proposition(left)?;
                self.proposition(right)?;
            }
        }
        Ok(())
    }

    fn predicate(&self, subject: &Ty, predicate: &PredicateExpr) -> Result<(), Diagnostic> {
        match predicate {
            PredicateExpr::Lt(right)
            | PredicateExpr::Gt(right)
            | PredicateExpr::Le(right)
            | PredicateExpr::Ge(right)
            | PredicateExpr::Eq(right)
            | PredicateExpr::Ne(right) => {
                self.numeric(subject)?;
                self.compatible(
                    Some(subject.without_reference_wrappers().clone()),
                    self.normal(right)?,
                )?;
            }
            PredicateExpr::And(predicates) => {
                for predicate in predicates {
                    self.predicate(subject, predicate)?;
                }
            }
            PredicateExpr::Proposition(proposition) => self.proposition(proposition)?,
        }
        Ok(())
    }
}

pub(super) fn validate_proposition(
    typed: &TypedFileAst,
    signatures: &CallableSignatureScope<'_>,
    names: &HashMap<Intern<String>, Ty>,
    proposition: &ProofProposition,
) -> Result<(), Diagnostic> {
    let mut names = names.clone();
    array_length_names(&mut names);
    let constants = super::constant_queries::ConstantQueries::for_layout(
        std::sync::Arc::clone(&typed.eval_ast),
        typed.target_layout,
    );
    for name in super::refinement_terms::predicate_names(&PredicateExpr::Proposition(Box::new(
        proposition.clone(),
    ))) {
        if !names.contains_key(&name)
            && let Some(value) = super::refinement_terms::constant_value(typed, name, &constants)
        {
            names.insert(name, Ty::Literal(value));
        }
    }
    OperandTypes {
        typed,
        signatures,
        names,
    }
    .proposition(proposition)
}

pub(super) fn validate(
    typed: &mut TypedFileAst,
    source: &FileAst,
    shared: &HashMap<DefId, TypedCallableSignature>,
) {
    let _profile = compiler_profile::span(compiler_profile::Phase::Refinements);
    let signatures = CallableSignatureScope::new(
        shared,
        typed
            .defs
            .iter()
            .map(|(id, bind)| (*id, TypedCallableSignature::from(bind))),
    );
    let mut dependencies: HashSet<_> = typed
        .defs
        .values()
        .flat_map(|bind| bind.param_refinements.iter().flatten())
        .flat_map(super::refinement_terms::predicate_names)
        .collect();
    for declaration in source.tags.values() {
        if let ast::DeclareValue::Has(members) = &declaration.value {
            for member in members {
                if let ast::HasMember::Property(field) = member
                    && let Some(predicate) = &field.refinement
                {
                    dependencies.extend(super::refinement_terms::predicate_names(predicate));
                }
            }
        }
    }
    let constants = super::constant_queries::ConstantQueries::for_layout(
        std::sync::Arc::clone(&typed.eval_ast),
        typed.target_layout,
    );
    let infer = |id: DefId, bind: &ast::Bind, ty: &Ty| {
        if *ty == Ty::Unit && bind.is_constant() && dependencies.contains(&id.0) {
            constants
                .value(id)
                .map(Ty::Literal)
                .unwrap_or_else(|| ty.clone())
        } else {
            ty.clone()
        }
    };
    let mut globals: HashMap<_, _> = typed
        .eval_ast
        .defs
        .iter()
        .filter(|(_, bind)| bind.params.is_none())
        .filter_map(|(id, bind)| {
            typed
                .fn_return_types
                .get(id)
                .map(|ty| (id.0, infer(*id, bind, ty)))
        })
        .collect();
    for (id, bind) in &typed.defs {
        if !bind.is_callable {
            let ty = source
                .defs
                .get(id)
                .map(|source| infer(*id, source, &bind.return_type))
                .unwrap_or_else(|| bind.return_type.clone());
            globals.insert(bind.name, ty);
        }
    }
    let mut diagnostics = Vec::new();
    for (name, declaration) in &source.tags {
        let Some(tag) = typed.tags.get(&TagId(*name)) else {
            continue;
        };
        let Ty::Record { fields, .. } = typed
            .type_registry
            .resolved_definition_for_type(&tag.resolved_ty)
        else {
            continue;
        };
        let ast::DeclareValue::Has(members) = &declaration.value else {
            continue;
        };
        let mut names = globals.clone();
        names.extend(fields.iter().map(|(name, ty)| (*name, ty.as_ref().clone())));
        for member in members {
            let ast::HasMember::Property(field) = member else {
                continue;
            };
            let Some(predicate) = &field.refinement else {
                continue;
            };
            if field
                .qualifier
                .as_ref()
                .is_some_and(|qualifier| qualifier.name != *name)
            {
                continue;
            }
            let Some(subject) = names.get(&field.name) else {
                continue;
            };
            let mut field_names = names.clone();
            field_names.insert(Intern::from_ref("self"), subject.clone());
            let scope = OperandTypes {
                typed,
                signatures: &signatures,
                names: field_names,
            };
            if let Err(diagnostic) = scope.predicate(subject, predicate) {
                diagnostics.push((field.name_span, diagnostic));
            }
        }
    }
    for bind in typed.defs.values() {
        let mut names = globals.clone();
        names.extend(bind.params.iter().cloned());
        array_length_names(&mut names);
        for ((name, subject), predicate) in bind.params.iter().zip(bind.param_refinements.iter()) {
            let Some(predicate) = predicate else {
                continue;
            };
            let mut parameter_names = names.clone();
            parameter_names
                .entry(Intern::from_ref("self"))
                .or_insert_with(|| subject.clone());
            let scope = OperandTypes {
                typed,
                signatures: &signatures,
                names: parameter_names,
            };
            if let Err(diagnostic) = scope.predicate(subject, predicate) {
                diagnostics.push((
                    bind.name_span,
                    diagnostic.with_arg("parameter", name.as_str().to_string()),
                ));
            }
        }
    }
    for (span, diagnostic) in diagnostics {
        typed
            .declaration_flaws
            .push((span, diagnostic.at_span_id(span, &typed.span_table)));
    }
}
