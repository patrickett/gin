use std::collections::{HashMap, HashSet};

use ast::{ConstValue, NormalExpr, ParamConvention};
use internment::Intern;

use super::callable_signatures::CallableSignatureScope;
use super::symbolic_values::{field_path_symbol, field_symbol, normal_expr};
use crate::subst::DepSubst;
use crate::ty::Ty;
use crate::typed::{
    BindBody, DefId, ExprId, ScalarFieldEffect, TypedBind, TypedExprKind, TypedFileAst,
};

pub(super) fn variables(expression: &NormalExpr, names: &mut HashSet<Intern<String>>) {
    let mut pending = vec![expression];
    while let Some(expression) = pending.pop() {
        match expression {
            NormalExpr::Var(name) => {
                names.insert(*name);
            }
            NormalExpr::Add(left, right)
            | NormalExpr::Sub(left, right)
            | NormalExpr::Mul(left, right) => pending.extend([left.as_ref(), right.as_ref()]),
            _ => {}
        }
    }
}

fn supported_value(expression: &NormalExpr) -> bool {
    let mut pending = vec![expression];
    for _ in 0..128 {
        let Some(expression) = pending.pop() else {
            return true;
        };
        match expression {
            NormalExpr::Add(left, right)
            | NormalExpr::Sub(left, right)
            | NormalExpr::Mul(left, right) => pending.extend([left.as_ref(), right.as_ref()]),
            NormalExpr::TargetQuery { .. } => return false,
            _ => {}
        }
    }
    pending.is_empty()
}

pub(super) fn instantiate(
    expression: &NormalExpr,
    arguments: &HashMap<Intern<String>, NormalExpr>,
) -> Option<NormalExpr> {
    let mut pending = vec![(expression, false)];
    let mut values = Vec::new();
    for _ in 0..256 {
        let Some((expression, combine)) = pending.pop() else {
            return (values.len() == 1).then(|| values.pop()).flatten();
        };
        if combine {
            let right = Box::new(values.pop()?);
            let left = Box::new(values.pop()?);
            values.push(match expression {
                NormalExpr::Add(_, _) => NormalExpr::Add(left, right),
                NormalExpr::Sub(_, _) => NormalExpr::Sub(left, right),
                NormalExpr::Mul(_, _) => NormalExpr::Mul(left, right),
                _ => return None,
            });
            continue;
        }
        match expression {
            NormalExpr::Var(name) => values.push(instantiate_variable(*name, arguments)?),
            NormalExpr::Add(left, right)
            | NormalExpr::Sub(left, right)
            | NormalExpr::Mul(left, right) => {
                pending.extend([
                    (expression, true),
                    (right.as_ref(), false),
                    (left.as_ref(), false),
                ]);
            }
            NormalExpr::Value(ConstValue::Int(_)) | NormalExpr::TargetQuery { .. } => {
                values.push(expression.clone());
            }
            _ => return None,
        }
    }
    None
}

fn instantiate_variable(
    name: Intern<String>,
    arguments: &HashMap<Intern<String>, NormalExpr>,
) -> Option<NormalExpr> {
    if let Some(value) = arguments.get(&name) {
        return Some(value.clone());
    }
    let (root, suffix) = name.as_str().split_once(".$field")?;
    match arguments.get(&Intern::from_ref(root))? {
        NormalExpr::Var(root) => Some(NormalExpr::Var(Intern::new(format!(
            "{root}.$field{suffix}"
        )))),
        NormalExpr::Value(value @ ConstValue::Record { .. }) => suffix
            .split(".$field")
            .try_fold(value.clone(), |value, field| {
                let ConstValue::Record { fields } = value else {
                    return None;
                };
                Some(fields.get(field.parse::<usize>().ok()?)?.1.clone())
            })
            .map(NormalExpr::Value),
        _ => None,
    }
}

