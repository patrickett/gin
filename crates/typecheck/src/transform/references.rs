use super::callable_signatures::{CallableSignatureScope, SignatureReads};
use super::lower_exprs::target_index_for_expr;
use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

use internment::Intern;

use crate::ty::{Ty, reference_pointee_for_type};
use crate::typed::{
    ArgumentReference, BindBody, DefId, EffectTarget, ExprId, PlaceId, ReferenceLoan,
    ReferenceSource, ReferenceSummary, ReferenceTargetGroup, ReferenceValue, ReferenceWriteTarget,
    TypedBind, TypedCallableSignature, TypedExprKind, TypedFileAst, TypedWhenArm,
    walk_expr_children,
};

#[derive(Default)]
pub(super) struct ReferenceWork {
    pub checked: usize,
    pub rounds: usize,
    pub reused: usize,
}

pub(super) fn infer_rounds(
    files: &mut [TypedFileAst],
    external: &HashMap<DefId, TypedCallableSignature>,
    parallel: bool,
) -> ReferenceWork {
    let _profile = compiler_profile::span(compiler_profile::Phase::References);
    infer_queries(files, external, parallel, None)
}

pub(super) type ReferenceQueries = super::check_session::FunctionQueries<ReferenceAnswer>;

#[derive(Clone)]
pub(super) struct ReferenceAnswer {
    summary: ReferenceSummary,
    expressions: Box<[ExpressionAnswer]>,
}

#[derive(Clone)]
struct ExpressionAnswer {
    id: ExprId,
    reference: Option<Box<ReferenceValue>>,
    populated: bool,
    reads: HashSet<ReferenceLoan>,
}

impl ReferenceAnswer {
    fn capture(file: &TypedFileAst, id: DefId, written: HashSet<ExprId>) -> Self {
        let expressions = written
            .into_iter()
            .map(|id| {
                let facts = &file.exprs.facts.ownership_facts;
                let reference = facts.references[id.index()].clone();
                let populated = reference.is_some();
                let reference = reference.filter(|value| **value != ReferenceValue::default());
                ExpressionAnswer {
                    id,
                    reference,
                    populated,
                    reads: facts.reference_reads[id.index()].clone(),
                }
            })
            .collect();
        Self {
            summary: (*file.defs[&id].references).clone(),
            expressions,
        }
    }

    fn relocate(&mut self, relocation: super::check_session::Relocation) -> Option<()> {
        relocation.summary(&mut self.summary)?;
        for expression in &mut self.expressions {
            expression.id = relocation.expression(expression.id)?;
            if let Some(reference) = &mut expression.reference {
                relocation.reference(reference)?;
            }
            relocation.loans(&mut expression.reads)?;
        }
        Some(())
    }

    fn apply(self, file: &mut TypedFileAst, id: DefId) -> bool {
        let changed = *file.defs[&id].references != self.summary;
        file.defs.get_mut(&id).unwrap().references = crate::typed::Shared::new(self.summary);
        for expression in self.expressions {
            let facts = &mut file.exprs.facts.ownership_facts;
            facts.references[expression.id.index()] = if expression.populated {
                Some(expression.reference.unwrap_or_default())
            } else {
                None
            };
            facts.reference_reads[expression.id.index()] = expression.reads;
        }
        changed
    }
}

pub(super) fn infer_queries(
    files: &mut [TypedFileAst],
    external: &HashMap<DefId, TypedCallableSignature>,
    parallel: bool,
    queries: Option<&ReferenceQueries>,
) -> ReferenceWork {
    let _profile = compiler_profile::span(compiler_profile::Phase::References);
    use rayon::prelude::*;
    let inputs: Option<Vec<_>> = queries.map(|queries| {
        files
            .iter()
            .map(|file| queries.inputs(file, None))
            .collect()
    });
    let mut work = ReferenceWork::default();
    loop {
        let signatures = CallableSignatureScope::new(
            external,
            files
                .iter()
                .flat_map(|file| file.defs.iter().map(|(id, bind)| (*id, bind.into()))),
        );
        work.rounds += 1;
        let (shared, local) = signatures.inputs();
        let profile_context = compiler_profile::Context::current();
        let infer = |(index, file): (usize, &mut TypedFileAst)| {
            let _profile_context = profile_context
                .as_ref()
                .map(compiler_profile::Context::enter);
            infer_cached_file(
                file,
                &CallableSignatureScope::borrowed(shared, local),
                queries,
                inputs.as_ref().map(|inputs| &inputs[index]),
            )
        };
        let results: Vec<_> = if parallel {
            files.par_iter_mut().enumerate().map(infer).collect()
        } else {
            files.iter_mut().enumerate().map(infer).collect()
        };
        let mut changed = false;
        for (updated, checked, reused) in results {
            changed |= updated;
            work.checked += checked;
            work.reused += reused;
        }
        if !changed {
            return work;
        }
    }
}

