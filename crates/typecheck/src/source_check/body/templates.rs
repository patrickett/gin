//! Template bodies: a generic function, or a method of a generic type, is recorded once with its
//! type parameters left as `SourceType::Generic`. Lowering gives each concrete instantiation its
//! own copy by substituting types into the recorded graph.
//!
//! The recorder does not resolve a type for a template: the checker already did, at each call
//! and construction, and left what it settled in `ProofState::call_info`, which is read here by
//! the expression's span. What it needs beyond that (the type of a field of a generic record, the
//! parameter types a callee expects once its type parameters are applied) is substituted without
//! charging or interning, so recording never changes what the checker accepts: a type the checker
//! did not already intern leaves the body unrecorded.
use super::*;
use state::CallInfo;

impl<'a> Checker<'a> {
    /// The checker applied a generic callee at `expression`.
    pub(in crate::source_check) fn note_call(
        &mut self,
        expression: &Expression,
        types: &[SourceType],
        result: SourceType,
    ) {
        if types.is_empty() {
            return;
        }
        let info = CallInfo {
            types: types.to_vec(),
            result,
        };
        self.proof.call_info.push((expression.span_id, info));
    }

    /// The checker built a value of the generic type `ty` at `expression`.
    pub(in crate::source_check) fn note_construct(&mut self, expression: &Expression, ty: SourceType) {
        let (SourceType::Aggregate { id } | SourceType::Sum { id }) = ty else {
            return;
        };
        if self.module.instances[id as usize].arguments.is_empty() {
            return;
        }
        let info = CallInfo {
            types: Vec::new(),
            result: ty,
        };
        self.proof.call_info.push((expression.span_id, info));
    }

    pub(super) fn info_of(&self, span: SpanId) -> Option<&CallInfo> {
        self.proof
            .call_info
            .iter()
            .rev()
            .find(|(other, _)| *other == span)
            .map(|(_, info)| info)
    }

    /// `root` with each `Generic{i}` replaced by `types[i]`. Uncharged and read-only: an
    /// instance that does not exist already is `None`.
    pub(super) fn peek_substitute(&self, root: SourceType, types: &[SourceType]) -> Option<SourceType> {
        self.peek_substituted(root, types, 0)
    }

    fn peek_substituted(&self, root: SourceType, types: &[SourceType], depth: usize) -> Option<SourceType> {
        if depth > RECORD_DEPTH_MAX {
            return None;
        }
        let id = match root {
            SourceType::Generic { index } => return types.get(index as usize).copied(),
            SourceType::Aggregate { id }
            | SourceType::Sum { id }
            | SourceType::Array { id }
            | SourceType::Sequence { id }
            | SourceType::Pointer { id } => id,
            _ => return Some(root),
        };
        let instance = self.module.instances.get(id as usize)?;
        if instance.arguments.is_empty() {
            return Some(root);
        }
        let arguments = instance
            .arguments
            .iter()
            .map(|argument| self.peek_substituted(*argument, types, depth + 1))
            .collect::<Option<Vec<_>>>()?;
        let head = instance.head;
        let found = self
            .module
            .instance_index
            .binary_search_by(|other| {
                let other = &self.module.instances[*other];
                other
                    .head
                    .cmp(&head)
                    .then_with(|| other.arguments.cmp(&arguments))
            })
            .ok()?;
        Some(self.head_type(head, self.module.instance_index[found] as u32))
    }