pub(super) fn infer_fields(
    typed: &TypedFileAst,
    bind: &TypedBind,
    signatures: &CallableSignatureScope<'_>,
    effects: &crate::typed::FunctionEffects,
) -> Vec<ScalarFieldEffect> {
    if shared_mutation_parameter(bind) {
        return Vec::new();
    }
    let mut fields = tracked_scalar_fields(typed, bind);
    let values: HashMap<_, _> = fields
        .iter()
        .filter_map(|field| match &field.value {
            NormalExpr::Var(name) => Some((*name, field.value.clone())),
            _ => None,
        })
        .collect();
    if fields.is_empty() {
        return fields;
    }
    let mut inference = FieldInference {
        typed,
        definition: bind,
        signatures,
        tracked_fields: values.keys().copied().collect(),
        values,
        visited: HashSet::new(),
        parameters: bind.params.iter().map(|(name, _)| *name).collect(),
        storage_aliases: HashSet::new(),
    };
    let roots = match &bind.body {
        BindBody::Expr(expression) => vec![*expression],
        BindBody::Body { exprs, ret } => exprs.iter().copied().chain(*ret).collect(),
        BindBody::Unlowered => return Vec::new(),
    };
    let mut complete = true;
    for root in &roots {
        if inference.walk(*root).is_none() {
            complete = false;
            break;
        }
    }
    let explicit = postcondition_fields(&inference, &roots);
    fields.retain_mut(|field| {
        if let Some(value) = inferred_field_value(field, &inference, &explicit, complete, effects) {
            field.value = value;
            true
        } else {
            false
        }
    });
    fields
}

fn field_unchanged(
    bind: &TypedBind,
    field: &ScalarFieldEffect,
    effects: &crate::typed::FunctionEffects,
) -> bool {
    use crate::typed::{Overlap, ReferenceTargetGroup};
    if effects.unknown_memory
        || bind
            .param_conventions
            .iter()
            .enumerate()
            .any(|(parameter, convention)| {
                parameter != field.parameter && *convention == ParamConvention::Mutate
            })
    {
        return false;
    }
    let Some(group) = bind.param_groups.get(field.parameter).copied().flatten() else {
        return false;
    };
    let mut target = ReferenceTargetGroup::Param(group);
    for index in &field.field {
        target = ReferenceTargetGroup::Field {
            base: Box::new(target),
            index: *index,
        };
    }
    let constraints = crate::solver::ConstraintEnv::default();
    effects.writes.iter().all(|write| {
        bind.param_groups.iter().flatten().all(|group| {
            write
                .substitute(*group, &ReferenceTargetGroup::Param(*group))
                .is_none_or(|write| {
                    write.target.overlap(&target, &constraints) == Overlap::Disjoint
                })
        })
    })
}

fn postcondition_fields(
    inference: &FieldInference<'_>,
    roots: &[ExprId],
) -> HashMap<Intern<String>, NormalExpr> {
    let Some(proof) = super::postconditions::common_proof(inference.typed, inference.definition)
    else {
        return HashMap::new();
    };
    let aliases = postcondition_aliases(inference, roots);
    let predicate = ast::PredicateExpr::Proposition(Box::new(proof.proposition.clone()));
    let mut names = HashMap::new();
    for name in super::refinement_terms::predicate_names(&predicate) {
        if let Ok((_, Some(value))) = super::refinement_terms::resolve(
            inference.typed,
            inference.signatures,
            &proof.types,
            &HashMap::new(),
            name,
        ) {
            names.insert(name, value);
        }
    }
    let mut output = HashMap::new();
    let mut pending = vec![&proof.proposition];
    while let Some(proposition) = pending.pop() {
        match proposition {
            ast::ProofProposition::And(left, right) => {
                pending.extend([left.as_ref(), right.as_ref()])
            }
            ast::ProofProposition::Compare {
                left,
                relation: ast::ProofRelation::Equal,
                right,
            } => {
                for (field, value) in [(left, right), (right, left)] {
                    let Some(NormalExpr::Var(field)) =
                        crate::solver::proof_term_to_normal(field, &0.into(), &names)
                    else {
                        continue;
                    };
                    if !inference.tracked_fields.contains(&field) {
                        continue;
                    }
                    let Some(value) = crate::solver::proof_term_to_normal(value, &0.into(), &names)
                    else {
                        continue;
                    };
                    let mut dependencies = HashSet::new();
                    variables(&value, &mut dependencies);
                    if dependencies
                        .iter()
                        .any(|name| inference.tracked_fields.contains(name))
                    {
                        continue;
                    }
                    output.insert(
                        field,
                        DepSubst {
                            consts: aliases.clone(),
                            ..Default::default()
                        }
                        .apply_to_normal(&value),
                    );
                }
            }
            _ => {}
        }
    }
    output
}