fn infer_cached_file(
    file: &mut TypedFileAst,
    signatures: &CallableSignatureScope<'_>,
    queries: Option<&ReferenceQueries>,
    inputs: Option<
        &HashMap<
            DefId,
            (
                std::sync::Arc<super::check_session::FunctionInput>,
                super::check_session::Relocation,
            ),
        >,
    >,
) -> (bool, usize, usize) {
    let Some(queries) = queries else {
        return (infer_file(file, signatures), file.defs.len(), 0);
    };
    let ids: Vec<_> = file.defs.keys().copied().collect();
    let mut changed = false;
    let mut checked = 0;
    let mut reused = 0;
    for id in ids {
        let input = inputs.and_then(|inputs| inputs.get(&id));
        if let Some((input, relocation)) = input {
            if let Some(mut output) = queries.get(file, id, input, *relocation, signatures) {
                if output.relocate(relocation.inverse()).is_some() {
                    changed |= output.apply(file, id);
                    reused += 1;
                    continue;
                }
            }
        }
        let reads = SignatureReads::default();
        let (shared, local) = signatures.inputs();
        let tracked = CallableSignatureScope::recorded(shared, local, &reads);
        let recording = input
            .as_ref()
            .map(|_| super::semantic_reads::SemanticReadGuard::begin());
        let bind = file.defs[&id].clone();
        changed |= infer_bind(file, id, &bind, &tracked);
        checked += 1;
        let types = recording.map(|recording| recording.finish());
        if let Some((input, relocation)) = input {
            let (lookups, scanned) = reads.into_parts();
            assert!(!scanned);
            let mut types = types.unwrap();
            let mut output = ReferenceAnswer::capture(file, id, std::mem::take(&mut types.written));
            if !types.scanned && output.relocate(*relocation).is_some() {
                queries.insert(
                    file,
                    id,
                    (**input).clone(),
                    *relocation,
                    signatures,
                    lookups,
                    types,
                    output,
                );
            }
        }
    }
    (changed, checked, reused)
}

fn infer_file(file: &mut TypedFileAst, signatures: &CallableSignatureScope<'_>) -> bool {
    let binds: Vec<_> = file
        .defs
        .iter()
        .map(|(id, bind)| (*id, bind.clone()))
        .collect();
    let mut changed = false;
    for (id, bind) in binds {
        changed |= infer_bind(file, id, &bind, signatures);
    }
    changed
}

fn infer_bind(
    file: &mut TypedFileAst,
    id: DefId,
    bind: &TypedBind,
    signatures: &CallableSignatureScope<'_>,
) -> bool {
    let mut analysis = ReferenceAnalysis::new(file, signatures, bind);
    analysis.body(&bind.body);
    let mut summary = analysis.summary;
    summary
        .result
        .constrain_to(&reference_shape(file, &bind.return_type));
    if *file.defs[&id].references != summary {
        file.defs.get_mut(&id).unwrap().references = crate::typed::Shared::new(summary);
        return true;
    }
    false
}

#[cfg(test)]
mod tests;

pub(crate) fn reference_shape(typed: &TypedFileAst, ty: &Ty) -> ReferenceValue {
    fn build(typed: &TypedFileAst, ty: &Ty, visiting: &mut Vec<Ty>) -> ReferenceValue {
        let pointee = reference_pointee_for_type(ty, Some(&typed.type_registry));
        let mut value = ReferenceValue::default();
        if pointee.is_some() {
            value.sources.insert(ReferenceSource::Unknown);
        }
        let ty = pointee.as_ref().unwrap_or(ty);
        if visiting.contains(ty) {
            return value;
        }
        visiting.push(ty.clone());
        value.fields = match typed.type_registry.resolved_definition_for_type(ty) {
            Ty::Record { fields, .. } => fields
                .iter()
                .map(|(_, ty)| build(typed, ty, visiting))
                .collect(),
            Ty::Sequence(fields) => fields
                .iter()
                .map(|field| build(typed, &field.ty, visiting))
                .collect(),
            Ty::Array { elem, size } => {
                if size.as_const_int() != Some(0) {
                    value.array_element = Some(Box::new(build(typed, &elem, visiting)));
                }
                Vec::new()
            }
            _ => Vec::new(),
        };
        visiting.pop();
        value
    }
    build(typed, ty, &mut Vec::new())
}

fn parameterize(value: &mut ReferenceValue, parameter: usize, path: &mut Vec<usize>) {
    if !value.sources.is_empty() {
        value.sources = HashSet::from([ReferenceSource::Argument(ArgumentReference {
            parameter,
            fields: path.clone(),
        })]);
    }
    if let Some(element) = &mut value.array_element {
        let origin = ArgumentReference {
            parameter,
            fields: path.clone(),
        };
        fn elements(value: &mut ReferenceValue, origin: &ArgumentReference) {
            if !value.sources.is_empty() {
                value.sources = HashSet::from([ReferenceSource::ArgumentElements(origin.clone())]);
            }
            for field in &mut value.fields {
                elements(field, origin);
            }
            if let Some(element) = &mut value.array_element {
                elements(element, origin);
            }
        }
        elements(element, &origin);
    }
    for (index, field) in value.fields.iter_mut().enumerate() {
        path.push(index);
        parameterize(field, parameter, path);
        path.pop();
    }
}

#[derive(Clone, Default)]
struct ReferenceState {
    values: HashMap<Intern<String>, ReferenceValue>,
    storage: HashMap<PlaceId, ReferenceValue>,
    writes: HashMap<ReferenceWriteTarget, ReferenceValue>,
    arguments: HashMap<ArgumentReference, ReferenceValue>,
    unknown_writes: bool,
    unknown_slot_writes: HashMap<crate::typed::ReferenceWriteOrigin, ReferenceValue>,
}

impl ReferenceState {
    fn join(&mut self, other: &Self, origins: &HashMap<ReferenceWriteTarget, ReferenceValue>) {
        for (name, value) in &other.values {
            self.values.entry(*name).or_default().join(value);
        }
        for (place, value) in &other.storage {
            self.storage.entry(*place).or_default().join(value);
        }
        for (target, value) in &mut self.writes {
            if !other.writes.contains_key(target) {
                value.join(&origins[target]);
            }
        }
        for (target, value) in &other.writes {
            self.writes
                .entry(target.clone())
                .or_insert_with(|| origins[target].clone())
                .join(value);
        }
        for (argument, value) in &other.arguments {
            self.arguments
                .entry(argument.clone())
                .or_default()
                .join(value);
        }
        self.unknown_writes |= other.unknown_writes;
        for (base, value) in &other.unknown_slot_writes {
            self.unknown_slot_writes
                .entry(base.clone())
                .or_default()
                .join(value);
        }
    }
}

