//! Type and number arguments a call leaves out.
//!
//! * `ArrayView.new(storage, len)`: a function of a generic type, called through the type
//!   name, has no receiver to read the type's parameters from.
//! * `identity(x)`: a free generic function written without its leading type arguments.
//!
//! Each missing argument is the one the value arguments determine (their types are unified
//! with the parameter types, numbers included: `[Byte; 16]` against `[element; capacity]`),
//! else the one the expected result type determines, else the call is an error naming the
//! parameter. Written arguments keep their meaning: a call whose argument count equals the
//! written parameter count is explicit, and each argument fewer omits one type parameter,
//! the leading ones first. `ArrayView(Byte, 16).new(...)` names every argument itself.
//!
//! The result is the same list of type arguments an explicit call has, so bounds, number
//! ranges, claims and the recording of the call see no difference.
use super::calls::{Function, Parameter, Passing};
use super::*;
use aggregates::EvalTask;

impl<'a> Checker<'a> {
    /// How many explicit type parameters this call omits, when it leaves any argument to be
    /// inferred. `None`: every type argument is written or comes from the receiver.
    pub(super) fn inference_plan(
        &self,
        interface: &Function,
        receiver: Option<&'a Expression>,
        written: usize,
    ) -> Option<usize> {
        if self.proof.forced_types.is_some() {
            return None;
        }
        let explicit = interface.parameters.len().checked_sub(interface.implicit_types)?;
        let supplied = written + usize::from(receiver.is_some());
        let omitted = explicit.checked_sub(supplied)?;
        let open_implicit = interface.implicit_types > 0 && receiver.is_none();
        (open_implicit || omitted > 0).then_some(omitted)
    }

    /// `Type(arguments).function`: a function of a generic type with the type written out.
    /// Yields the function and the type arguments the written type supplies.
    pub(super) fn applied_target(
        &mut self,
        callee: &'a Expression,
    ) -> Result<Option<(usize, Vec<SourceType>)>> {
        let Expr::Member { base, name } = &callee.value else {
            return Ok(None);
        };
        let Expr::Call { callee: head, .. } = &base.value else {
            return Ok(None);
        };
        let Expr::Name(head_name) = &head.value else {
            return Ok(None);
        };
        self.charge(self.proof.generics.len() + 1)?;
        if self.local_position(head_name)?.is_some()
            || self.proof.generics.contains(&head_name.as_str())
        {
            return Ok(None);
        }
        if self.lookup_symbol(head_name, false)?.is_none() {
            return Ok(None);
        }
        let scope = self.generic_scope();
        let ty = self.resolve_type(base, &scope)?;
        let (SourceType::Aggregate { id } | SourceType::Sum { id }) = ty else {
            return Err(SourceCheckError::Type);
        };
        self.child(callee.span_id, base.span_id)?;
        self.child(callee.span_id, name.span_id)?;
        self.name(name)?;
        let schema = self.module.instances[id as usize].schema();
        let owner = self.module.layouts[schema].schema.name.clone();
        let Some(function) = self.lookup_method(&owner, &name.value)? else {
            let detail = format!("`{owner}` has no function `{}`", name.value);
            return Err(self.explain(SourceCheckError::Type, detail));
        };
        Ok(Some((function, self.module.instances[id as usize].arguments.clone())))
    }

    pub(super) fn generic_scope(&self) -> Vec<(&'a str, SourceType)> {
        self.proof
            .generics
            .iter()
            .enumerate()
            .map(|(index, name)| (*name, SourceType::Generic { index: index as u32 }))
            .collect()
    }