struct FieldInference<'a> {
    typed: &'a TypedFileAst,
    definition: &'a TypedBind,
    signatures: &'a CallableSignatureScope<'a>,
    values: HashMap<Intern<String>, NormalExpr>,
    tracked_fields: HashSet<Intern<String>>,
    visited: HashSet<ExprId>,
    parameters: HashSet<Intern<String>>,
    storage_aliases: HashSet<Intern<String>>,
}

impl FieldInference<'_> {
    fn remember_value(
        &mut self,
        expression: ExprId,
        name: Intern<String>,
        value: Option<NormalExpr>,
    ) {
        if let Some(value) = value.filter(supported_value) {
            self.values.insert(name, value);
        } else {
            self.unknown(expression, name);
        }
    }

    fn unknown(&mut self, expression: ExprId, name: Intern<String>) {
        self.values.insert(
            name,
            NormalExpr::Var(Intern::new(format!("$unknown{}.{}", expression.0, name))),
        );
    }

    fn argument_symbol(&self, expression: ExprId) -> Option<NormalExpr> {
        if self.typed.exprs.ty_of(expression).is_some_and(|ty| {
            crate::ty::reference_pointee_for_type(ty, Some(&self.typed.type_registry)).is_some()
        }) {
            let references = self.typed.exprs.references_of(expression.index());
            if references.sources.len() == 1 {
                match references.sources.iter().next()? {
                    crate::typed::ReferenceSource::Argument(argument)
                        if argument.fields.is_empty() =>
                    {
                        let (name, _) = self.definition.params.get(argument.parameter)?;
                        return Some(NormalExpr::Var(*name));
                    }
                    crate::typed::ReferenceSource::Unknown => return None,
                    _ => {}
                }
            } else if !references.sources.is_empty() {
                return None;
            }
        }
        normal_expr(
            self.typed,
            Some(self.signatures),
            &HashMap::new(),
            expression,
        )
    }

    fn walk_call(&mut self, expression: ExprId, target: DefId, args: &[ExprId]) -> Option<()> {
        let signature = self.signatures.get(&target)?;
        if signature.effects.unknown_memory {
            return None;
        }
        let mut arguments = HashMap::new();
        let mut roots = HashMap::new();
        for ((name, _), arg) in signature.params.iter().zip(args) {
            self.walk(*arg)?;
            if let Some(value) = self.argument_symbol(*arg) {
                roots.insert(*name, value.clone());
                if self.typed.exprs.ty_of(*arg).is_some_and(|ty| {
                    !ty.is_ref() && crate::transform::symbolic_values::scalar_type(self.typed, ty)
                }) {
                    if let Some(value) = self.value(*arg) {
                        arguments.insert(*name, value);
                    }
                } else {
                    arguments.insert(*name, value);
                }
            }
        }
        let mut names = HashSet::new();
        for effect in &signature.effects.scalar_fields {
            variables(&effect.value, &mut names);
        }
        for name in names {
            if name.as_str().contains(".$field") {
                let projected = instantiate(&NormalExpr::Var(name), &roots)?;
                let value = DepSubst {
                    consts: self.values.clone(),
                    ..Default::default()
                }
                .apply_to_normal(&projected);
                arguments.insert(name, value);
            }
        }
        let mut updates = Vec::new();
        let mut mutated_roots = Vec::new();
        for (parameter, convention) in signature.param_conventions.iter().enumerate() {
            if *convention != ParamConvention::Mutate {
                continue;
            }
            let (name, _) = signature.params.get(parameter)?;
            let NormalExpr::Var(root) = roots.get(name)? else {
                return None;
            };
            mutated_roots.push(*root);
            let effects: Vec<_> = signature
                .effects
                .scalar_fields
                .iter()
                .filter(|effect| effect.parameter == parameter)
                .collect();
            for effect in effects {
                let NormalExpr::Var(field) = field_path_symbol(*root, &effect.field) else {
                    unreachable!()
                };
                updates.push((field, instantiate(&effect.value, &arguments)?));
            }
        }
        if !signature.effects.writes.is_empty() && updates.is_empty() {
            return None;
        }
        self.apply_field_updates(expression, &mutated_roots, updates);
        Some(())
    }

    fn apply_field_updates(
        &mut self,
        expression: ExprId,
        mutated_roots: &[Intern<String>],
        updates: Vec<(Intern<String>, NormalExpr)>,
    ) {
        let invalidated: Vec<_> = self
            .values
            .keys()
            .copied()
            .filter(|name| {
                mutated_roots
                    .iter()
                    .any(|root| name.as_str().starts_with(&format!("{root}.$field")))
            })
            .collect();
        for name in invalidated {
            self.unknown(expression, name);
        }
        for (name, value) in updates {
            self.remember_value(expression, name, Some(value));
        }
    }

    fn write_field(
        &mut self,
        expression: ExprId,
        base: ExprId,
        index: usize,
        value: ExprId,
    ) -> Option<()> {
        let NormalExpr::Var(base) =
            normal_expr(self.typed, Some(self.signatures), &HashMap::new(), base)?
        else {
            return None;
        };
        let NormalExpr::Var(field) = field_symbol(base, index) else {
            unreachable!()
        };
        self.remember_value(expression, field, self.value(value));
        Some(())
    }

    fn value(&self, expression: ExprId) -> Option<NormalExpr> {
        let expression = normal_expr(
            self.typed,
            Some(self.signatures),
            &HashMap::new(),
            expression,
        )?;
        Some(
            DepSubst {
                consts: self.values.clone(),
                ..Default::default()
            }
            .apply_to_normal(&expression),
        )
    }

    fn external_storage(&self, expression: ExprId) -> bool {
        match self.typed.exprs.kind_of(expression) {
            Some(TypedExprKind::Cast { expr, .. } | TypedExprKind::Ref(expr)) => {
                self.external_storage(*expr)
            }
            Some(TypedExprKind::AggregateGet { base, index }) => {
                let Some(Ty::Record { fields, .. }) = self.typed.exprs.ty_of(*base).map(|ty| {
                    self.typed
                        .type_registry
                        .resolved_definition_for_type(ty.without_reference_wrappers())
                }) else {
                    return false;
                };
                fields.get(*index).is_some_and(|(_, ty)| matches!(ty.as_ref(), Ty::OwnedStorage { .. }))
                    && normal_expr(self.typed, Some(self.signatures), &HashMap::new(), *base)
                        .is_some_and(|value| matches!(value, NormalExpr::Var(name) if self.parameters.contains(&name)))
            }
            Some(TypedExprKind::FnCall {
                target, args: None, ..
            }) => self.storage_aliases.contains(&target.0),
            _ => false,
        }
    }

    fn walk_loop(&mut self, expression: ExprId) -> Option<()> {
        let mut assigned = HashSet::new();
        let _ = crate::typed::walk_expr_preorder(self.typed, expression, &mut |expression| {
            if let Some(TypedExprKind::Reassign { name, .. } | TypedExprKind::Bind { name, .. }) =
                self.typed.exprs.kind_of(expression)
            {
                assigned.insert(*name);
            }
            std::ops::ControlFlow::Continue(())
        });
        for name in assigned {
            if self.values.contains_key(&name) && !self.tracked_fields.contains(&name) {
                self.unknown(expression, name);
                self.storage_aliases.remove(&name);
            }
        }
        let incoming = self.values.clone();
        let aliases = self.storage_aliases.clone();
        self.walk_children(expression)?;
        let changed: Vec<_> = incoming
            .iter()
            .filter(|(name, value)| {
                self.tracked_fields.contains(name) && self.values.get(name) != Some(value)
            })
            .map(|(name, _)| *name)
            .collect();
        self.values = incoming;
        for name in changed {
            self.unknown(expression, name);
        }
        self.storage_aliases = aliases;
        Some(())
    }

    fn walk(&mut self, expression: ExprId) -> Option<()> {
        if !self.visited.insert(expression) {
            return Some(());
        }
        let kind = self.typed.exprs.kind_of(expression)?;
        if matches!(
            kind,
            TypedExprKind::Invalid
                | TypedExprKind::If(_)
                | TypedExprKind::When(_)
                | TypedExprKind::Emit { .. }
        ) {
            return None;
        }
        if matches!(kind, TypedExprKind::Loop(_)) {
            return self.walk_loop(expression);
        }
        if let TypedExprKind::BufSet { buf, .. } = kind
            && !self.external_storage(*buf)
        {
            return None;
        }
        if let TypedExprKind::FnCall {
            target,
            args: Some(args),
            ..
        } = kind
        {
            return self.walk_call(expression, *target, args);
        }
        if let TypedExprKind::FnCall {
            target, args: None, ..
        } = kind
            && self.typed.exprs.place_of(expression).flatten().is_none()
            && self.signatures.get(target).is_some()
        {
            return self.walk_call(expression, *target, &[]);
        }
        self.walk_children(expression)?;
        self.update_expression(expression)
    }

    fn walk_children(&mut self, expression: ExprId) -> Option<()> {
        let kind = self.typed.exprs.kind_of(expression)?;
        let mut children = Vec::new();
        let _ = crate::typed::walk_expr_children(kind, &mut |child| {
            children.push(child);
            std::ops::ControlFlow::Continue(())
        });
        for child in children {
            self.walk(child)?;
        }
        Some(())
    }

    fn update_expression(&mut self, expression: ExprId) -> Option<()> {
        let kind = self.typed.exprs.kind_of(expression)?;
        match kind {
            TypedExprKind::Bind {
                name,
                body,
                unassigned,
                ..
            } => {
                if self.parameters.contains(name) {
                    return None;
                }
                if self.typed.exprs.ty_of(expression).is_some_and(Ty::is_ref) {
                    return None;
                }
                if self.external_storage(*body) {
                    self.storage_aliases.insert(*name);
                } else {
                    self.storage_aliases.remove(name);
                }
                self.remember_value(
                    expression,
                    *name,
                    (!unassigned).then(|| self.value(*body)).flatten(),
                );
            }
            TypedExprKind::Reassign {
                name,
                value,
                operator: None,
            } => {
                if !self.typed.exprs.ty_of(*value).is_some_and(|ty| {
                    crate::transform::symbolic_values::scalar_type(self.typed, ty)
                }) {
                    return None;
                }
                self.remember_value(expression, *name, self.value(*value));
            }
            TypedExprKind::AggregateSet {
                base,
                index,
                value,
                operator: None,
            } => {
                self.write_field(expression, *base, *index, *value)?;
            }
            TypedExprKind::RecordSet {
                base,
                field,
                value,
                operator: None,
            } => {
                let index = super::effects::record_field_index(self.typed, *base, *field)?;
                self.write_field(expression, *base, index, *value)?;
            }
            TypedExprKind::RecordSet { .. }
            | TypedExprKind::AggregateSet { .. }
            | TypedExprKind::Reassign { .. } => return None,
            _ => {}
        }
        Some(())
    }
}