struct ReferenceAnalysis<'a> {
    typed: &'a mut TypedFileAst,
    signatures: &'a CallableSignatureScope<'a>,
    state: ReferenceState,
    active: HashSet<ExprId>,
    summary: ReferenceSummary,
    write_origins: HashMap<ReferenceWriteTarget, ReferenceValue>,
    exits: usize,
    parameter_types: Vec<Ty>,
}

impl<'a> ReferenceAnalysis<'a> {
    fn new(
        typed: &'a mut TypedFileAst,
        signatures: &'a CallableSignatureScope<'a>,
        bind: &TypedBind,
    ) -> Self {
        let values: HashMap<_, _> = bind
            .params
            .iter()
            .enumerate()
            .map(|(index, (name, ty))| {
                let mut value = reference_shape(typed, ty);
                if matches!(
                    bind.param_conventions.get(index),
                    Some(ast::ParamConvention::Observe | ast::ParamConvention::Mutate)
                ) {
                    value.sources.insert(ReferenceSource::Unknown);
                }
                parameterize(&mut value, index, &mut Vec::new());
                (*name, value)
            })
            .collect();
        let mut arguments = HashMap::new();
        for value in values.values() {
            value.visit(&mut |value| {
                for source in &value.sources {
                    if let ReferenceSource::Argument(argument) = source {
                        arguments.insert(argument.clone(), value.clone());
                    }
                }
            });
        }
        Self {
            typed,
            signatures,
            state: ReferenceState {
                values,
                storage: HashMap::new(),
                writes: HashMap::new(),
                arguments,
                unknown_writes: false,
                unknown_slot_writes: HashMap::new(),
            },
            active: HashSet::new(),
            summary: ReferenceSummary::default(),
            write_origins: HashMap::new(),
            exits: 0,
            parameter_types: bind.params.iter().map(|(_, ty)| ty.clone()).collect(),
        }
    }

    fn body(&mut self, body: &BindBody) {
        match body {
            BindBody::Expr(expr) => {
                let value = self.expr(*expr);
                self.summary.result.join(&value.without_loans());
            }
            BindBody::Body { exprs, ret } => self.sequence(exprs, *ret),
            BindBody::Unlowered => return,
        }
        self.capture_reference_writes();
    }

    fn capture_reference_writes(&mut self) {
        self.summary.untracked_writes |= self.state.unknown_writes;
        self.summary.unknown_writes |=
            self.state.unknown_writes || !self.state.unknown_slot_writes.is_empty();
        for (base, value) in &self.state.unknown_slot_writes {
            self.summary
                .unknown_slot_writes
                .entry(base.clone())
                .or_default()
                .join(&value.clone().without_loans());
        }
        for (target, value) in &self.state.writes {
            let summary = self
                .summary
                .writes
                .entry(target.clone())
                .or_insert_with(|| {
                    if self.exits > 0 {
                        self.write_origins[target].clone()
                    } else {
                        ReferenceValue::default()
                    }
                });
            summary.join(&value.clone().without_loans());
        }
        for (target, value) in &mut self.summary.writes {
            if !self.state.writes.contains_key(target) {
                value.join(&self.write_origins[target]);
            }
        }
        self.exits += 1;
    }

    fn sequence(&mut self, exprs: &[ExprId], ret: Option<ExprId>) {
        for expr in exprs {
            self.expr(*expr);
        }
        if let Some(ret) = ret {
            let value = self.expr(ret);
            self.summary.result.join(&value.without_loans());
        }
    }

    fn read(&mut self, value: &ReferenceValue, loans: &mut HashSet<ReferenceLoan>) {
        loans.extend(value.loans.iter().cloned());
        for source in &value.sources {
            if let ReferenceSource::Argument(argument)
            | ReferenceSource::ArgumentElements(argument) = source
            {
                self.summary.reads.insert(argument.clone());
            }
        }
    }

    fn storage_value(
        &self,
        target: &ReferenceTargetGroup,
        active: &mut HashSet<ReferenceTargetGroup>,
    ) -> Option<ReferenceValue> {
        if !active.insert(target.clone()) {
            return None;
        }
        let value = match target {
            ReferenceTargetGroup::Local(place) => self.state.storage.get(place).cloned(),
            ReferenceTargetGroup::Field { base, index } => self
                .storage_value(base, active)
                .and_then(|value| value.fields.get(*index).cloned()),
            ReferenceTargetGroup::Deref(base) => self
                .storage_value(base, active)
                .map(|value| self.refresh_reference(value, active)),
            _ => None,
        };
        active.remove(target);
        value
    }

    fn refresh_reference(
        &self,
        mut value: ReferenceValue,
        active: &mut HashSet<ReferenceTargetGroup>,
    ) -> ReferenceValue {
        let mut contents = ReferenceValue::default();
        let mut complete = !value.sources.is_empty();
        for source in &value.sources {
            let current = match source {
                ReferenceSource::Target(target) => self.storage_value(target, active),
                ReferenceSource::Argument(argument) => self.state.arguments.get(argument).cloned(),
                ReferenceSource::ArgumentElements(_) | ReferenceSource::Unknown => None,
            };
            if let Some(current) = current {
                contents.join(&current);
            } else {
                complete = false;
            }
        }
        if complete {
            value.fields = contents.fields;
            value.array_element = contents.array_element;
        } else {
            for (index, field) in contents.fields.iter().enumerate() {
                if let Some(old) = value.fields.get_mut(index) {
                    old.join(field);
                }
            }
        }
        value
    }

    fn assign_storage_field(
        &mut self,
        target: &ReferenceTargetGroup,
        index: usize,
        value: &ReferenceValue,
        exact: bool,
    ) {
        let mut path = vec![index];
        let mut target = target;
        while let ReferenceTargetGroup::Field { base, index } = target {
            path.push(*index);
            target = base;
        }
        let ReferenceTargetGroup::Local(place) = target else {
            return;
        };
        let Some(mut target) = self.state.storage.get_mut(place) else {
            return;
        };
        for index in path.into_iter().rev() {
            if index >= target.fields.len() {
                if let Some(element) = &mut target.array_element {
                    element.join(value);
                }
                return;
            }
            target = &mut target.fields[index];
        }
        if exact {
            *target = value.clone();
        } else {
            target.join(value);
        }
    }