    /// A call that applies type parameters: `callee<types>(arguments)`. The receiver, if any,
    /// is the first value; type parameters are written in the argument list and are not values.
    pub(super) fn record_generic_invocation(
        &mut self,
        expression: &'a Expression,
        (function, interface): (usize, &Rc<calls::Function>),
        (receiver, arguments, parameter_count, depth): (
            Option<u32>,
            &'a [Spanned<Argument>],
            usize,
            usize,
        ),
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        let receiver_place = self.recording_receiver.take();
        let mutated = &interface.signature.mutated;
        if interface.signature.instruction.is_some()
            || (!mutated.is_empty() && (receiver.is_some() && receiver_place.is_none()))
        {
            return None;
        }
        let info = self.info_of(expression.span_id)?.clone();
        if info.types.len() != interface.signature.type_parameters.len() {
            return None;
        }
        let written = interface.parameters.get(interface.implicit_types..)?;
        // Type parameters the call left to inference (leading ones first) have no argument.
        let mut omitted = written
            .len()
            .checked_sub(arguments.len() + usize::from(receiver.is_some()))?;
        if omitted > written.iter().filter(|p| matches!(p, calls::Parameter::Type)).count() {
            return None;
        }
        let mut ids = Vec::new();
        let mut places: Vec<Option<&'a Expression>> = Vec::new();
        let first_is_receiver = receiver.is_some();
        let mut position = 0;
        for parameter in written.iter() {
            if omitted > 0 && matches!(parameter, calls::Parameter::Type) {
                omitted -= 1;
                continue;
            }
            position += 1;
            let position = position - 1;
            // The receiver, when there is one, stands for the first written parameter.
            let argument = match (position, receiver) {
                (0, Some(node)) => Err(node),
                _ => Ok(&arguments[position - usize::from(first_is_receiver)]),
            };
            match parameter {
                // A type is no value: it is in `info.types`.
                calls::Parameter::Type if argument.is_ok() => {}
                calls::Parameter::Type | calls::Parameter::Literal => return None,
                calls::Parameter::Value(template) => {
                    places.push(match argument {
                        Err(_) => receiver_place,
                        Ok(argument) => match &argument.value {
                            Argument::Positional(value) => Some(value),
                            _ => None,
                        },
                    });
                    let node = match argument {
                        Err(node) => node,
                        Ok(argument) => {
                            let Argument::Positional(value) = &argument.value else {
                                return None;
                            };
                            self.record_expr(value, parameter_count, depth + 1, arena, scope)?
                        }
                    };
                    let expected = self.peek_substitute(*template, &info.types)?;
                    ids.push(self.coerce(node, expected, expression.span_id, arena)?);
                }
            }
        }
        let callee = self.callee_ref(function)?;
        let kind = CheckedSourceBodyKind::Call {
            callee,
            arguments: ids,
            type_arguments: info.types.clone(),
        };
        let call = push(arena, expression.span_id, info.result, kind);
        if !mutated.is_empty() {
            // What a `mut` parameter hands back is its type with this call's type arguments.
            let mut tys = Vec::new();
            for &parameter in mutated {
                let template = *interface.signature.parameters.get(parameter as usize)?;
                if !matches!(template, SourceType::Integer { .. } | SourceType::Aggregate { .. }) {
                    return None;
                }
                tys.push(self.peek_substitute(template, &info.types)?);
            }
            self.record_handed_back(expression.span_id, call, mutated, &tys, &places, arena, scope)?;
        }
        Some(call)
    }