fn shared_mutation_parameter(bind: &TypedBind) -> bool {
    for (parameter, convention) in bind.param_conventions.iter().enumerate() {
        if *convention != ParamConvention::Mutate {
            continue;
        }
        if let Some(group) = bind.param_groups.get(parameter).copied().flatten()
            && bind
                .param_groups
                .iter()
                .enumerate()
                .any(|(other, candidate)| {
                    other != parameter
                        && *candidate == Some(group)
                        && matches!(
                            bind.param_conventions.get(other),
                            Some(ParamConvention::Observe | ParamConvention::Mutate)
                        )
                })
        {
            return true;
        }
    }
    false
}

fn tracked_scalar_fields(typed: &TypedFileAst, bind: &TypedBind) -> Vec<ScalarFieldEffect> {
    let mut fields = Vec::new();
    for (parameter, ((name, ty), convention)) in
        bind.params.iter().zip(&bind.param_conventions).enumerate()
    {
        if *convention != ParamConvention::Mutate {
            continue;
        }
        let Ty::Record {
            fields: record_fields,
            ..
        } = typed
            .type_registry
            .resolved_definition_for_type(ty.without_reference_wrappers())
        else {
            continue;
        };
        let mut pending: Vec<_> = record_fields
            .iter()
            .enumerate()
            .map(|(field, (_, ty))| (vec![field], ty.as_ref().clone()))
            .collect();
        let mut remaining = 128_usize;
        while let Some((path, ty)) = pending.pop() {
            let Some(next) = remaining.checked_sub(1) else {
                return Vec::new();
            };
            remaining = next;
            if path.len() > 16 {
                return Vec::new();
            }
            if crate::transform::symbolic_values::scalar_type(typed, &ty) {
                let value = field_path_symbol(*name, &path);
                fields.push(ScalarFieldEffect {
                    parameter,
                    field: path,
                    value,
                });
            } else if let Ty::Record { fields, .. } =
                typed.type_registry.resolved_definition_for_type(&ty)
            {
                for (index, (_, ty)) in fields.iter().enumerate() {
                    let mut child = path.clone();
                    child.push(index);
                    pending.push((child, ty.as_ref().clone()));
                }
            }
        }
    }
    fields
}