    fn argument_slot_shape(
        &self,
        base: &ArgumentReference,
        field: usize,
    ) -> Option<ReferenceValue> {
        let mut ty = self.parameter_types.get(base.parameter)?.clone();
        let mut path = base.fields.clone();
        path.push(field);
        for index in &path {
            let pointee = reference_pointee_for_type(&ty, Some(&self.typed.type_registry));
            ty = self
                .typed
                .type_registry
                .resolved_definition_for_type(pointee.as_ref().unwrap_or(&ty));
            ty = match ty {
                Ty::Record { fields, .. } => fields.get(*index)?.1.as_ref().clone(),
                Ty::Sequence(fields) => fields.get(*index)?.ty.as_ref().clone(),
                Ty::Array { elem, size } => {
                    if size
                        .as_const_int()
                        .is_some_and(|length| length < 0 || *index as u128 >= length as u128)
                    {
                        return None;
                    }
                    elem.as_ref().clone()
                }
                _ => return None,
            };
        }
        let mut value = reference_shape(self.typed, &ty);
        parameterize(&mut value, base.parameter, &mut path);
        Some(value)
    }

    fn record_argument_write(
        &mut self,
        base: ArgumentReference,
        field: usize,
        mut value: ReferenceValue,
        exact: bool,
    ) {
        let Some(original) = self
            .state
            .arguments
            .get(&base)
            .and_then(|argument| argument.fields.get(field))
            .cloned()
            .or_else(|| self.argument_slot_shape(&base, field))
        else {
            self.state.unknown_writes = true;
            return;
        };
        let mut has_references = false;
        original.visit(&mut |value| has_references |= !value.sources.is_empty());
        value.visit(&mut |value| has_references |= !value.sources.is_empty());
        if !has_references {
            return;
        }
        let target = ReferenceWriteTarget {
            base: base.clone(),
            field,
        };
        let shape = self
            .write_origins
            .entry(target.clone())
            .or_insert_with(|| original.clone().without_loans())
            .clone();
        value.constrain_to(&shape);
        if let Some(element) = self
            .state
            .arguments
            .get_mut(&base)
            .and_then(|argument| argument.array_element.as_deref_mut())
        {
            element.join(&value);
        }
        let slot = self
            .state
            .arguments
            .get_mut(&base)
            .and_then(|argument| argument.fields.get_mut(field));
        if exact {
            if let Some(slot) = slot {
                *slot = value.clone();
            }
            self.state.writes.insert(target, value.without_loans());
        } else {
            if let Some(slot) = slot {
                slot.join(&value);
            }
            self.state
                .writes
                .entry(target)
                .or_insert(original)
                .join(&value.without_loans());
        }
    }

    fn record_unknown_slot_write(&mut self, destination: &ReferenceValue, value: &ReferenceValue) {
        let pointees_only = !destination.sources.is_empty()
            && destination
                .sources
                .iter()
                .all(|source| matches!(source, ReferenceSource::ArgumentElements(_)));
        for source in &destination.sources {
            let origin = match source {
                ReferenceSource::Argument(base) => {
                    crate::typed::ReferenceWriteOrigin::Slot(base.clone())
                }
                ReferenceSource::ArgumentElements(base) => {
                    crate::typed::ReferenceWriteOrigin::Elements(base.clone())
                }
                ReferenceSource::Unknown => {
                    self.state.unknown_writes = true;
                    continue;
                }
                ReferenceSource::Target(target) => {
                    self.state.unknown_writes |= target.root_local().is_none();
                    continue;
                }
            };
            self.state
                .unknown_slot_writes
                .entry(origin)
                .or_default()
                .join(&value.clone().without_loans());
        }
        self.widen_reference_fields(pointees_only);
    }

    fn invalidate_unknown_reference_fields(&mut self) {
        self.widen_reference_fields(false);
        self.state.unknown_writes = true;
    }

    fn widen_reference_fields(&mut self, pointees_only: bool) {
        fn widen_pointees(value: &mut ReferenceValue) {
            if !value.sources.is_empty() {
                invalidate(value, false);
            } else {
                for field in value
                    .fields
                    .iter_mut()
                    .chain(value.array_element.as_deref_mut())
                {
                    widen_pointees(field);
                }
            }
        }
        fn invalidate(value: &mut ReferenceValue, pointees_only: bool) {
            for field in &mut value.fields {
                if !field.sources.is_empty() {
                    field.sources.insert(ReferenceSource::Unknown);
                }
                invalidate(field, pointees_only);
            }
            if let Some(element) = &mut value.array_element {
                if pointees_only {
                    widen_pointees(element);
                } else {
                    if !element.sources.is_empty() {
                        element.sources.insert(ReferenceSource::Unknown);
                    }
                    invalidate(element, false);
                }
            }
        }
        for value in self
            .state
            .storage
            .values_mut()
            .chain(self.state.values.values_mut())
            .chain(self.state.arguments.values_mut())
        {
            invalidate(value, pointees_only);
        }
        for value in self.state.writes.values_mut() {
            if !value.sources.is_empty() {
                value.sources.insert(ReferenceSource::Unknown);
            }
            invalidate(value, pointees_only);
        }
    }

