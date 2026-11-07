//! Ordinary calls consume only verified interfaces, including for local forward calls.
use super::*;
use aggregates::EvalTask;
use contracts::Constraint;
use diagnostics::{DiagnosticResult, FunctionDeclaration};

/// The names a place is reached by, root first: `a.b.c` is `[a, b, c]`.
fn place_path(expression: &Expression) -> Option<Vec<&str>> {
    match &expression.value {
        Expr::Name(name) => Some(vec![name.as_str()]),
        Expr::Member { base, name } => {
            let mut path = place_path(base)?;
            path.push(name.value.as_str());
            Some(path)
        }
        _ => None,
    }
}

/// How a value parameter takes its argument. An owner is the default; `end` is an owner
/// that must also be ended here; `ref` and `mut` borrow and never take ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Passing {
    Owner,
    End,
    Ref,
    Mut,
}

impl Passing {
    pub(super) fn borrows(self) -> bool {
        matches!(self, Self::Ref | Self::Mut)
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Parameter {
    Type,
    Value(SourceType),
    /// The unannotated parameter of a `#instruction` function: a string literal written
    /// at each call, which becomes the instruction's operand. It is never a value, so
    /// string literals have no type and exist nowhere else.
    Literal,
}

#[derive(Debug, Clone)]
pub(super) struct Function {
    pub signature: CheckedSourceFunction,
    pub parameters: Vec<Parameter>,
    /// How each parameter takes its argument, parallel to `parameters`.
    pub passing: Vec<Passing>,
    pub constraints: Vec<Constraint>,
    /// Claims the function proves where it returns, assumed by a caller about the result.
    pub postconditions: Vec<Constraint>,
    /// Claims of each member of a result written as variants, assumed in that member's world.
    pub variant_postconditions: Vec<(SourceType, Vec<Constraint>)>,
    /// The `::` transitions of each member of such a result: the caller forgets the place at
    /// the call and learns its value, worked out from the call's arguments, where it matches it.
    pub variant_effects: Vec<(SourceType, Vec<(postconditions::PlacePath, postconditions::ClauseExpr)>)>,
    /// How many leading `Type` parameters were not written: a generic type's own parameters,
    /// which a method that takes `self` reads from the receiver's type.
    pub implicit_types: usize,
    pub summaries: Vec<summaries::Summary>,
    /// Executable body for the real world, when the recorder represented it. Shared and
    /// immutable, so a capsule hands it to every importer without copying it.
    pub body: Option<std::sync::Arc<CheckedSourceBody>>,
    /// What a call does to the places it lends `mut`, when the function says. `None`:
    /// callers forget what they lent.
    pub effects: Option<Vec<postconditions::Effect>>,
    /// Written `ref T` / `mut T`: the result is a borrow, not a value.
    pub result_borrow: Option<BorrowKind>,
    /// The arguments (among the value parameters) the result stays borrowed from.
    pub result_sources: Vec<borrowing_results::ResultSource>,
    /// Whole body is `return #lower(r)` / `#upper(r)`: a call that knows `r` answers exactly.
    pub bound_query: Option<range_bounds::BoundQuery>,
    /// Why the recorder declined the body, when it did.
    pub decline: Option<BodyDecline>,
    /// Calls the recorder replaced by their value, by the callee they would have reached.
    pub folds: Vec<CheckedSourceCallee>,
}

struct Arguments<'a> {
    types: Vec<SourceType>,
    expressions: Vec<(&'a Expression, SourceType)>,
}

impl Function {
    pub(super) fn prepay(&self, remaining: &mut u32) -> Result<()> {
        prepay_signature(&self.signature, remaining)?;
        prepay(
            remaining,
            self.parameters.len() + self.constraints.len() + self.summaries.len() + 1,
        )?;
        for summary in &self.summaries {
            prepay(
                remaining,
                summary.inputs.len()
                    + summary.roots.len()
                    + summary.dispositions.len()
                    + summary.outputs.len()
                    + summary.integer_guards.len(),
            )?;
            for node in &summary.inputs {
                prepay(remaining, node.children.len() + node.slots.len())?;
            }
            for node in &summary.outputs {
                prepay(remaining, node.children.len() + node.slots.len())?;
            }
        }
        Ok(())
    }
}

pub(super) fn prepay_signature(
    signature: &CheckedSourceFunction,
    remaining: &mut u32,
) -> Result<()> {
    prepay(
        remaining,
        signature.name.len() + signature.parameters.len() + signature.type_parameters.len() + 1,
    )?;
    for name in &signature.type_parameters {
        prepay(remaining, name.len())?;
    }
    Ok(())
}

impl<'a> Checker<'a> {
    pub(super) fn publish_function(
        &mut self,
        binding: &'a Binding,
        parameters: Vec<SourceType>,
        result: SourceType,
    ) -> Result<Function> {
        // A parameter that only adds a bound to the receiver's type parameter is no role.
        let added = std::mem::take(&mut self.proof.added_bounds);
        let written: Vec<&Spanned<ast::Parameter>> = binding
            .parameters
            .iter()
            .flat_map(|p| &p.value)
            .filter(|p| {
                !(matches!(p.annotation.as_ref().map(|a| &a.value), Some(Annotation::Bound { .. }))
                    && added.contains(&p.name.value.as_str()))
            })
            .collect();
        self.charge(written.len() * 2 + binding.name.value.len())?;
        let positions = std::mem::take(&mut self.proof.parameter_roles);
        let mut passing = std::mem::take(&mut self.proof.parameter_passing);
        passing.resize(positions.len(), Passing::Owner);
        let implicit = positions
            .len()
            .checked_sub(written.len())
            .ok_or(SourceCheckError::Malformed)?;
        if !positions[..implicit]
            .iter()
            .all(|position| matches!(position, Parameter::Type))
        {
            return Err(SourceCheckError::Malformed);
        }
        let mut names = Vec::new();
        let mut annotations = Vec::new();
        for (index, (parameter, position)) in
            written.iter().zip(&positions[implicit..]).enumerate()
        {
            // Only a method's bare receiver and an instruction's literal omit an annotation.
            let receiver = index == 0
                && self.proof.receiver.is_some()
                && modules::receiver_annotation(&parameter.annotation)
                && parameter.name.value == "self";
            let bare_type = matches!(position, Parameter::Type) && parameter.annotation.is_none();
            // A receiver's own claims (`self and self.length < cap`) are checked at each call.
            let claims = receiver && parameter.annotation.is_some();
            let annotation = if claims {
                parameter.annotation.as_ref()
            } else if receiver || bare_type || matches!(position, Parameter::Literal) {
                None
            } else {
                Some(
                    parameter
                        .annotation
                        .as_ref()
                        .ok_or(SourceCheckError::Type)?,
                )
            };
            if matches!(position, Parameter::Value(_)) {
                names.push(parameter.name.value.as_str());
                annotations.push(annotation);
            }
        }
        let mut constraints = Vec::new();
        for (index, annotation) in annotations.iter().enumerate() {
            if let Some(annotation) = annotation {
                self.compile_constraints(annotation, index, &names, &mut constraints)?;
            }
        }
        let name = match self.proof.receiver {
            _ if self.proof.abstract_trait.is_some() => format!(
                "{}.{}",
                self.proof.abstract_trait.map_or("", |t| t.name.value.as_str()),
                binding.name.value
            ),
            Some(SourceType::Aggregate { id } | SourceType::Sum { id }) => {
                let schema = self.module.instances[id as usize].schema();
                format!(
                    "{}.{}",
                    self.module.layouts[schema].schema.name, binding.name.value
                )
            }
            _ => binding.name.value.clone(),
        };
        let signature = CheckedSourceFunction {
            provided_by: None,
            instruction: self.instruction_of(binding),
            memory: self.memory_of(binding),
            literals: self
                .proof
                .literals
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            infix: self.proof.infix,
            private: self.proof.receiver.is_some()
                && binding
                    .attributes
                    .iter()
                    .any(|attribute| matches!(attribute.value, FieldAttribute::Private { .. })),
            name,
            span: binding.name.span_id,
            parameters,
            mutated: positions
                .iter()
                .zip(&passing)
                .filter(|(position, _)| matches!(position, Parameter::Value(_)))
                .enumerate()
                .filter(|(_, (_, passing))| **passing == Passing::Mut)
                .map(|(index, _)| index as u32)
                .collect(),
            type_parameters: self.checked_type_parameters()?,
            type_bounds: self.checked_type_bounds(),
            result,
        };
        if let Some(op) = signature.memory {
            // `load(element Type, pointer @element, index N) element`; `store` adds the value
            // and answers nothing. The pointer and the index are values, the element a type.
            let shape = match (op, positions.as_slice()) {
                (
                    MemoryOp::Load,
                    [Parameter::Type, Parameter::Value(SourceType::Pointer { .. }), Parameter::Value(_)],
                ) => true,
                (
                    MemoryOp::Store,
                    [Parameter::Type, Parameter::Value(SourceType::Pointer { .. }), Parameter::Value(_), Parameter::Value(_)],
                ) => signature.result == SourceType::Unit,
                _ => false,
            };
            if !shape {
                let detail = "`#load` marks `name(element Type, pointer @element, index N) element` and `#store` marks `name(element Type, pointer @element, index N, value element)`".to_owned();
                return Err(self.explain(SourceCheckError::InstructionTarget, detail));
            }
        }
        let summary = self
            .proof
            .summary
            .take()
            .ok_or(SourceCheckError::Malformed)?;
        let result_sources = self.published_result_sources(
            binding.value.is_none(),
            signature.result,
            &positions,
            &passing,
        );
        Ok(Function {
            signature,
            parameters: positions,
            passing,
            constraints,
            postconditions: std::mem::take(&mut self.proof.postconditions),
            variant_postconditions: std::mem::take(&mut self.proof.variant_postconditions),
            variant_effects: std::mem::take(&mut self.proof.variant_effects),
            implicit_types: implicit,
            summaries: std::iter::once(summary)
                .chain(std::mem::take(&mut self.proof.member_summaries))
                .collect(),
            body: None,
            effects: self.proof.effects.take(),
            result_borrow: self.proof.result_borrow,
            result_sources,
            bound_query: self.bound_query_of(binding),
            decline: None,
            folds: Vec::new(),
        })
    }

    pub(super) fn call_target(&mut self, callee: &'a Expression) -> Result<Option<usize>> {
        let Expr::Name(name) = &callee.value else {
            return Ok(None);
        };
        self.token(callee.span_id, name)?;
        let function = self.lookup_symbol(name, true)?;
        if function.is_some() {
            self.charge(self.proof.generics.len())?;
            if self.local_position(name)?.is_some() || self.proof.generics.contains(&name.as_str())
            {
                return Err(SourceCheckError::Type);
            }
        }
        Ok(function)
    }

    /// `local.method(...)` on a bound record, or `Type.function(...)` on a type name.
    /// Yields the function and the receiver expression, which a type path lacks.
    /// Other receivers are not resolved yet.
    pub(super) fn method_target(
        &mut self,
        callee: &'a Expression,
    ) -> Result<Option<(usize, Option<&'a Expression>)>> {
        let Expr::Member { base, name } = &callee.value else {
            return Ok(None);
        };
        let (schema, receiver) = match &base.value {
            Expr::Name(head) => {
                let (schema, receiver) = if let Some(position) = self.local_position(head)? {
                    let (SourceType::Aggregate { id } | SourceType::Sum { id }) =
                        self.proof.names[position].1.ty
                    else {
                        return Ok(None);
                    };
                    (self.module.instances[id as usize].schema(), Some(&**base))
                } else if let Some(schema) = self.lookup_symbol(head, false)? {
                    (schema, None)
                } else {
                    return Ok(None);
                };
                self.token(base.span_id, head)?;
                (schema, receiver)
            }
            // `Union.Variant.method`: the receiver is a variant written out, and the method
            // is the union's.
            Expr::Member {
                base: union_base, ..
            } if self.names_union(union_base)? => {
                let Expr::Name(union) = &union_base.value else {
                    return Ok(None);
                };
                (self.schema(union)?, Some(&**base))
            }
            // A field of a record: the method is found on the field's own type.
            Expr::Member { .. } => {
                let field = match self.project_place(base) {
                    Ok(field) => field,
                    Err(SourceCheckError::UnknownName) => return Ok(None),
                    Err(error) => return Err(error),
                };
                let (SourceType::Aggregate { id } | SourceType::Sum { id }) = field.ty else {
                    return Ok(None);
                };
                (self.module.instances[id as usize].schema(), Some(&**base))
            }
            _ => return Ok(None),
        };
        self.child(callee.span_id, base.span_id)?;
        self.child(callee.span_id, name.span_id)?;
        self.name(name)?;
        let owner = self.module.layouts[schema].schema.name.clone();
        let mut found = self.lookup_method(&owner, &name.value)?;
        // A variant a union declared is `Union.Variant`: its methods are the union's.
        if found.is_none()
            && let Some((union, _)) = owner.rsplit_once('.')
        {
            found = self.lookup_method(union, &name.value)?;
        }
        Ok(found.map(|function| (function, receiver)))
    }

    /// The function implementing `left <operator> right` for a left operand of type `ty`:
    /// the `#infix` method whose first parameter has that type. A local one is ordered
    /// before its uses by the call graph; an imported one is already checked.
    pub(super) fn find_infix(
        &mut self,
        ty: SourceType,
        operator: InfixOperator,
    ) -> Result<Option<usize>> {
        self.charge(self.module.functions.len())?;
        Ok(self.module.functions.iter().position(|function| {
            function.as_ref().is_some_and(|function| {
                function.signature.infix == Some(operator)
                    && matches!(function.parameters.first(), Some(Parameter::Value(first)) if *first == ty)
            })
        }))
    }

    /// Whether `left <operator> ...` is evaluated here: native integer arithmetic, or an
    /// operator the nominal local on its left defines.
    pub(super) fn dispatches(
        &mut self,
        left: &'a Expression,
        operator: BinaryOperator,
    ) -> Result<bool> {
        let Some(infix) = InfixOperator::from_binary(operator) else {
            return Ok(false);
        };
        if infix.arithmetic().is_some() || infix.is_integer_operation() {
            return Ok(true);
        }
        // A nominal left operand always dispatches; with no such method that is a type
        // error reported by the dispatch, not a construct the checker does not know.
        let Expr::Name(name) = &left.value else {
            return Ok(false);
        };
        let Some(index) = self.local_position(name)? else {
            return Ok(false);
        };
        Ok(matches!(
            self.proof.names[index].1.ty,
            SourceType::Aggregate { .. }
        ))
    }

    /// How `left <operator> ...` takes its operands, when `left` is a plain local of a
    /// nominal type with an operator method: the method and its parameter passing. Only
    /// then are the operands known to be places that can be lent instead of moved.
    pub(super) fn operator_plan(
        &mut self,
        left: &'a Expression,
        operator: InfixOperator,
    ) -> Result<Option<(usize, Vec<Passing>)>> {
        let Expr::Name(name) = &left.value else {
            return Ok(None);
        };
        let Some(index) = self.local_position(name)? else {
            return Ok(None);
        };
        let ty = self.proof.names[index].1.ty;
        if !matches!(ty, SourceType::Aggregate { .. }) {
            return Ok(None);
        }
        let Some(function) = self.find_infix(ty, operator)? else {
            return Ok(None);
        };
        let interface = self.function_interface(function)?;
        Ok(Some((function, interface.passing.clone())))
    }

    /// Whether `operand` is a local that `passing` lends rather than moves.
    pub(super) fn is_lent(&mut self, operand: &Expression, passing: Passing) -> Result<bool> {
        Ok(passing.borrows()
            && matches!(&operand.value, Expr::Name(name) if self.local_position(name)?.is_some()))
    }

    /// Apply a non-generic function to arguments that are already evaluated: each is
    /// viewed as its parameter type (which is where integer-to-domain conversion happens),
    /// then the function's verified ownership outcome is applied.
    pub(super) fn apply_call(&mut self, function: usize, mut actual: Vec<Value>) -> Result<Value> {
        let interface = self.function_interface(function)?;
        if actual.len() != interface.parameters.len() {
            return Err(SourceCheckError::Type);
        }
        for (value, parameter) in actual.iter_mut().zip(&interface.parameters) {
            let Parameter::Value(template) = parameter else {
                return Err(SourceCheckError::Type);
            };
            *value = self.view_as(*value, *template)?;
        }
        self.constraints(&interface.constraints, &mut actual, false)?;
        let value = self.apply_outcomes(&interface.summaries, &actual, &[])?;
        self.view_as(value, interface.signature.result)
    }

    pub(super) fn call_tasks(
        &mut self,
        expression: &'a Expression,
        function: usize,
        receiver: Option<&'a Expression>,
        arguments: Option<&'a Spanned<Vec<Spanned<Argument>>>>,
        visited: bool,
        pending: &mut Vec<EvalTask<'a>>,
        values: &mut Vec<Value>,
    ) -> Result<()> {
        let interface = self.function_interface(function)?;
        let written = arguments.map_or(0, |arguments| arguments.value.len());
        let context = self.proof.call_expected.take();
        if let Some(omitted) = self.inference_plan(&interface, receiver, written) {
            return self.inferred_call_tasks(
                expression,
                interface,
                omitted,
                context,
                (receiver, arguments),
                visited,
                pending,
                values,
            );
        }
        if written + usize::from(receiver.is_some()) + interface.implicit_types
            != interface.parameters.len()
        {
            return Err(SourceCheckError::Type);
        }
        let Arguments { types, expressions } =
            self.call_arguments(&interface, receiver, arguments)?;
        self.check_type_bounds(&interface.signature.type_bounds, &types)?;
        let constraints = self.number_constraints(&interface.constraints, &types)?;
        let passing: Vec<Passing> = interface
            .parameters
            .iter()
            .zip(&interface.passing)
            .filter(|(parameter, _)| matches!(parameter, Parameter::Value(_)))
            .map(|(_, passing)| *passing)
            .collect();
        if visited {
            if values.len() < expressions.len() {
                return Err(SourceCheckError::Malformed);
            }
            let mut actual = values.split_off(values.len() - expressions.len());
            for ((_, template), value) in expressions.iter().zip(&mut actual) {
                let expected = self.substitute(*template, &types)?;
                *value = self.view_as(*value, expected)?;
            }
            self.constraints(&constraints, &mut actual, false)?;
            let known = self.known_bound(interface.bound_query, &types);
            let result = match known {
                Some(exact) => exact.ty,
                None => self.substitute(interface.signature.result, &types)?,
            };
            self.note_call(expression, &types, result);
            let value = match known {
                Some(exact) => exact,
                // A `#load` makes a value of its element type from memory. For a type parameter
                // no summary can invent one, so it is made here, under the same authority check
                // as rebuilding a value from its address (a private representation of another
                // module may not be forged). What the caller passes is its own type parameter.
                None if interface.signature.memory == Some(MemoryOp::Load) => {
                    self.load_access(result)?;
                    self.apply_outcomes(&interface.summaries, &actual, &types)?
                }
                None => self.apply_outcomes(&interface.summaries, &actual, &types)?,
            };
            // A member's transitions are worked out from the values before the call.
            let mut transitions = Vec::new();
            for (member, sets) in &interface.variant_effects {
                let member = self.substitute(*member, &types)?;
                for (path, expr) in sets {
                    let expected = self.clause_value(expr, &actual)?;
                    let expected = self.ensure_identity(expected)?;
                    transitions.push((member, path, expected));
                }
            }
            // What a `mut` borrow lent out may have changed: what the function says, else
            // nothing is known of it.
            self.apply_effects(
                interface.effects.as_deref(),
                &expressions,
                &passing,
                &actual,
            )?;
            let value = self.view_as(value, result)?;
            // The place a transition names holds a fresh value now; where the member is matched
            // it equals what the transition says.
            let mut transition_claims = Vec::new();
            for (member, path, expected) in transitions {
                let place = expressions.get(path.parameter as usize).ok_or(SourceCheckError::Malformed)?.0;
                let post = self.lent_value(place, &path.fields)?;
                let claim = Constraint {
                    left: 0,
                    left_field: None,
                    left_term: None,
                    modulus: None,
                    operator: BinaryOperator::Equal,
                    right: contracts::Term::Slot(1),
                };
                transition_claims.push((member, vec![claim], vec![post, expected]));
            }
            if !transition_claims.is_empty() {
                let seen = self.owner_view(value)?;
                if let Some(owner) = seen.owner {
                    self.proof.owners[owner].claims.extend(transition_claims);
                }
            }
            // The claims of each variant the result may be are kept with the result and assumed
            // where that variant is in play: in the arm that matches it (or a detached check of that
            // arm), not here, so an arm that cannot be taken is not checked under them.
            if !interface.variant_postconditions.is_empty() {
                let mut claims = Vec::new();
                for (member, post) in &interface.variant_postconditions {
                    claims.push((self.substitute(*member, &types)?, post.clone(), actual.clone()));
                }
                let seen = self.owner_view(value)?;
                match seen.owner {
                    Some(owner) => {
                        self.proof.owners[owner].claims.extend(claims);
                    }
                    None => {}
                }
            }
            // What the callee proved where it returned is a fact about this result.
            if !interface.postconditions.is_empty() {
                let mut slots = actual.clone();
                slots.push(value);
                match self.constraints(&interface.postconditions, &mut slots, true) {
                    Err(SourceCheckError::Budget) => return Err(SourceCheckError::Budget),
                    _ => {}
                }
            }
            values.push(self.attach_result_loans(&interface, &expressions, &actual, value)?);
        } else {
            self.charge(expressions.len() + 1)?;
            self.check_call_borrows(&expressions, &passing)?;
            pending.push(EvalTask::Expression(expression, None, true));
            for ((argument, template), passing) in expressions.into_iter().zip(passing).rev() {
                let expected = self.substitute(template, &types)?;
                // A borrowed local is lent, not moved; anything else is evaluated as a value.
                let place = passing.borrows() && self.is_place(argument)?;
                if place {
                    pending.push(EvalTask::Borrow(argument, passing == Passing::Mut));
                } else {
                    pending.push(EvalTask::Expression(argument, Some(expected), false));
                }
            }
        }
        Ok(())
    }

    /// Within one call a local may be lent shared to several parameters, but never lent
    /// exclusively while also lent or moved: those uses would overlap.
    pub(super) fn check_call_borrows(
        &mut self,
        expressions: &[(&'a Expression, SourceType)],
        passing: &[Passing],
    ) -> Result<()> {
        // Overlap needs a borrow and a second use; owners alone are caught by their moves.
        if expressions.len() < 2 || !passing.iter().any(|passing| passing.borrows()) {
            return Ok(());
        }
        self.charge_product(expressions.len(), expressions.len())?;
        for (index, (first, _)) in expressions.iter().enumerate() {
            let Some(path) = place_path(first) else {
                continue;
            };
            for (other, (second, _)) in expressions.iter().enumerate().skip(index + 1) {
                // Two paths overlap when one is the other or lies inside it.
                let Some(second_path) = place_path(second) else {
                    continue;
                };
                let overlap = path.starts_with(&second_path) || second_path.starts_with(&path);
                if !overlap {
                    continue;
                }
                let (left, right) = (passing[index], passing[other]);
                if left == Passing::Mut
                    || right == Passing::Mut
                    || left.borrows() != right.borrows()
                {
                    return Err(SourceCheckError::BorrowConflict);
                }
            }
        }
        Ok(())
    }

    fn call_arguments(
        &mut self,
        interface: &Function,
        receiver: Option<&'a Expression>,
        arguments: Option<&'a Spanned<Vec<Spanned<Argument>>>>,
    ) -> Result<Arguments<'a>> {
        let list: &[Spanned<Argument>] = arguments.map_or(&[], |arguments| &arguments.value);
        self.charge(self.proof.generics.len() + list.len() * 2)?;
        let scope: Vec<_> = self
            .proof
            .generics
            .iter()
            .enumerate()
            .map(|(index, name)| {
                (
                    *name,
                    SourceType::Generic {
                        index: index as u32,
                    },
                )
            })
            .collect();
        // A generic type's parameters are the receiver's own type arguments.
        let mut types = if let Some(forced) = self.proof.forced_types.take() {
            forced
        } else if interface.implicit_types > 0 {
            self.receiver_type_arguments(receiver, interface.implicit_types)?
        } else {
            Vec::new()
        };
        let mut expressions = Vec::new();
        // The receiver, when present, is the first supplied argument.
        let mut supplied: Vec<&'a Expression> = receiver.into_iter().collect();
        for argument in list {
            if let Some(arguments) = arguments {
                self.child(arguments.span_id, argument.span_id)?;
            }
            let Argument::Positional(expression) = &argument.value else {
                return Err(SourceCheckError::Type);
            };
            self.child(argument.span_id, expression.span_id)?;
            supplied.push(expression);
        }
        for (parameter, expression) in interface
            .parameters
            .iter()
            .skip(interface.implicit_types)
            .zip(supplied)
        {
            match parameter {
                Parameter::Type => types.push(self.resolve_type(expression, &scope)?),
                Parameter::Value(ty) => expressions.push((expression, *ty)),
                // Compile-time text, decided where the call is recorded and its locals are
                // in scope; the checker has nothing of it to run.
                Parameter::Literal => {}
            }
        }
        Ok(Arguments { types, expressions })
    }

    /// The type arguments of the receiver of a call, read from its static type.
    pub(super) fn receiver_type_arguments(
        &mut self,
        receiver: Option<&'a Expression>,
        count: usize,
    ) -> Result<Vec<SourceType>> {
        let receiver = receiver.ok_or(SourceCheckError::Type)?;
        let ty = match &receiver.value {
            Expr::Name(name) => {
                let position = self
                    .local_position(name)?
                    .ok_or(SourceCheckError::UnknownName)?;
                self.proof.names[position].1.ty
            }
            _ => self.project_place(receiver)?.ty,
        };
        let (SourceType::Aggregate { id } | SourceType::Sum { id }) = ty else {
            return Err(SourceCheckError::Type);
        };
        let arguments = self.module.instances[id as usize].arguments.clone();
        if arguments.len() != count {
            return Err(SourceCheckError::Type);
        }
        self.charge(count)?;
        Ok(arguments)
    }

    pub(super) fn substitute(
        &mut self,
        root: SourceType,
        types: &[SourceType],
    ) -> Result<SourceType> {
        self.charge(1)?;
        match root {
            SourceType::Generic { index } => {
                return types
                    .get(index as usize)
                    .copied()
                    .ok_or(SourceCheckError::Type);
            }
            SourceType::Integer { .. } | SourceType::Unit | SourceType::Text => return Ok(root),
            _ => {}
        }
        let id = match root {
            SourceType::Aggregate { id }
            | SourceType::Sum { id }
            | SourceType::Array { id }
            | SourceType::Sequence { id }
            | SourceType::Pointer { id } => id,
            _ => unreachable!("scalar substitution returned above"),
        };
        self.charge(1)?;
        if self.module.instances[id as usize].arguments.is_empty() {
            return Ok(root);
        }
        self.charge_product(
            (usize::BITS - self.module.substitutions.len().leading_zeros()) as usize + 1,
            types.len() + 1,
        )?;
        let cached = self
            .module
            .substitutions
            .binary_search_by(|((ty, arguments), _)| {
                ty.cmp(&root).then_with(|| arguments.as_slice().cmp(types))
            });
        if let Ok(index) = cached {
            return Ok(self.module.substitutions[index].1);
        }
        let mut pending = vec![(root, false)];
        let mut values = Vec::new();
        while let Some((ty, visited)) = pending.pop() {
            self.charge(1)?;
            match ty {
                SourceType::Generic { index } => {
                    values.push(*types.get(index as usize).ok_or(SourceCheckError::Type)?)
                }
                SourceType::Aggregate { id }
                | SourceType::Sum { id }
                | SourceType::Array { id }
                | SourceType::Sequence { id }
                | SourceType::Pointer { id } => {
                    let head = self.module.instances[id as usize].head;
                    let count = self.module.instances[id as usize].arguments.len();
                    self.charge(count + 1)?;
                    if visited {
                        if values.len() < count {
                            return Err(SourceCheckError::Malformed);
                        }
                        let arguments = values.split_off(values.len() - count);
                        values.push(self.intern_instance(head, arguments)?);
                    } else {
                        pending.push((ty, true));
                        for argument in self.module.instances[id as usize].arguments.iter().rev() {
                            pending.push((*argument, false));
                        }
                    }
                }
                _ => values.push(ty),
            }
        }
        if values.len() != 1 {
            return Err(SourceCheckError::Malformed);
        }
        self.charge(self.module.substitutions.len() + types.len() + 1)?;
        self.module
            .substitutions
            .insert(cached.unwrap_err(), ((root, types.to_vec()), values[0]));
        Ok(values[0])
    }

    pub(super) fn function_interface(&mut self, index: usize) -> Result<Rc<Function>> {
        // Calls borrow an immutable verified capsule; only module import/export copies it.
        self.charge(1)?;
        self.module.functions[index]
            .as_ref()
            .cloned()
            .ok_or(SourceCheckError::Unsupported)
    }

    pub(super) fn call_graph(
        &mut self,
        bindings: &[FunctionDeclaration<'a>],
    ) -> DiagnosticResult<Vec<Vec<usize>>> {
        self.charge(bindings.len())?;
        let mut graph = Vec::new();
        self.module.weak_edges.clear();
        for (index, declaration) in bindings.iter().enumerate() {
            let (strong, weak) = self
                .function_edges(declaration.binding, index)
                .map_err(|error| self.failure(error, declaration.span))?;
            graph.push(strong);
            self.module.weak_edges.push(weak);
        }
        Ok(graph)
    }

    /// Calls by name are strong edges: the callee must be checked first. Method and operator
    /// uses depend on a receiver type unknown here, so they are weak: advisory ordering
    /// that never blocks, since a false cycle between providers must not deadlock.
    fn function_edges(
        &mut self,
        binding: &'a Binding,
        own: usize,
    ) -> Result<(Vec<usize>, Vec<usize>)> {
        // A trait's bodiless method calls nothing: it is only a signature.
        if self.instruction_of(binding).is_some() || self.memory_of(binding).is_some() || binding.value.is_none() {
            return Ok((Vec::new(), Vec::new()));
        }
        let body = binding
            .value
            .as_ref()
            .ok_or(SourceCheckError::MissingBody)?;
        self.charge(1)?;
        let parameter_count = binding.parameters.as_ref().map_or(0, |p| p.value.len());
        if parameter_count > self.parameters_max as usize {
            return Err(SourceCheckError::Budget);
        }
        self.charge(parameter_count)?;
        let scope: Vec<_> = binding
            .parameters
            .as_ref()
            .map(|p| p.value.iter().map(|p| p.name.value.as_str()).collect())
            .unwrap_or_default();
        // Names whose type is written: parameters, and the function's top-level locals given a
        // type or a constructor. `x.m` on one of them is `T.m`, known exactly.
        let typed = self.typed_names(binding, &body.expression);
        let mut pending = vec![(&body.expression, scope)];
        let mut edges = Vec::new();
        let mut weak = Vec::new();
        while let Some((expression, scope)) = pending.pop() {
            self.charge(scope.len() + 1)?;
            match &expression.value {
                Expr::Loop(loop_expression) => {
                    if let LoopExpression::For {
                        binder,
                        iterable,
                        body,
                        ..
                    } = loop_expression.as_ref()
                    {
                        self.charge(scope.len() * 2 + 3)?;
                        pending.push((iterable, scope.clone()));
                        let mut body_scope = scope;
                        body_scope.push(&binder.value);
                        pending.push((body, body_scope));
                    } else if let LoopExpression::While {
                        condition, body, ..
                    } = loop_expression.as_ref()
                    {
                        // A `while` calls what its condition and body call (G-E: a bound call
                        // in the body was checked before the trait method it reaches).
                        self.charge(scope.len() * 2 + 2)?;
                        pending.push((condition, scope.clone()));
                        pending.push((body, scope));
                    }
                }
                Expr::Sequence(sequence) => {
                    self.charge_product(sequence.elements.len(), scope.len() + 1)?;
                    for element in &sequence.elements {
                        pending.push((element, scope.clone()));
                    }
                }
                Expr::Array(array) => {
                    self.charge_product(array.elements.len(), scope.len() + 1)?;
                    for element in &array.elements {
                        pending.push((element, scope.clone()));
                    }
                }
                Expr::Select(select) => {
                    self.charge(scope.len() + 2)?;
                    pending.push((&select.base, scope.clone()));
                    pending.push((&select.index, scope));
                }
                Expr::NumericSelect(select) => {
                    self.charge(1)?;
                    pending.push((&select.base, scope));
                }
                Expr::Name(name) => {
                    if !scope.contains(&name.as_str())
                        && let Some(index) = self.lookup_symbol(name, true)?
                    {
                        edges.push(index);
                    }
                }
                Expr::Call { callee, arguments } => {
                    self.charge_product(arguments.value.len() + 1, scope.len() + 1)?;
                    pending.push((callee, scope.clone()));
                    for argument in &arguments.value {
                        let value = match &argument.value {
                            Argument::Positional(value) | Argument::Named { value, .. } => value,
                            Argument::Typed { annotation, .. } => annotation,
                        };
                        pending.push((value, scope.clone()));
                    }
                }
                // `Type.name`: a function of a named type, known exactly.
                Expr::Member { base, name }
                    if matches!(&base.value, Expr::Name(head) if !scope.contains(&head.as_str()))
                        && {
                            let Expr::Name(head) = &base.value else { unreachable!() };
                            let qualified = format!("{head}.{}", name.value);
                            self.module.function_names.iter().any(|(symbol, index)| *symbol == qualified && *index != own)
                        } =>
                {
                    let Expr::Name(head) = &base.value else { unreachable!() };
                    let qualified = format!("{head}.{}", name.value);
                    self.charge(self.module.function_names.len() + 1)?;
                    for (symbol, index) in &self.module.function_names {
                        if *symbol == qualified && *index != own {
                            edges.push(*index);
                        }
                    }
                }
                Expr::Member { base, name }
                    if matches!(&base.value, Expr::Name(local) if typed.iter().any(|(n, _)| n == local)) =>
                {
                    let Expr::Name(local) = &base.value else { unreachable!() };
                    let head = typed.iter().rev().find(|(n, _)| n == local).map(|(_, t)| *t).unwrap_or_default();
                    let qualified = format!("{head}.{}", name.value);
                    self.charge(self.module.function_names.len() + 1)?;
                    let mut found = false;
                    for (symbol, index) in &self.module.function_names {
                        if *symbol == qualified && *index != own {
                            edges.push(*index);
                            found = true;
                        }
                    }
                    // A known type without this method here: an imported one (already checked)
                    // or a trait's; either way nothing local to wait for, apart from a provider.
                    if !found && !self.module.type_names.iter().any(|(t, _)| t == head) {
                        for (symbol, index) in &self.module.function_names {
                            if symbol.rsplit_once('.').is_some_and(|(_, method)| method == name.value)
                                && *index != own
                            {
                                weak.push(*index);
                            }
                        }
                    }
                    pending.push((base, scope));
                }
                Expr::Member { base, name } => {
                    // `x.name` may call a method, with or without parentheses. The
                    // receiver's type is unknown here: depend on every method of that
                    // name. A false self edge would deadlock, so it is dropped; genuine
                    // recursion still fails closed when the body is checked.
                    self.charge(self.module.function_names.len() + 1)?;
                    for (symbol, index) in &self.module.function_names {
                        if symbol
                            .rsplit_once('.')
                            .is_some_and(|(_, method)| method == name.value)
                            && *index != own
                        {
                            weak.push(*index);
                        }
                    }
                    pending.push((base, scope));
                }
                Expr::Group(child)
                | Expr::Unary { operand: child, .. }
                | Expr::Cast { value: child, .. } => {
                    self.charge(1)?;
                    pending.push((child, scope));
                }
                Expr::Block(block) => {
                    self.graph_block(block, scope, &mut pending, own, &mut weak)?
                }
                // A subject can call a method, and the arms' values see the binders of their
                // conditions: the names of an `is` pattern are added to the scope of the value.
                Expr::IsTest(test) => {
                    self.charge(1)?;
                    pending.push((&test.subject, scope));
                }
                Expr::Chain(chain) => {
                    self.charge_product(chain.arms.len() + 1, scope.len() + 1)?;
                    for arm in &chain.arms {
                        let mut arm_scope = scope.clone();
                        collect_binders(&arm.condition, &mut arm_scope);
                        pending.push((&arm.condition, scope.clone()));
                        pending.push((&arm.value.value, arm_scope));
                    }
                    pending.push((&chain.otherwise, scope));
                }
                Expr::Conditional(case) => {
                    self.charge(3)?;
                    pending.push((&case.condition, scope.clone()));
                    pending.push((&case.then_branch, scope.clone()));
                    pending.push((&case.else_branch, scope));
                }
                Expr::Match(case) => {
                    pending.push((&case.subject, scope.clone()));
                    for arm in &case.arms.value {
                        self.charge(scope.len() + 1)?;
                        let mut arm_scope = scope.clone();
                        if let MatchPattern::Variant(pattern) = &arm.pattern.value
                            && let Some(fields) = &pattern.fields
                        {
                            self.charge(fields.binders.value.len())?;
                            arm_scope.extend(
                                fields
                                    .binders
                                    .value
                                    .iter()
                                    .map(|binder| binder.value.as_str()),
                            );
                        }
                        if let Some(guard) = &arm.guard {
                            collect_binders(&guard.condition, &mut arm_scope);
                            pending.push((&guard.condition, arm_scope.clone()));
                        }
                        pending.push((&arm.body, arm_scope));
                    }
                }
                Expr::Binary {
                    left,
                    operator,
                    right,
                } => {
                    self.charge(2)?;
                    if let Some(operator) = InfixOperator::from_binary(operator.value) {
                        // The left operand's type is unknown here: depend on every local
                        // method implementing this operator, apart from the caller itself.
                        self.charge(self.module.infix_decls.len())?;
                        weak.extend(
                            self.module
                                .infix_decls
                                .iter()
                                .filter(|(found, function)| *found == operator && *function != own)
                                .map(|(_, function)| *function),
                        );
                    }
                    pending.push((left, scope.clone()));
                    pending.push((right, scope));
                }
                _ => {}
            }
        }
        Ok((edges, weak))
    }

    fn graph_block(
        &mut self,
        block: &'a Block,
        mut scope: Vec<&'a str>,
        pending: &mut Vec<(&'a Expression, Vec<&'a str>)>,
        own: usize,
        weak: &mut Vec<usize>,
    ) -> Result<()> {
        self.charge(block.statements.len())?;
        for statement in &block.statements {
            match &statement.value {
                Statement::Binding(binding) => {
                    if let Some(body) = &binding.value {
                        self.charge(scope.len())?;
                        pending.push((&body.expression, scope.clone()));
                    }
                    scope.push(&binding.name.value);
                }
                Statement::Destructure(pattern) => {
                    self.charge(scope.len())?;
                    pending.push((&pattern.value, scope.clone()));
                    if let Some(fields) = &pattern.pattern.fields {
                        self.charge(fields.binders.value.len())?;
                        scope.extend(
                            fields
                                .binders
                                .value
                                .iter()
                                .map(|binder| binder.value.as_str()),
                        );
                    }
                }
                Statement::Return(ret) => {
                    self.charge(1)?;
                    if let Some(value) = &ret.value {
                        self.charge(scope.len())?;
                        pending.push((value, scope.clone()));
                    }
                }
                Statement::Expression(value) => {
                    self.charge(scope.len())?;
                    pending.push((value, scope.clone()));
                }
                Statement::Assign(assign) => {
                    self.charge(scope.len())?;
                    // `place +: value` uses the place type's operator like `place + value`.
                    if let Some(operator) = assign
                        .compound
                        .as_ref()
                        .and_then(|compound| InfixOperator::from_binary(compound.value))
                    {
                        self.charge(self.module.infix_decls.len())?;
                        weak.extend(
                            self.module
                                .infix_decls
                                .iter()
                                .filter(|(found, function)| *found == operator && *function != own)
                                .map(|(_, function)| *function),
                        );
                    }
                    pending.push((&assign.value, scope.clone()));
                }
                Statement::WhenIs(when) => {
                    self.charge(scope.len())?;
                    pending.push((&when.subject, scope.clone()));
                    let otherwise = when.else_branch.iter().map(|branch| &branch.block);
                    for block in std::iter::once(&when.then_block).chain(otherwise) {
                        // The pattern's binders are in scope in its block.
                        let mut inner = scope.clone();
                        if let MatchPattern::Variant(pattern) = &when.pattern.value
                            && let Some(fields) = &pattern.fields
                        {
                            inner.extend(fields.binders.value.iter().map(|binder| binder.value.as_str()));
                        }
                        self.graph_block(&block.value, inner, pending, own, weak)?;
                    }
                }
                Statement::When(when) => {
                    self.charge(scope.len())?;
                    pending.push((&when.condition, scope.clone()));
                    let otherwise = when.else_branch.iter().map(|branch| &branch.block);
                    for block in std::iter::once(&when.then_block).chain(otherwise) {
                        self.graph_block(&block.value, scope.clone(), pending, own, weak)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(super) fn check_bodies(
        &mut self,
        bindings: &[FunctionDeclaration<'a>],
        graph: &[Vec<usize>],
        parameters_max: u32,
    ) -> DiagnosticResult<()> {
        let mut remaining = bindings.len();
        // Weak edges order bodies when they can, and are dropped once nothing else can run.
        let mut relaxed = false;
        for _ in 0..=bindings.len() * 2 {
            let mut progress = false;
            self.charge(bindings.len())?;
            for (index, binding) in bindings.iter().enumerate() {
                if self.module.functions[index].is_some() {
                    continue;
                }
                self.charge(graph[index].len() + self.module.weak_edges[index].len())?;
                if graph[index]
                    .iter()
                    .any(|edge| self.module.functions[*edge].is_none())
                {
                    continue;
                }
                if !relaxed
                    && self.module.weak_edges[index]
                        .iter()
                        .any(|edge| self.module.functions[*edge].is_none())
                {
                    continue;
                }
                self.proof.infix = binding.infix;
                // A constant that is not an integer is a function of nothing, its type inferred.
                self.proof.infer_result =
                    binding.provided || constants::is_constant(binding.binding);
                self.proof.trait_result = binding.trait_result;
                self.proof.abstract_trait = binding.abstract_trait;
                self.proof.receiver = match binding.receiver {
                    _ if binding.abstract_trait.is_some() => Some(SourceType::Generic { index: 0 }),
                    Some(schema) => {
                        // A generic type's receiver ranges over its own parameters.
                        let count = self.module.layouts[schema].schema.parameters.len();
                        let arguments = (0..count)
                            .map(|index| SourceType::Generic {
                                index: index as u32,
                            })
                            .collect();
                        Some(
                            self.instance(schema, arguments)
                                .map_err(|error| self.failure(error, binding.span))?,
                        )
                    }
                    None => None,
                };
                self.proof.receiver_schema = binding.receiver;
                self.module.current_function = match (binding.abstract_trait, binding.receiver) {
                    (Some(declaration), _) => format!("{}.{}", declaration.name.value, binding.binding.name.value),
                    (None, Some(schema)) => format!("{}.{}", self.module.schemas[schema].name.value, binding.binding.name.value),
                    (None, None) => binding.binding.name.value.clone(),
                };
                let function = self
                    .function_worlds(binding.binding, parameters_max)
                    .map_err(|error| self.failure(error, binding.span))?;
                self.module.functions[index] = Some(Rc::new(function));
                remaining -= 1;
                progress = true;
            }
            if remaining == 0 {
                return Ok(());
            }
            if !progress {
                if relaxed {
                    return Err(SourceCheckError::Unsupported.into());
                }
                relaxed = true;
            } else {
                // Progress may unblock weak edges again, so try them before relaxing.
                relaxed = false;
            }
        }
        if remaining == 0 {
            Ok(())
        } else {
            Err(SourceCheckError::Unsupported.into())
        }
    }
}

/// The names the `is` tests of a condition may bind, however it is wrapped.
fn collect_binders<'a>(condition: &'a Expression, scope: &mut Vec<&'a str>) {
    let mut stack = vec![condition];
    while let Some(next) = stack.pop() {
        match &next.value {
            Expr::Group(inner) => stack.push(inner),
            Expr::Unary { operand, .. } => stack.push(operand),
            Expr::Binary { left, right, .. } => stack.extend([left.as_ref(), right.as_ref()]),
            Expr::IsTest(test) => {
                if let MatchPattern::Variant(pattern) = &test.pattern.value
                    && let Some(fields) = &pattern.fields
                {
                    scope.extend(fields.binders.value.iter().map(|binder| binder.value.as_str()));
                }
            }
            _ => {}
        }
    }
}

impl<'a> Checker<'a> {
    /// `(name, type head)` for a function's parameters with a written nominal type, and for its
    /// top-level locals written `name Type: ...` or `name: Type(...)` / `name: Type.f(...)`.
    fn typed_names(&self, binding: &'a Binding, body: &'a Expression) -> Vec<(&'a str, &'a str)> {
        fn head(expression: &Expression) -> Option<&str> {
            match &expression.value {
                Expr::Name(name) => Some(name.as_str()),
                Expr::Call { callee, .. } => match &callee.value {
                    Expr::Name(name) => Some(name.as_str()),
                    Expr::Member { base, .. } => match &base.value {
                        Expr::Name(name) if name.chars().next().is_some_and(char::is_uppercase) => Some(name.as_str()),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            }
        }
        let annotated = |annotation: &'a Option<Spanned<Annotation>>| -> Option<&'a str> {
            match &annotation.as_ref()?.value {
                Annotation::Expression(expression) => head(expression),
                Annotation::Refined { base: Some(expression), .. } => head(expression),
                _ => None,
            }
        };
        let mut typed = Vec::new();
        for parameter in binding.parameters.iter().flat_map(|p| &p.value) {
            if let Some(ty) = annotated(&parameter.value.annotation)
                && ty.chars().next().is_some_and(char::is_uppercase)
            {
                typed.push((parameter.value.name.value.as_str(), ty));
            }
        }
        if let Expr::Block(block) = &body.value {
            for statement in &block.statements {
                if let Statement::Binding(local) = &statement.value {
                    let ty = annotated(&local.annotation)
                        .or_else(|| local.value.as_ref().and_then(|value| head(&value.expression)));
                    if let Some(ty) = ty
                        && ty.chars().next().is_some_and(char::is_uppercase)
                    {
                        typed.push((local.name.value.as_str(), ty));
                    }
                }
            }
        }
        typed
    }
}

impl<'a> Checker<'a> {
    /// Whether `actual` is one of the members of the sum `member` (a world of `or AllocError`).
    /// Assume the claims kept with `value`'s owner for the variant `value` is, as facts about its
    /// payload and about the call's arguments. Consumed: assumed once per world.
    pub(super) fn apply_claims(&mut self, value: Value) -> Result<()> {
        let Some(owner) = value.owner else {
            return Ok(());
        };
        // No claims, no work: a program without them costs what it cost before.
        if self.proof.owners[owner].claims.is_empty() {
            return Ok(());
        }
        let view = self.owner_view(value)?;
        let member = view.ty;
        let mut index = 0;
        while index < self.proof.owners[owner].claims.len() {
            if self.proof.owners[owner].claims[index].0 != member {
                index += 1;
                continue;
            }
            let (_, post, actual) = self.proof.owners[owner].claims.remove(index);
            let mut slots = actual;
            slots.push(view);
            // (Read from the layout, which an importer has: the names in the source are the
            // defining module's.)
            let named: Vec<usize> = match member {
                SourceType::Aggregate { id } => self.module.layouts[self.module.instances[id as usize].schema()]
                    .schema
                    .fields
                    .iter()
                    .enumerate()
                    .filter(|(_, field)| !field.name.is_empty())
                    .map(|(index, _)| index)
                    .collect(),
                _ => Vec::new(),
            };
            if !named.is_empty() {
                self.materialize_children(view)?;
                for field in named {
                    slots.push(self.proof.owners[owner].children[field]);
                }
            }
            if let Err(SourceCheckError::Budget) = self.constraints(&post, &mut slots, true) {
                return Err(SourceCheckError::Budget);
            }
        }
        Ok(())
    }

    /// Compile the `where` claims of each variant of `binding`'s result into constraints over
    /// the parameters and the result, which follows them. A claim the caller cannot read is left
    /// out (every claim here is proved where the function returns, or is a trait's signature).
    /// The named payload fields of `member`, a variant: name and type, in field order. A variant
    /// whose one payload is unnamed has none, and no claim can refer to it.
    pub(super) fn named_payload_fields(&mut self, member: SourceType) -> Result<Vec<(&'a str, SourceType)>> {
        let SourceType::Aggregate { id } = member else {
            return Ok(Vec::new());
        };
        let schema = self.module.instances[id as usize].schema();
        if !self.is_declared_variant(schema) {
            return Ok(Vec::new());
        }
        let Some((_, written)) = self.module.variant_field_names.iter().find(|(layout, _)| *layout == schema) else {
            return Ok(Vec::new());
        };
        let written = written.clone();
        let types = self.instance_fields(id)?;
        let mut named = Vec::new();
        for (name, ty) in written.into_iter().zip(types) {
            if !name.is_empty() {
                named.push((name, ty));
            }
        }
        Ok(named)
    }

    pub(super) fn compile_variant_claims(&mut self, binding: &'a Binding) -> Result<()> {
        let contracts: Vec<Option<&'a Spanned<Contract>>> = std::iter::once(binding.contract.as_ref())
            .chain(binding.alternatives.iter().map(|alt| alt.value.contract.as_ref()))
            .collect();
        let members = self.proof.variants.clone();
        if members.len() != contracts.len() {
            return Ok(());
        }
        // The `::` transitions of each member, which a caller learns where it matches it.
        self.proof.variant_effects.clear();
        let parameter_count = self.proof.names.len();
        for (member, contract) in members.iter().zip(&contracts) {
            let sets = self.variant_transitions(*contract, parameter_count)?;
            if !sets.is_empty() {
                self.proof.variant_effects.push((*member, sets));
            }
        }
        for (member, contract) in members.into_iter().zip(contracts) {
            let Some(contract) = contract else { continue };
            let result = self.input_value(member)?;
            let mark = self.proof.names.len();
            // The slot after the parameters is the result (named by the binder when there is one);
            // the named payload fields of the variant follow, in order.
            self.proof.names.push((binding.result_binder.as_ref().map_or("", |b| b.value.as_str()), result));
            let fields = self.named_payload_fields(member)?;
            let mut field_values = Vec::new();
            for (name, ty) in fields {
                if self.proof.names[..mark].iter().any(|(other, _)| *other == name) {
                    let detail = format!("the payload field `{name}` has the name of a parameter; rename one");
                    return Err(self.explain(SourceCheckError::Duplicate, detail));
                }
                let value = self.input_value(ty)?;
                self.proof.names.push((name, value));
                field_values.push(value);
            }
            let names: Vec<&'a str> = self.proof.names.iter().map(|(name, _)| *name).collect();
            let mut post = Vec::new();
            for item in &contract.value.items {
                if let ClauseItem::Claim(RefinementPredicate::Expression(relation)) = &item.value {
                    // `(a and b)` is two claims.
                    let mut parts = vec![relation];
                    while let Some(part) = parts.pop() {
                        match &part.value {
                            Expr::Group(inner) => parts.push(inner),
                            Expr::Binary { left, operator, right }
                                if operator.value == BinaryOperator::And =>
                            {
                                parts.push(right);
                                parts.push(left);
                            }
                            _ => {
                                let before = post.len();
                                match self.relation_constraint(part, &names, &mut post) {
                                    Err(SourceCheckError::Budget) => return Err(SourceCheckError::Budget),
                                    Ok(()) => {}
                                    Err(_) => post.truncate(before),
                                }
                            }
                        }
                    }
                }
            }
            self.proof.names.truncate(mark);
            for value in field_values {
                if let Some(owner) = value.owner {
                    self.proof.owners[owner].live = false;
                }
            }
            if let Some(owner) = result.owner {
                self.proof.owners[owner].live = false;
            }
            if !post.is_empty() {
                self.proof.variant_postconditions.push((member, post));
            }
        }
        Ok(())
    }
}