    /// The generic parameters a template mentions.
    fn mentioned_generics(&mut self, root: SourceType, output: &mut Vec<u32>) -> Result<()> {
        let mut pending = vec![root];
        while let Some(ty) = pending.pop() {
            self.charge(1)?;
            match ty {
                SourceType::Generic { index } => output.push(index),
                SourceType::Aggregate { id }
                | SourceType::Sum { id }
                | SourceType::Array { id }
                | SourceType::Sequence { id }
                | SourceType::Pointer { id } => {
                    let arguments = &self.module.instances[id as usize].arguments;
                    self.charge(arguments.len())?;
                    pending.extend_from_slice(&self.module.instances[id as usize].arguments);
                }
                SourceType::Integer { .. } | SourceType::Unit | SourceType::Text => {}
            }
        }
        Ok(())
    }

    fn parameter_label(interface: &Function, index: usize) -> String {
        interface
            .signature
            .type_parameters
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("#{index}"))
    }

    /// A call whose type arguments are partly left to inference. The arguments are evaluated
    /// first, without the parameter types they would be viewed as; the second visit unifies
    /// those with the parameter types, fills what is left from `context` (the type the call
    /// is expected to yield) and then proceeds as an explicit call does.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn inferred_call_tasks(
        &mut self,
        expression: &'a Expression,
        interface: Rc<Function>,
        omitted: usize,
        context: Option<SourceType>,
        (receiver, arguments): (
            Option<&'a Expression>,
            Option<&'a Spanned<Vec<Spanned<Argument>>>>,
        ),
        visited: bool,
        pending: &mut Vec<EvalTask<'a>>,
        values: &mut Vec<Value>,
    ) -> Result<()> {
        let list: &[Spanned<Argument>] = arguments.map_or(&[], |arguments| &arguments.value);
        self.charge(self.proof.generics.len() + list.len() * 2)?;
        let scope = self.generic_scope();
        let count = interface
            .parameters
            .iter()
            .filter(|parameter| matches!(parameter, Parameter::Type))
            .count();
        let mut slots: Vec<Option<SourceType>> = vec![None; count];
        if interface.implicit_types > 0 && receiver.is_some() {
            let known = self.receiver_type_arguments(receiver, interface.implicit_types)?;
            for (slot, ty) in slots.iter_mut().zip(known) {
                *slot = Some(ty);
            }
        }
        let mut supplied: Vec<&'a Expression> = receiver.into_iter().collect();
        for argument in list {
            if let Some(arguments) = arguments {
                self.child(arguments.span_id, argument.span_id)?;
            }
            let Argument::Positional(value) = &argument.value else {
                return Err(SourceCheckError::Type);
            };
            self.child(argument.span_id, value.span_id)?;
            supplied.push(value);
        }
        let mut supplied = supplied.into_iter();
        let mut skip = omitted;
        let mut slot = interface.implicit_types;
        let mut expressions: Vec<(&'a Expression, SourceType)> = Vec::new();
        for parameter in interface.parameters.iter().skip(interface.implicit_types) {
            match parameter {
                Parameter::Type if skip > 0 => {
                    skip -= 1;
                    slot += 1;
                }
                Parameter::Type => {
                    let written = supplied.next().ok_or(SourceCheckError::Type)?;
                    slots[slot] = Some(self.resolve_type(written, &scope)?);
                    slot += 1;
                }
                Parameter::Value(template) => {
                    let written = supplied.next().ok_or(SourceCheckError::Type)?;
                    expressions.push((written, *template));
                }
                Parameter::Literal => {
                    supplied.next().ok_or(SourceCheckError::Type)?;
                }
            }
        }
        if skip > 0 || supplied.next().is_some() {
            return Err(SourceCheckError::Type);
        }
        let passing: Vec<Passing> = interface
            .parameters
            .iter()
            .zip(&interface.passing)
            .filter(|(parameter, _)| matches!(parameter, Parameter::Value(_)))
            .map(|(_, passing)| *passing)
            .collect();
        if !visited {
            self.charge(expressions.len() + 1)?;
            self.check_call_borrows(&expressions, &passing)?;
            pending.push(EvalTask::Expression(expression, context, true));
            for ((argument, template), passing) in expressions.into_iter().zip(passing).rev() {
                // What the parameter's type does not depend on an open argument for is known.
                let mut mentioned = Vec::new();
                self.mentioned_generics(template, &mut mentioned)?;
                let closed = mentioned
                    .iter()
                    .all(|index| slots.get(*index as usize).is_some_and(Option::is_some));
                let expected = if closed {
                    let known: Vec<SourceType> = slots
                        .iter()
                        .map(|slot| slot.unwrap_or(SourceType::Unit))
                        .collect();
                    Some(self.substitute(template, &known)?)
                } else {
                    None
                };
                let place = passing.borrows() && self.is_place(argument)?;
                if place {
                    pending.push(EvalTask::Borrow(argument, passing == Passing::Mut));
                } else {
                    pending.push(EvalTask::Expression(argument, expected, false));
                }
            }
            return Ok(());
        }
        if values.len() < expressions.len() {
            return Err(SourceCheckError::Malformed);
        }
        let mut actual = values.split_off(values.len() - expressions.len());
        // Arguments first: they are what the call is given.
        for (position, ((_, template), value)) in expressions.iter().zip(&actual).enumerate() {
            let ty = value
                .sum
                .map(|id| SourceType::Sum { id })
                .or(value.structural)
                .unwrap_or(value.ty);
            let mut mentioned = Vec::new();
            self.mentioned_generics(*template, &mut mentioned)?;
            if mentioned.is_empty() {
                continue;
            }
            if self.infer_argument(*template, ty, &mut slots).is_err() {
                let detail = format!(
                    "argument {} of `{}` has type `{}`, which does not agree with the type or number arguments the other arguments give",
                    position + 1,
                    interface.signature.name,
                    self.type_label(ty),
                );
                return Err(self.explain(SourceCheckError::Type, detail));
            }
        }
        // Then the type the call is expected to yield.
        if slots.iter().any(Option::is_none)
            && let Some(context) = context
        {
            let mut trial = slots.clone();
            if self
                .infer_argument(interface.signature.result, context, &mut trial)
                .is_ok()
            {
                slots = trial;
            }
        }
        let mut types = Vec::with_capacity(slots.len());
        for (index, slot) in slots.iter().enumerate() {
            let Some(ty) = slot else {
                let name = Self::parameter_label(&interface, index);
                let detail = format!(
                    "cannot infer `{name}` for `{}`: no argument and no expected type determines it, so write it",
                    interface.signature.name,
                );
                return Err(self.explain(SourceCheckError::Type, detail));
            };
            types.push(*ty);
        }
        self.check_inferred_numbers(&interface, &types)?;
        self.check_type_bounds(&interface.signature.type_bounds, &types)?;
        let constraints = self.number_constraints(&interface.constraints, &types)?;
        for ((_, template), value) in expressions.iter().zip(&mut actual) {
            let expected = self.substitute(*template, &types)?;
            *value = self.view_as(*value, expected)?;
        }
        self.constraints(&constraints, &mut actual, false)?;
        let result = self.substitute(interface.signature.result, &types)?;
        self.note_call(expression, &types, result);
        let value = self.apply_outcomes(&interface.summaries, &actual, &types)?;
        self.apply_effects(interface.effects.as_deref(), &expressions, &passing, &actual)?;
        let value = self.view_as(value, result)?;
        values.push(self.attach_result_loans(&interface, &expressions, &actual, value)?);
        Ok(())
    }

    /// The numbers inferred for a generic type's parameters lie within the ranges it declares.
    fn check_inferred_numbers(&mut self, interface: &Function, types: &[SourceType]) -> Result<()> {
        if interface.implicit_types == 0 {
            return Ok(());
        }
        let Some((owner, _)) = interface.signature.name.rsplit_once('.') else {
            return Ok(());
        };
        self.charge(self.module.type_names.len() + 1)?;
        let Some(schema) = self
            .module
            .type_names
            .iter()
            .find(|(name, _)| name == owner)
            .map(|(_, schema)| *schema)
        else {
            return Ok(());
        };
        self.check_number_arguments(schema, &types[..interface.implicit_types.min(types.len())])
    }
}