    /// `element.release(value)` or `value.release` where `element` is a type parameter bound to
    /// a trait with that method. Outer `None`: this is not a bound call. `Some(None)`: it is, and
    /// cannot be recorded.
    pub(super) fn record_bound_call(
        &mut self,
        expression: &'a Expression,
        (base, method): (&'a Expression, &str),
        arguments: &'a [Spanned<Argument>],
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<Option<u32>> {
        let Expr::Name(head) = &base.value else {
            return None;
        };
        if self.proof.bounds.is_empty() {
            return None;
        }
        // A value of the type parameter's type, else the parameter's own name.
        let value = scope
            .iter()
            .rev()
            .find(|(local, _)| *local == head.as_str())
            .map(|(_, node)| arena[*node as usize].ty)
            .or_else(|| {
                let position = self
                    .proof
                    .names
                    .iter()
                    .position(|(other, _)| *other == head.as_str())
                    .filter(|position| *position < parameter_count)?;
                Some(self.proof.names[position].1.ty)
            });
        let index = match value {
            Some(SourceType::Generic { index }) => index,
            Some(_) => return None,
            None => self.proof.generics.iter().position(|name| *name == head.as_str())? as u32,
        };
        let interface = self.bound_method(index, method)?;
        Some(self.record_bound(expression, (base, head, value.is_some()), (index, method, &interface), arguments, parameter_count, depth, arena, scope))
    }

    /// The bodiless method `method` of a trait that type parameter `index` is bound to.
    fn bound_method(&self, index: u32, method: &str) -> Option<Rc<calls::Function>> {
        self.proof
            .bounds
            .iter()
            .filter(|(generic, _)| *generic == index)
            .find_map(|(_, bound)| {
                let trait_name = match &bound.value {
                    Expr::Name(name) => name.as_str(),
                    Expr::Call { callee, .. } => match &callee.value {
                        Expr::Name(name) => name.as_str(),
                        _ => return None,
                    },
                    _ => return None,
                };
                let (_, interface) = self.recorded_callee(&format!("{trait_name}.{method}"))?;
                Some(interface)
            })
    }

    fn record_bound(
        &mut self,
        expression: &'a Expression,
        (base, head, is_value): (&'a Expression, &'a str, bool),
        (index, method, interface): (u32, &str, &Rc<calls::Function>),
        arguments: &'a [Spanned<Argument>],
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        let mutated = &interface.signature.mutated;
        let info = self.info_of(expression.span_id)?.clone();
        if info.types.first() != Some(&SourceType::Generic { index }) {
            return None;
        }
        // Which function this reaches is unknown here, so it may run an instruction.
        self.recording_effects = true;
        let mut ids = Vec::new();
        if is_value {
            ids.push(self.record_name(head, base.span_id, parameter_count, arena, scope)?);
        }
        // A type written among the arguments (`allocator.reserve(element, count)`) is no value:
        // the checker settled it into `info.types`.
        let written = interface.parameters.get(interface.implicit_types..)?;
        let skip = usize::from(is_value);
        for (position, argument) in arguments.iter().enumerate() {
            if matches!(written.get(position + skip), Some(calls::Parameter::Type)) {
                continue;
            }
            let Argument::Positional(value) = &argument.value else {
                return None;
            };
            ids.push(self.record_expr(value, parameter_count, depth + 1, arena, scope)?);
        }
        let templates: Vec<SourceType> = interface
            .parameters
            .iter()
            .filter_map(|parameter| match parameter {
                calls::Parameter::Value(ty) => Some(*ty),
                _ => None,
            })
            .collect();
        if templates.len() != ids.len() {
            return None;
        }
        for (id, template) in ids.iter_mut().zip(templates) {
            let expected = self.peek_substitute(template, &info.types)?;
            *id = self.coerce(*id, expected, expression.span_id, arena)?;
        }
        let kind = CheckedSourceBodyKind::BoundCall {
            generic: index,
            method: method.to_owned(),
            trait_arguments: info.types[1..].to_vec(),
            arguments: ids,
        };
        let call = push(arena, expression.span_id, info.result, kind);
        if !mutated.is_empty() {
            // The places lent `mut` now hold what the call handed back, as for any call.
            let mut places: Vec<Option<&'a Expression>> = Vec::new();
            if is_value {
                places.push(Some(base));
            }
            for (position, argument) in arguments.iter().enumerate() {
                if matches!(written.get(position + skip), Some(calls::Parameter::Type)) {
                    continue;
                }
                places.push(match &argument.value {
                    Argument::Positional(value) => Some(value),
                    _ => None,
                });
            }
            let mut tys = Vec::new();
            for &parameter in mutated {
                let template = *interface.signature.parameters.get(parameter as usize)?;
                if !matches!(template, SourceType::Integer { .. } | SourceType::Aggregate { .. } | SourceType::Generic { .. }) {
                    return None;
                }
                tys.push(self.peek_substitute(template, &info.types)?);
            }
            self.record_handed_back(expression.span_id, call, mutated, &tys, &places, arena, scope)?;
        }
        Some(call)
    }

    /// Uncharged: the schema a generic record (or a variant a union declares) is declared by,
    /// when `name` is not a type without parameters.
    pub(super) fn recorded_generic_schema(&self, name: &str) -> Option<usize> {
        if name == "Self" {
            let SourceType::Aggregate { id } = self.proof.receiver? else {
                return None;
            };
            let schema = self.module.instances.get(id as usize)?.schema();
            let layout = &self.module.layouts.get(schema)?.schema;
            return (!layout.parameters.is_empty() && layout.variants.is_empty()).then_some(schema);
        }
        let symbols = &self.module.type_names;
        if let Ok(position) = symbols.binary_search_by(|(other, _)| other.as_str().cmp(name)) {
            let schema = symbols[position].1;
            let layout = &self.module.layouts.get(schema)?.schema;
            return (!layout.parameters.is_empty() && layout.variants.is_empty()).then_some(schema);
        }
        // A variant a union declared is `Union.Variant`: found through the bare name when unique.
        let mut found = None;
        for (index, layout) in self.module.layouts.iter().enumerate() {
            let schema = &layout.schema;
            let declared = schema.variants.is_empty()
                && !schema.parameters.is_empty()
                && schema.name.rsplit_once('.').is_some_and(|(_, last)| last == name);
            if declared && found.replace(index).is_some() {
                return None;
            }
        }
        found
    }

    /// `Name(fields)` building a value of a generic type: the type is the one the checker gave it.
    pub(super) fn record_generic_construct(
        &mut self,
        expression: &'a Expression,
        name: &str,
        arguments: &'a Spanned<Vec<Spanned<Argument>>>,
        parameter_count: usize,
        depth: usize,
        arena: &mut Vec<CheckedSourceBodyExpr>,
        scope: &mut Scope<'a>,
    ) -> Option<u32> {
        let schema = self
            .variant_built_here(expression, name)
            .or_else(|| self.recorded_generic_schema(name))?;
        let ty = self.info_of(expression.span_id)?.result;
        let types = self.field_types_of(ty, schema)?;
        let names: Vec<String> = self.module.layouts[schema]
            .schema
            .fields
            .iter()
            .map(|field| field.name.clone())
            .collect();
        if names.len() != arguments.value.len() {
            return None;
        }
        // A variant's payload has no field name: values go by position.
        let positional = names.iter().all(String::is_empty);
        let mut fields: Vec<Option<u32>> = vec![None; names.len()];
        for (position, argument) in arguments.value.iter().enumerate() {
            let (index, value) = if positional {
                let Argument::Positional(value) = &argument.value else {
                    return None;
                };
                (position, value)
            } else {
                let (field, value) = match &argument.value {
                    Argument::Named { name, value } => (name.value.as_str(), value),
                    Argument::Typed { .. } => return None,
                    // A variant's first payload is positional even when it has a name.
                    Argument::Positional(value) if position == 0 && self.is_declared_variant(schema) => {
                        (names.first()?.as_str(), value)
                    }
                    Argument::Positional(value) => match &value.value {
                        Expr::Name(field) => (field.as_str(), value),
                        _ => return None,
                    },
                };
                (names.iter().position(|candidate| candidate == field)?, value)
            };
            let id = self.record_expr(value, parameter_count, depth + 1, arena, scope)?;
            let id = self.coerce(id, *types.get(index)?, value.span_id, arena)?;
            if fields[index].replace(id).is_some() {
                return None;
            }
        }
        let fields = fields.into_iter().collect::<Option<Vec<_>>>()?;
        Some(push(
            arena,
            expression.span_id,
            ty,
            CheckedSourceBodyKind::Construct {
                schema: schema as u32,
                fields,
            },
        ))
    }

    /// The type arguments of the member `schema` as it is held by a value of type `ty`: `ty`
    /// itself when it is that member, else the member of the sum `ty`.
    fn member_arguments(&self, ty: SourceType, schema: usize) -> Option<Vec<SourceType>> {
        match ty {
            SourceType::Aggregate { id } => {
                let instance = self.module.instances.get(id as usize)?;
                (matches!(instance.head, CheckedSourceInstanceHead::Nominal { .. })
                    && instance.schema() == schema)
                    .then(|| instance.arguments.clone())
            }
            SourceType::Sum { id } => {
                let sum = self.module.instances.get(id as usize)?;
                let CheckedSourceInstanceHead::Nominal { schema: sum_schema } = sum.head else {
                    return None;
                };
                for variant in &self.module.layouts.get(sum_schema as usize)?.schema.variants {
                    let SourceType::Aggregate { id: member } = variant else {
                        continue;
                    };
                    let member_instance = self.module.instances.get(*member as usize)?;
                    if !matches!(member_instance.head, CheckedSourceInstanceHead::Nominal { .. })
                        || member_instance.schema() != schema
                    {
                        continue;
                    }
                    let SourceType::Aggregate { id } = self.peek_substitute(*variant, &sum.arguments)? else {
                        return None;
                    };
                    return Some(self.module.instances.get(id as usize)?.arguments.clone());
                }
                None
            }
            _ => None,
        }
    }

    /// The types of the fields of `schema` in a value of type `ty`.
    pub(super) fn field_types_of(&self, ty: SourceType, schema: usize) -> Option<Vec<SourceType>> {
        let layout = &self.module.layouts.get(schema)?.schema;
        if layout.parameters.is_empty() {
            return Some(layout.fields.iter().map(|field| field.ty).collect());
        }
        let arguments = self.member_arguments(ty, schema)?;
        layout
            .fields
            .iter()
            .map(|field| self.peek_substitute(field.ty, &arguments))
            .collect()
    }

    /// `record.field` of a generic record.
    pub(super) fn record_generic_field(
        &mut self,
        expression: &'a Expression,
        record: u32,
        name: &str,
        arena: &mut Vec<CheckedSourceBodyExpr>,
    ) -> Option<u32> {
        self.record_generic_field_named(expression.span_id, record, name, arena)
    }

    pub(super) fn record_generic_field_named(
        &mut self,
        span: SpanId,
        record: u32,
        name: &str,
        arena: &mut Vec<CheckedSourceBodyExpr>,
    ) -> Option<u32> {
        let ty = arena[record as usize].ty;
        let SourceType::Aggregate { id } = ty else {
            return None;
        };
        let schema = self.module.instances[id as usize].schema();
        let layout = &self.module.layouts[schema].schema;
        if layout.kind == AggregateKind::Domain {
            return None;
        }
        let index = layout.fields.iter().position(|field| field.name == name)?;
        let field = self.field_types_of(ty, schema)?[index];
        if let CheckedSourceBodyKind::Construct { fields, .. } = &arena[record as usize].kind {
            return fields.get(index).copied();
        }
        Some(push(
            arena,
            span,
            field,
            CheckedSourceBodyKind::Field {
                record,
                index: u32::try_from(index).ok()?,
            },
        ))
    }

    /// The generic sum whose members include the type of every node, for the value a `when`
    /// over members yields. The function's own result type wins when several would do.
    pub(super) fn sum_of_generic(
        &self,
        arena: &[CheckedSourceBodyExpr],
        nodes: impl Iterator<Item = u32> + Clone,
    ) -> Option<SourceType> {
        let mut found: Vec<SourceType> = Vec::new();
        for (id, instance) in self.module.instances.iter().enumerate() {
            let CheckedSourceInstanceHead::Nominal { schema } = instance.head else {
                continue;
            };
            let Some(layout) = self.module.layouts.get(schema as usize) else {
                continue;
            };
            let variants = &layout.schema.variants;
            if variants.is_empty() || instance.arguments.is_empty() {
                continue;
            }
            let joined = self.head_type(instance.head, id as u32);
            let members: Option<Vec<SourceType>> = variants
                .iter()
                .map(|variant| self.peek_substitute(*variant, &instance.arguments))
                .collect();
            let Some(members) = members else {
                continue;
            };
            let holds = nodes.clone().all(|node| {
                let ty = arena[node as usize].ty;
                members.contains(&ty) || ty == joined
            });
            if holds {
                found.push(joined);
            }
        }
        match (found.len(), self.recording_result) {
            (_, Some(result)) if found.contains(&result) => Some(result),
            (1, _) => found.first().copied(),
            _ => None,
        }
    }
}