    fn assign_field(&mut self, base: ExprId, index: usize, value: ReferenceValue) {
        let reference = match self.typed.exprs.kind_of(base) {
            Some(TypedExprKind::Deref(inner)) => *inner,
            _ => base,
        };
        let sources = self
            .typed
            .exprs
            .references_of(reference.index())
            .sources
            .clone();
        if !sources.is_empty() {
            let exact = sources.len() == 1;
            for source in sources {
                match source {
                    ReferenceSource::Target(target) => {
                        self.assign_storage_field(&target, index, &value, exact);
                    }
                    ReferenceSource::Argument(base) => {
                        self.record_argument_write(base, index, value.clone(), exact);
                    }
                    ReferenceSource::ArgumentElements(base) => {
                        let destination = ReferenceValue {
                            sources: HashSet::from([ReferenceSource::ArgumentElements(base)]),
                            ..Default::default()
                        };
                        self.record_unknown_slot_write(&destination, &value);
                    }
                    ReferenceSource::Unknown => {
                        let mut has_references = false;
                        value.visit(&mut |value| has_references |= !value.sources.is_empty());
                        if has_references {
                            self.invalidate_unknown_reference_fields();
                        }
                    }
                }
            }
        } else if let Some(mut place) = self.typed.exprs.place_of(base).flatten() {
            let mut path = Vec::new();
            while let Some(parent) = self.typed.places[place.0 as usize].parent {
                match self.typed.places[place.0 as usize].projection {
                    Some(crate::typed::PlaceProjection::Field(index)) => path.push(index),
                    Some(crate::typed::PlaceProjection::Deref) => {}
                    _ => return,
                }
                place = parent;
            }
            let mut target = ReferenceTargetGroup::Local(place);
            for index in path.into_iter().rev() {
                target = ReferenceTargetGroup::Field {
                    base: Box::new(target),
                    index,
                };
            }
            self.assign_storage_field(&target, index, &value, true);
        }
        let Some(mut place) = self.typed.exprs.place_of(base).flatten() else {
            return;
        };
        let mut path = vec![index];
        while let Some(parent) = self.typed.places[place.0 as usize].parent {
            match self.typed.places[place.0 as usize].projection {
                Some(crate::typed::PlaceProjection::Field(index)) => path.push(index),
                Some(crate::typed::PlaceProjection::Deref) => {}
                _ => return,
            }
            place = parent;
        }
        let name = self.typed.places[place.0 as usize].name;
        let Some(mut target) = self.state.values.get_mut(&name) else {
            return;
        };
        for index in path.into_iter().rev() {
            if index >= target.fields.len() {
                if let Some(element) = &mut target.array_element {
                    element.join(&value);
                }
                return;
            }
            target = &mut target.fields[index];
        }
        *target = value;
    }