fn inferred_field_value(
    field: &ScalarFieldEffect,
    inference: &FieldInference<'_>,
    explicit: &HashMap<Intern<String>, NormalExpr>,
    complete: bool,
    effects: &crate::typed::FunctionEffects,
) -> Option<NormalExpr> {
    let NormalExpr::Var(symbol) = &field.value else {
        return None;
    };
    let unchanged = field_unchanged(inference.definition, field, effects).then_some(&field.value);
    let Some(value) = explicit
        .get(symbol)
        .or_else(|| complete.then(|| inference.values.get(symbol)).flatten())
        .or(unchanged)
    else {
        return None;
    };
    let mut names = HashSet::new();
    variables(value, &mut names);
    if !supported_value(value) {
        return None;
    }
    if !names.iter().all(|name| {
        inference.definition.params.iter().any(|(parameter, _)| {
            name == parameter || name.as_str().starts_with(&format!("{parameter}.$field"))
        })
    }) {
        return None;
    }
    Some(value.clone())
}

fn postcondition_aliases(
    inference: &FieldInference<'_>,
    roots: &[ExprId],
) -> HashMap<Intern<String>, NormalExpr> {
    let mut aliases = inference.values.clone();
    let mut pending = roots.to_vec();
    let mut visited = HashSet::new();
    while let Some(expression) = pending.pop() {
        if !visited.insert(expression) {
            continue;
        }
        let Some(kind) = inference.typed.exprs.kind_of(expression) else {
            continue;
        };
        if let TypedExprKind::Reassign { name, .. } = kind {
            aliases.remove(name);
        }
        if let TypedExprKind::Bind { name, .. } = kind
            && !inference.visited.contains(&expression)
        {
            aliases.remove(name);
        }
        let _ = crate::typed::walk_expr_children(kind, &mut |child| {
            pending.push(child);
            std::ops::ControlFlow::Continue(())
        });
    }
    aliases
}