    fn substitute(
        &self,
        value: &ReferenceValue,
        args: &[ReferenceValue],
        ids: &[ExprId],
        signature: &TypedCallableSignature,
        site: &ReferenceLoan,
    ) -> ReferenceValue {
        let mut result = ReferenceValue::default();
        for source in &value.sources {
            match source {
                ReferenceSource::Argument(argument) => {
                    if let Some(actual) = args
                        .get(argument.parameter)
                        .and_then(|value| value.field(&argument.fields))
                    {
                        result.sources.extend(actual.sources.iter().cloned());
                        result.loans.extend(actual.loans.iter().cloned());
                        if actual.sources.is_empty()
                            && matches!(
                                signature.param_conventions.get(argument.parameter),
                                Some(ast::ParamConvention::Observe | ast::ParamConvention::Mutate)
                            )
                            && let Some(targets) = ids
                                .get(argument.parameter)
                                .and_then(|id| self.typed.exprs.target_group_of(*id))
                                .and_then(Option::as_ref)
                        {
                            for target in targets.iter() {
                                let mut target = target.clone();
                                for index in &argument.fields {
                                    target = crate::typed::ReferenceTargetGroup::Field {
                                        base: Box::new(target),
                                        index: *index,
                                    };
                                }
                                result.sources.insert(ReferenceSource::Target(target));
                            }
                            result.loans.insert(site.clone());
                        }
                    } else {
                        result.sources.insert(ReferenceSource::Unknown);
                    }
                }
                ReferenceSource::ArgumentElements(argument) => {
                    if let Some(actual) = args
                        .get(argument.parameter)
                        .and_then(|value| value.field(&argument.fields))
                    {
                        let contents = actual.array_contents();
                        result.sources.extend(contents.sources);
                        result.loans.extend(contents.loans);
                        result.loans.insert(site.clone());
                    } else {
                        result.sources.insert(ReferenceSource::Unknown);
                    }
                }
                ReferenceSource::Target(target) if target.is_static() => {
                    result.sources.insert(source.clone());
                }
                ReferenceSource::Target(target) => {
                    let application = self.typed.apply_target_groups(ids, signature);
                    if let Some(group) = target.root_param()
                        && let Some(targets) = application.targets(group)
                        && let Some(effect) = EffectTarget::from_reference(target)
                    {
                        for actual in targets.iter() {
                            if let Some(applied) = effect.substitute(group, actual) {
                                result
                                    .sources
                                    .insert(ReferenceSource::Target(applied.target));
                            }
                        }
                        result.loans.insert(site.clone());
                    } else {
                        result.sources.insert(ReferenceSource::Unknown);
                    }
                }
                ReferenceSource::Unknown => {
                    result.sources.insert(ReferenceSource::Unknown);
                }
            }
        }
        result.array_element = value
            .array_element
            .as_ref()
            .map(|element| Box::new(self.substitute(element, args, ids, signature, site)));
        result.fields = value
            .fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let mut site = site.clone();
                site.fields.push(index);
                self.substitute(field, args, ids, signature, &site)
            })
            .collect();
        result
    }

    fn expr(&mut self, id: ExprId) -> ReferenceValue {
        if !self.active.insert(id) {
            return ReferenceValue::default();
        }
        let kind = self.typed.exprs.kind[id.index()].clone();
        let ty = self.typed.exprs.ty[id.index()].clone();
        let mut reads = HashSet::new();
        let value = match kind {
            TypedExprKind::FnCall { target, args, .. } => {
                if args.as_ref().is_none_or(Vec::is_empty)
                    && self.state.values.contains_key(&target.0)
                {
                    let value = self
                        .typed
                        .exprs
                        .place_of(id)
                        .flatten()
                        .and_then(|place| self.state.storage.get(&place))
                        .unwrap_or(&self.state.values[&target.0])
                        .clone();
                    self.refresh_reference(value, &mut HashSet::new())
                } else if let Some(signature) = self.signatures.get(&target).cloned() {
                    let ids = args.unwrap_or_default();
                    let args: Vec<_> = ids.iter().map(|id| self.expr(*id)).collect();
                    for argument in &signature.references.reads {
                        if let Some(value) = args
                            .get(argument.parameter)
                            .and_then(|value| value.field(&argument.fields))
                        {
                            self.read(value, &mut reads);
                        }
                    }
                    let mut result = self.substitute(
                        &signature.references.result,
                        &args,
                        &ids,
                        &signature,
                        &ReferenceLoan::new(id),
                    );
                    let static_result = !result.sources.is_empty()
                        && result.sources.iter().all(|source| {
                            matches!(source, ReferenceSource::Target(target) if target.is_static())
                        });
                    if reference_pointee_for_type(&ty, Some(&self.typed.type_registry)).is_some()
                        && let Some(group) = signature.result_groups.first().copied().flatten()
                        && !static_result
                        && (result.sources.is_empty()
                            || result.sources.contains(&ReferenceSource::Unknown))
                    {
                        result.sources.retain(|source| matches!(source, ReferenceSource::Target(target) if target.is_static()));
                        result.loans.clear();
                        for (index, parameter_group) in signature.param_groups.iter().enumerate() {
                            if *parameter_group == Some(group)
                                && let Some(arg) = args.get(index)
                            {
                                result.sources.extend(arg.sources.iter().cloned());
                                result.loans.extend(arg.loans.iter().cloned());
                            }
                        }
                    }
                    for (target, value) in &signature.references.writes {
                        let Some(destination) = args
                            .get(target.base.parameter)
                            .and_then(|argument| argument.field(&target.base.fields))
                        else {
                            continue;
                        };
                        let value = self.substitute(
                            value,
                            &args,
                            &ids,
                            &signature,
                            &ReferenceLoan::new(id),
                        );
                        let exact = destination.sources.len() == 1;
                        for source in &destination.sources {
                            match source {
                                ReferenceSource::Target(target_group) => {
                                    self.assign_storage_field(
                                        target_group,
                                        target.field,
                                        &value,
                                        exact,
                                    );
                                }
                                ReferenceSource::Argument(base) => {
                                    self.record_argument_write(
                                        base.clone(),
                                        target.field,
                                        value.clone(),
                                        exact,
                                    );
                                }
                                ReferenceSource::ArgumentElements(base) => {
                                    let destination = ReferenceValue {
                                        sources: HashSet::from([
                                            ReferenceSource::ArgumentElements(base.clone()),
                                        ]),
                                        ..Default::default()
                                    };
                                    self.record_unknown_slot_write(&destination, &value);
                                }
                                ReferenceSource::Unknown => {
                                    self.invalidate_unknown_reference_fields()
                                }
                            }
                        }
                    }
                    for (origin, value) in &signature.references.unknown_slot_writes {
                        let (base, elements) = match origin {
                            crate::typed::ReferenceWriteOrigin::Slot(base) => (base, false),
                            crate::typed::ReferenceWriteOrigin::Elements(base) => (base, true),
                        };
                        if let Some(destination) = args
                            .get(base.parameter)
                            .and_then(|argument| argument.field(&base.fields))
                        {
                            let value = self.substitute(
                                value,
                                &args,
                                &ids,
                                &signature,
                                &ReferenceLoan::new(id),
                            );
                            if elements {
                                self.record_unknown_slot_write(
                                    &destination.array_contents(),
                                    &value,
                                );
                            } else {
                                self.record_unknown_slot_write(destination, &value);
                            }
                        }
                    }
                    if signature.references.untracked_writes {
                        self.invalidate_unknown_reference_fields();
                    }
                    result
                } else {
                    for arg in args.unwrap_or_default() {
                        self.expr(arg);
                    }
                    reference_shape(self.typed, &ty)
                }
            }
            TypedExprKind::SelfRef { .. } => self
                .state
                .values
                .get(&Intern::from_ref("self"))
                .cloned()
                .unwrap_or_default(),
            TypedExprKind::Ref(inner) => {
                let mut value = if let Some(TypedExprKind::Deref(referent)) =
                    self.typed.exprs.kind_of(inner)
                {
                    let referent = *referent;
                    let value = self.expr(referent);
                    let mut storage = value.clone();
                    storage.sources.clear();
                    storage.loans.clear();
                    super::semantic_reads::reference_written(inner);
                    *self.typed.exprs.references_of_mut(inner.index()) = storage;
                    self.typed.exprs.facts.ownership_facts.reference_reads[inner.index()].clear();
                    value
                } else {
                    self.expr(inner)
                };
                if !ty.is_mutable_ref()
                    && crate::array_promotion::promoted_array_value(
                        &self.typed.exprs,
                        &self.typed.type_registry,
                        inner,
                    )
                    .is_some()
                {
                    value
                        .sources
                        .insert(ReferenceSource::Target(ReferenceTargetGroup::Static));
                    value.loans.clear();
                }
                if value.sources.is_empty() {
                    let parent = match self.typed.exprs.kind_of(inner) {
                        Some(TypedExprKind::AggregateGet { base, .. }) => {
                            Some(self.typed.exprs.references_of(base.index()))
                        }
                        Some(
                            TypedExprKind::SliceGet { buf, .. } | TypedExprKind::BufGet { buf, .. },
                        ) => {
                            let reference = self.typed.exprs.references_of(buf.index());
                            Some(reference)
                        }
                        _ => None,
                    };
                    if let Some(parent) = parent.filter(|parent| !parent.sources.is_empty()) {
                        value.sources.clone_from(&parent.sources);
                        if let Some(TypedExprKind::AggregateGet { index, .. }) =
                            self.typed.exprs.kind_of(inner)
                        {
                            value.sources = value
                                .sources
                                .into_iter()
                                .map(|source| match source {
                                    ReferenceSource::Argument(mut argument) => {
                                        argument.fields.push(*index);
                                        ReferenceSource::Argument(argument)
                                    }
                                    ReferenceSource::Target(target) => ReferenceSource::Target(
                                        crate::typed::ReferenceTargetGroup::Field {
                                            base: Box::new(target),
                                            index: *index,
                                        },
                                    ),
                                    ReferenceSource::ArgumentElements(argument) => {
                                        ReferenceSource::ArgumentElements(argument)
                                    }
                                    ReferenceSource::Unknown => ReferenceSource::Unknown,
                                })
                                .collect();
                        }
                        value.loans.clone_from(&parent.loans);
                    } else if let Some(targets) =
                        self.typed.exprs.target_group_of(id).cloned().flatten()
                    {
                        value.sources = targets
                            .iter()
                            .cloned()
                            .map(ReferenceSource::Target)
                            .collect();
                        value.loans.insert(ReferenceLoan::new(id));
                    } else {
                        value.sources.insert(ReferenceSource::Unknown);
                        value.loans.insert(ReferenceLoan::new(id));
                    }
                }
                if matches!(self.typed.exprs.kind_of(inner), Some(TypedExprKind::SliceGet { buf, .. } | TypedExprKind::BufGet { buf, .. })
                    if crate::ty::reference_pointee_for_type(&self.typed.exprs.ty[buf.index()], Some(&self.typed.type_registry))
                            .is_some_and(|pointee| matches!(pointee, Ty::Array { .. })))
                {
                    value.loans.insert(ReferenceLoan::new(id));
                }
                value
            }
            TypedExprKind::Bind {
                name,
                stmts,
                body,
                unassigned,
            } => {
                for stmt in stmts {
                    if stmt != id {
                        self.expr(stmt);
                    }
                }
                let value = if unassigned {
                    reference_shape(self.typed, &ty)
                } else {
                    self.expr(body)
                };
                self.state.values.insert(name, value.clone());
                if let Some(place) = self.typed.exprs.place_of(id).flatten() {
                    self.state.storage.insert(place, value.clone());
                }
                value
            }
            TypedExprKind::Reassign { name, value, .. } => {
                let value = self.expr(value);
                self.state.values.insert(name, value.clone());
                if let Some(place) = self.typed.exprs.place_of(id).flatten() {
                    self.state.storage.insert(place, value.clone());
                }
                value
            }
            TypedExprKind::RecordValue(fields)
            | TypedExprKind::ResultRow(fields)
            | TypedExprKind::ArrayLit(fields)
            | TypedExprKind::List(fields) => ReferenceValue {
                fields: fields.into_iter().map(|field| self.expr(field)).collect(),
                ..Default::default()
            },
            TypedExprKind::TagCall { args, .. } => ReferenceValue {
                fields: args
                    .unwrap_or_default()
                    .into_iter()
                    .map(|field| self.expr(field))
                    .collect(),
                ..Default::default()
            },
            TypedExprKind::AggregateGet { base, index } => {
                let value = self.expr(base);
                self.read(&value, &mut reads);
                value
                    .fields
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| reference_shape(self.typed, &ty))
            }
            TypedExprKind::SliceGet { buf, start, length } => {
                let value = self.expr(buf);
                self.expr(start);
                self.expr(length);
                value
            }
            TypedExprKind::BufGet { buf, index } => {
                let value = self.expr(buf);
                self.expr(index);
                self.read(&value, &mut reads);
                let buffer = value;
                self.read(&buffer, &mut reads);
                let index = match target_index_for_expr(self.typed, index) {
                    crate::typed::TargetIndex::Symbolic(index) => index
                        .as_const_int()
                        .and_then(|index| usize::try_from(index).ok()),
                    crate::typed::TargetIndex::Unknown => None,
                };
                let mut element =
                    if let Some(element) = index.and_then(|index| buffer.fields.get(index)) {
                        element.clone()
                    } else if buffer.fields.is_empty() {
                        let mut element = ReferenceValue::default();
                        let mut complete = index.is_some() && !buffer.sources.is_empty();
                        for source in &buffer.sources {
                            let ReferenceSource::Argument(base) = source else {
                                complete = false;
                                continue;
                            };
                            let Some(index) = index else {
                                break;
                            };
                            let target = ReferenceWriteTarget {
                                base: base.clone(),
                                field: index,
                            };
                            if let Some(value) = self
                                .state
                                .writes
                                .get(&target)
                                .cloned()
                                .or_else(|| self.argument_slot_shape(base, index))
                            {
                                element.join(&value);
                            } else {
                                complete = false;
                            }
                        }
                        if !complete {
                            if let Some(template) = &buffer.array_element {
                                element.join(template);
                            } else {
                                element.join(&reference_shape(self.typed, &ty));
                            }
                        }
                        element
                    } else {
                        let mut element = ReferenceValue::default();
                        for field in &buffer.fields {
                            element.join(field);
                        }
                        element
                    };
                for source in &buffer.sources {
                    let origin = match source {
                        ReferenceSource::Argument(base) => {
                            crate::typed::ReferenceWriteOrigin::Slot(base.clone())
                        }
                        ReferenceSource::ArgumentElements(base) => {
                            crate::typed::ReferenceWriteOrigin::Elements(base.clone())
                        }
                        _ => continue,
                    };
                    if let Some(value) = self.state.unknown_slot_writes.get(&origin) {
                        element.join(value);
                    }
                }
                element
            }
            TypedExprKind::BufSet {
                buf, index, value, ..
            } => {
                let buffer = self.expr(buf);
                self.read(&buffer, &mut reads);
                self.expr(index);
                let value_ty = &self.typed.exprs.ty[value.index()];
                let scalar = reference_pointee_for_type(value_ty, Some(&self.typed.type_registry))
                    .is_none()
                    && self
                        .typed
                        .type_registry
                        .integer_validity_for_type(value_ty)
                        .is_some();
                let value = self.expr(value);
                let index = match target_index_for_expr(self.typed, index) {
                    crate::typed::TargetIndex::Symbolic(index) => index
                        .as_const_int()
                        .and_then(|index| usize::try_from(index).ok()),
                    crate::typed::TargetIndex::Unknown => None,
                };
                if !scalar {
                    if let Some(index) = index {
                        self.assign_field(buf, index, value);
                    } else if buffer.fields.is_empty() {
                        self.record_unknown_slot_write(&buffer, &value);
                    } else {
                        for (index, element) in buffer.fields.iter().enumerate() {
                            let mut joined = element.clone();
                            joined.join(&value);
                            self.assign_field(buf, index, joined);
                        }
                    }
                }
                ReferenceValue::default()
            }
            TypedExprKind::AggregateSet {
                base, index, value, ..
            } => {
                self.expr(base);
                let value = self.expr(value);
                self.assign_field(base, index, value);
                ReferenceValue::default()
            }
            TypedExprKind::RecordSet {
                base, field, value, ..
            } => {
                self.expr(base);
                let value = self.expr(value);
                if let Some(index) = super::effects::record_field_index(self.typed, base, field) {
                    self.assign_field(base, index, value);
                }
                ReferenceValue::default()
            }
            TypedExprKind::Deref(inner) => {
                let mut value = self.expr(inner);
                self.read(&value, &mut reads);
                value.sources.clear();
                value.loans.clear();
                value
            }
            TypedExprKind::Cast { expr, .. }
            | TypedExprKind::Rematerialize { source: expr, .. }
            | TypedExprKind::UnionPayload { expr, .. } => self.expr(expr),
            TypedExprKind::When(branch) => {
                if let Some(subject) = branch.subject {
                    let value = self.expr(subject);
                    self.read(&value, &mut reads);
                }
                let incoming = self.state.clone();
                let mut joined = incoming.clone();
                let mut result = ReferenceValue::default();
                for arm in branch.arms {
                    self.state = incoming.clone();
                    let body = match arm {
                        TypedWhenArm::Cond {
                            condition, body, ..
                        } => {
                            condition.visit_subjects(&mut |id| {
                                let value = self.expr(id);
                                self.read(&value, &mut reads);
                            });
                            body
                        }
                        TypedWhenArm::Is { body, .. } | TypedWhenArm::Else(body, _) => body,
                    };
                    result.join(&self.expr(body));
                    joined.join(&self.state, &self.write_origins);
                }
                self.state = joined;
                result
            }
            TypedExprKind::If(branch) => {
                branch.condition.visit_subjects(&mut |id| {
                    let value = self.expr(id);
                    self.read(&value, &mut reads);
                });
                let incoming = self.state.clone();
                self.sequence(&branch.stmts, branch.ret);
                self.capture_reference_writes();
                self.state = incoming;
                ReferenceValue::default()
            }
            TypedExprKind::Loop(body) => {
                let incoming = self.state.clone();
                match &body.kind {
                    crate::typed::TypedLoopKind::While { condition, .. } => {
                        condition.visit_subjects(&mut |id| {
                            let value = self.expr(id);
                            self.read(&value, &mut reads);
                        });
                    }
                    crate::typed::TypedLoopKind::ForIn { iterable, .. } => {
                        self.expr(*iterable);
                    }
                }
                if let crate::typed::TypedLoopKind::ForIn {
                    traversal: crate::typed::SequenceTraversal::IntegerRange { variable, .. },
                    ..
                } = &body.kind
                {
                    self.state.values.remove(variable);
                }
                for statements in body.statement_batches() {
                    let before = self.state.clone();
                    self.sequence(statements, None);
                    for statement in statements {
                        if let TypedExprKind::Bind { name, .. } =
                            self.typed.exprs.kind[statement.index()]
                        {
                            if let Some(place) = self.typed.exprs.place_of(*statement).flatten() {
                                if let Some(value) = before.storage.get(&place) {
                                    self.state.storage.insert(place, value.clone());
                                } else {
                                    self.state.storage.remove(&place);
                                }
                            }
                            if let Some(value) = before.values.get(&name) {
                                self.state.values.insert(name, value.clone());
                            } else {
                                self.state.values.remove(&name);
                            }
                        }
                    }
                }
                if !matches!(
                    body.kind,
                    crate::typed::TypedLoopKind::ForIn {
                        traversal: crate::typed::SequenceTraversal::Fixed { .. },
                        ..
                    }
                ) {
                    self.state.join(&incoming, &self.write_origins);
                }
                ReferenceValue::default()
            }
            _ => {
                let _ = walk_expr_children(&kind, &mut |child| {
                    self.expr(child);
                    ControlFlow::Continue(())
                });
                reference_shape(self.typed, &ty)
            }
        };
        let value = if reference_pointee_for_type(&ty, Some(&self.typed.type_registry)).is_some() {
            self.refresh_reference(value, &mut HashSet::new())
        } else {
            value
        };
        super::semantic_reads::reference_written(id);
        *self.typed.exprs.references_of_mut(id.index()) = value.clone();
        self.typed.exprs.facts.ownership_facts.reference_reads[id.index()] = reads;
        self.active.remove(&id);
        value
    }
}
