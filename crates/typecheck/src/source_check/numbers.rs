//! Compile-time number type parameters: `ArrayView(element, capacity Length)`.
//!
//! A parameter annotated by an integer domain (`capacity Length`) or a written range
//! (`n in 0 to 9`) takes a number known at check time instead of a type. Representation:
//!
//! * **Applied** (`ArrayView(Byte, 16)`): the argument is the singleton integer type
//!   `Integer { 16, 16 }`, so two instances with different numbers are different types by the
//!   ordinary instance arena. The argument is range-checked against the annotation.
//! * **Abstract** (inside a generic type's declaration or its methods): the argument is
//!   `Generic { index }` like any type parameter. `proof.numbers` remembers which generics
//!   are numbers and their range; in a method it also holds one scalar `Value` (a fresh
//!   identity in the parameter's range), which is what the name `capacity` evaluates to. So
//!   `len <= capacity` is an ordinary fact between two identities, and the existing extent
//!   machinery proves `n <= capacity` for a slice of an array of `capacity` elements.
//! * **Arrays** `[element; capacity]` are the `Sized` head with arguments
//!   `[element, number]`. Applying the number rewrites it to `Array { extent }` at interning,
//!   so after substitution it is the ordinary constant-extent array. While abstract, its values
//!   get `Owner::extent` = the identity of the `capacity` scalar (`bind_sized_extent`).
//! * **Claims** may mention a number parameter: `Term::Number`, resolved to the constant when
//!   a call supplies the instance, or to the scalar inside the method.
use super::contracts::{Constraint, Term};
use super::*;

/// A number parameter of the schema or method being checked.
#[derive(Clone, Copy)]
pub(super) struct NumberParam {
    /// The parameter's generic index.
    pub(super) index: u32,
    pub(super) lower: I256,
    pub(super) upper: I256,
    /// In a method: the scalar the parameter's name evaluates to.
    pub(super) value: Option<Value>,
}

impl<'a> Checker<'a> {
    /// Whether a parameter's annotation is written like a number's type: a domain name or a
    /// written range. Shape only; `number_ranges` decides what it names.
    pub(super) fn number_shaped(annotation: &Annotation) -> bool {
        match annotation {
            Annotation::Expression(expression) => {
                matches!(&expression.value, Expr::Name(name) if name != "Type")
            }
            Annotation::Domain { .. } => true,
            _ => false,
        }
    }

    /// The range of each parameter of `schema` that is a number, `None` for a type parameter.
    /// Empty when no parameter is a number.
    pub(super) fn number_ranges(&mut self, schema: usize) -> Result<Vec<Option<(I256, I256)>>> {
        if schema >= self.module.schemas.len() {
            return Ok(self.module.layouts[schema].schema.numbers.clone());
        }
        let declaration = self.module.schemas[schema];
        let Some(parameters) = declaration.parameters.as_ref() else {
            return Ok(Vec::new());
        };
        // Uncharged shape look: most generic types have no number parameter at all.
        if !parameters.value.iter().any(|parameter| {
            parameter
                .value
                .annotation
                .as_ref()
                .is_some_and(|annotation| Self::number_shaped(&annotation.value))
        }) {
            return Ok(Vec::new());
        }
        self.charge(parameters.value.len())?;
        let mut output = Vec::new();
        for parameter in &parameters.value {
            output.push(self.parameter_range(parameter)?);
        }
        if output.iter().all(Option::is_none) {
            output.clear();
        }
        Ok(output)
    }

    fn parameter_range(&mut self, parameter: &'a Spanned<Parameter>) -> Result<Option<(I256, I256)>> {
        let Some(annotation) = parameter.value.annotation.as_ref() else {
            return Ok(None);
        };
        self.annotation_range(annotation)
    }

    pub(super) fn annotation_range(
        &mut self,
        annotation: &'a Spanned<Annotation>,
    ) -> Result<Option<(I256, I256)>> {
        match &annotation.value {
            Annotation::Bound { .. } => Ok(None),
            Annotation::Domain { domain, .. } => Ok(Some(self.domain_bounds(domain)?)),
            Annotation::Expression(expression) => {
                let Expr::Name(name) = &expression.value else {
                    return Err(SourceCheckError::Unsupported);
                };
                self.token(expression.span_id, name)?;
                let schema = self
                    .lookup_symbol(name, false)?
                    .ok_or(SourceCheckError::Unsupported)?;
                if schema < self.module.schemas.len() {
                    if let TypeBody::Domain(written) = &self.module.schemas[schema].body.value {
                        return Ok(Some(self.domain_bounds(written)?));
                    }
                    return Err(SourceCheckError::Unsupported);
                }
                let layout = &self.module.layouts[schema].schema;
                match layout.fields.first().map(|field| field.ty) {
                    Some(SourceType::Integer { lower, upper })
                        if layout.kind == AggregateKind::Domain =>
                    {
                        Ok(Some((lower, upper)))
                    }
                    _ => Err(SourceCheckError::Unsupported),
                }
            }
            _ => Err(SourceCheckError::Unsupported),
        }
    }

    /// Methods of a type with number parameters: declare the numbers among the generics
    /// `first..`, each with a scalar the parameter's name evaluates to.
    pub(super) fn declare_numbers(&mut self, schema: usize, first: usize) -> Result<()> {
        let ranges = self.number_ranges(schema)?;
        self.charge(ranges.len())?;
        for (position, range) in ranges.into_iter().enumerate() {
            let Some((lower, upper)) = range else {
                continue;
            };
            let value = self.fresh_value(SourceType::Integer { lower, upper })?;
            self.proof.numbers.push(NumberParam {
                index: (first + position) as u32,
                lower,
                upper,
                value: Some(value),
            });
        }
        Ok(())
    }

    /// Inside a schema's declaration: the range of each number parameter, now computable.
    pub(super) fn fill_numbers(&mut self, schema: usize) -> Result<()> {
        let ranges = self.number_ranges(schema)?;
        for (position, range) in ranges.into_iter().enumerate() {
            if let Some((lower, upper)) = range
                && let Some(number) = self
                    .proof
                    .numbers
                    .iter_mut()
                    .find(|number| number.index as usize == position)
            {
                number.lower = lower;
                number.upper = upper;
            }
        }
        Ok(())
    }

    /// The scalar a number parameter's name evaluates to inside a method.
    pub(super) fn number_value(&self, name: &str) -> Option<Value> {
        if self.proof.numbers.is_empty() {
            return None;
        }
        let index = self.proof.generics.iter().position(|other| *other == name)? as u32;
        self.proof
            .numbers
            .iter()
            .find(|number| number.index == index)
            .and_then(|number| number.value)
    }

    fn is_number_generic(&self, ty: SourceType) -> Option<NumberParam> {
        let SourceType::Generic { index } = ty else {
            return None;
        };
        self.proof
            .numbers
            .iter()
            .find(|number| number.index == index)
            .copied()
    }

    /// `[element; capacity]` with `capacity` a number parameter in `scope`: that parameter.
    pub(super) fn symbolic_extent(
        &mut self,
        expression: &'a Expression,
        array: &'a ArrayExpression,
        scope: &[(&'a str, SourceType)],
    ) -> Result<Option<SourceType>> {
        if self.proof.numbers.is_empty() {
            return Ok(None);
        }
        let Some(length) = &array.length else {
            return Ok(None);
        };
        let Expr::Name(name) = &length.value.value else {
            return Ok(None);
        };
        let Some((_, ty)) = scope.iter().find(|(other, _)| other == name) else {
            return Ok(None);
        };
        if self.is_number_generic(*ty).is_none() {
            return Ok(None);
        }
        self.array_syntax(expression, array)?;
        self.token(length.value.span_id, name)?;
        if array.elements.len() != 1 {
            return Err(SourceCheckError::Type);
        }
        Ok(Some(*ty))
    }

    pub(super) fn sized_array(&mut self, element: SourceType, number: SourceType) -> Result<SourceType> {
        self.intern_instance(CheckedSourceInstanceHead::Sized, vec![element, number])
    }

    /// Applying a number to `Sized [element, number]` gives the constant-extent array.
    pub(super) fn apply_sized(&mut self, arguments: &[SourceType]) -> Result<Option<SourceType>> {
        match arguments[1] {
            SourceType::Integer { lower, upper } if lower == upper => {
                if lower < I256::ZERO || lower > I256::from(u32::MAX) {
                    return Err(SourceCheckError::Type);
                }
                Ok(Some(self.array_type(arguments[0], lower.as_u32())?))
            }
            SourceType::Generic { .. } => Ok(None),
            _ => Err(SourceCheckError::Type),
        }
    }

    /// Two array types of unknown extent are the same only when built the same way.
    pub(super) fn sized_mismatch(&self, left: SourceType, right: SourceType) -> bool {
        let (SourceType::Array { id: a }, SourceType::Array { id: b }) = (left, right) else {
            return false;
        };
        let (a, b) = (&self.module.instances[a as usize], &self.module.instances[b as usize]);
        let dynamic = |head| {
            matches!(
                head,
                CheckedSourceInstanceHead::Sized | CheckedSourceInstanceHead::Slice
            )
        };
        (dynamic(a.head) || dynamic(b.head)) && a.head != b.head
            || (a.head == CheckedSourceInstanceHead::Sized && a.arguments != b.arguments)
    }

    /// A value of `[element; capacity]` is `capacity` elements long.
    pub(super) fn bind_sized_extent(&mut self, owner: usize, ty: SourceType) {
        if self.proof.numbers.is_empty() {
            return;
        }
        let SourceType::Array { id } = ty else {
            return;
        };
        let instance = &self.module.instances[id as usize];
        if instance.head != CheckedSourceInstanceHead::Sized {
            return;
        }
        let Some(number) = self.is_number_generic(instance.arguments[1]) else {
            return;
        };
        self.proof.owners[owner].extent = number.value.and_then(|value| value.identity);
    }

    /// An integer literal, a constant, or a number parameter written where a type argument
    /// goes. Returns `None` when `expression` names a type or something else.
    pub(super) fn number_argument(&mut self, expression: &'a Expression) -> Result<SourceType> {
        let value = self.evaluate(expression, None)?;
        self.scalar(value)?;
        if value.lower != value.upper {
            return Err(SourceCheckError::Unsupported);
        }
        Ok(SourceType::Integer {
            lower: value.lower,
            upper: value.lower,
        })
    }

    /// Each number argument is a number within its parameter's range; each type argument is
    /// not a number.
    pub(super) fn check_number_arguments(
        &mut self,
        schema: usize,
        arguments: &[SourceType],
    ) -> Result<()> {
        let ranges = self.number_ranges(schema)?;
        if ranges.is_empty() {
            return Ok(());
        }
        self.charge(arguments.len())?;
        for (position, argument) in arguments.iter().enumerate() {
            let range = ranges.get(position).copied().flatten();
            match (range, *argument) {
                (Some((lower, upper)), SourceType::Integer { lower: a, upper: b }) => {
                    if a != b {
                        let detail = "a number parameter takes one known number".to_owned();
                        return Err(self.explain(SourceCheckError::Type, detail));
                    }
                    if a < lower || a > upper {
                        let detail = format!("{a} is outside the range {lower} to {upper} this parameter accepts");
                        return Err(self.explain(SourceCheckError::Type, detail));
                    }
                }
                (Some((lower, upper)), generic @ SourceType::Generic { .. }) => {
                    let inside = self.is_number_generic(generic).is_some_and(|number| {
                        number.lower >= lower && number.upper <= upper
                    });
                    if !inside {
                        let detail = "this is not a number within the parameter's range".to_owned();
                        return Err(self.explain(SourceCheckError::Type, detail));
                    }
                }
                (Some(_), _) => {
                    let detail = "this parameter takes a number, not a type".to_owned();
                    return Err(self.explain(SourceCheckError::Type, detail));
                }
                (None, SourceType::Integer { lower, upper }) if !ranges.is_empty() && lower == upper => {
                    let detail = "this parameter takes a type, not a number".to_owned();
                    return Err(self.explain(SourceCheckError::Type, detail));
                }
                (None, _) => {}
            }
        }
        Ok(())
    }

    /// `capacity` in a claim: the parameter itself, as a term.
    pub(super) fn number_term(&mut self, name: &str) -> Result<Term> {
        let index = self
            .proof
            .generics
            .iter()
            .position(|other| *other == name)
            .ok_or(SourceCheckError::UnknownName)? as u32;
        if self.is_number_generic(SourceType::Generic { index }).is_none() {
            return Err(SourceCheckError::UnknownName);
        }
        Ok(Term::Number(index))
    }

    /// A call's constraints with the number terms replaced by the numbers the call supplies.
    pub(super) fn number_constraints(
        &mut self,
        constraints: &[Constraint],
        types: &[SourceType],
    ) -> Result<Vec<Constraint>> {
        // The copy is charged where `constraints` applies it.
        let mut output = Vec::with_capacity(constraints.len());
        for constraint in constraints {
            let mut constraint = constraint.clone();
            constraint.right = self.substitute_term(&constraint.right, types)?;
            // The left side of a claim over arithmetic (`count * #size(item)`) names the type
            // arguments too.
            if let Some(term) = &constraint.left_term {
                constraint.left_term = Some(self.substitute_term(term, types)?);
            }
            if let Term::Number(index) = constraint.right {
                match types.get(index as usize) {
                    Some(SourceType::Integer { lower, upper }) if lower == upper => {
                        constraint.right = Term::Constant(*lower);
                    }
                    // The caller's own abstract number: its scalar stands in, so the claim is
                    // proved from what the caller knows.
                    Some(generic @ SourceType::Generic { .. }) => {
                        let identity = self
                            .is_number_generic(*generic)
                            .and_then(|number| number.value)
                            .and_then(|value| value.identity);
                        let Some(identity) = identity else {
                            return Err(SourceCheckError::Unsupported);
                        };
                        constraint.right = Term::Identity(identity);
                    }
                    _ => {
                        let detail = "a claim over a number parameter needs the number the call supplies".to_owned();
                        return Err(self.explain(SourceCheckError::Unsupported, detail));
                    }
                }
            }
            output.push(constraint);
        }
        Ok(output)
    }
}

/// Whether `method` declares type parameters of its own (a bare name, `has Trait`, or `Type`),
/// as opposed to using the ones its generic type declares.
pub(super) fn writes_type_parameters(method: &Binding) -> bool {
    method.parameters.as_ref().is_some_and(|parameters| {
        parameters.value.iter().any(|parameter| match &parameter.value.annotation {
            None => parameter.value.name.value != "self",
            Some(annotation) => match &annotation.value {
                Annotation::Bound { .. } => true,
                Annotation::Expression(expression) => {
                    matches!(&expression.value, Expr::Name(name) if name == "Type")
                }
                _ => false,
            },
        })
    })
}

impl<'a> Checker<'a> {
    /// `left <= right` written as a claim: `left` names a parameter, `right` is a term.
    pub(super) fn relation_constraint(
        &mut self,
        relation: &'a Expression,
        names: &[&str],
        output: &mut Vec<Constraint>,
    ) -> Result<()> {
        let Expr::Binary { left, operator, right } = &relation.value else {
            return Err(SourceCheckError::Unsupported);
        };
        self.child(relation.span_id, left.span_id)?;
        self.child(relation.span_id, right.span_id)?;
        self.child(relation.span_id, operator.span_id)?;
        self.comparison(operator)?;
        // `address % 4096 = 0`: the parameter is a multiple of the modulus.
        if operator.value == BinaryOperator::Equal
            && let Expr::Binary { left: inner, operator: modulo, right: modulus } = &left.value
            && modulo.value == BinaryOperator::Remainder
            && let Expr::Name(name) = &inner.value
            && self.constant_expression(right)? == I256::ZERO
        {
            self.child(left.span_id, inner.span_id)?;
            self.token(inner.span_id, name)?;
            let slot = names
                .iter()
                .position(|other| *other == name)
                .ok_or(SourceCheckError::UnknownName)?;
            let modulus = self.constant_expression(modulus)?;
            output.push(Constraint {
                left: slot,
                left_field: None,
                left_term: None,
                modulus: Some(modulus),
                operator: operator.value,
                right: Term::Constant(I256::ZERO),
            });
            return Ok(());
        }
        // `count * #size(element) <= max`: arithmetic on the left.
        if let Some(term) = self.arithmetic_term(left, names)? {
            let right = self.contract_term(right, names)?;
            output.push(Constraint {
                left: 0,
                left_field: None,
                left_term: Some(term),
                modulus: None,
                operator: operator.value,
                right,
            });
            return Ok(());
        }
        // `buffer.length < cap`: a field of a parameter on the left.
        if let Expr::Member { base, name: field } = &left.value {
            let Term::Path { slot, field, deeper } = self.path_term(left, base, field, names)? else {
                return Err(SourceCheckError::Malformed);
            };
            // A deeper path on the left (`self.storage.length < cap`) is a term, not a field.
            if !deeper.is_empty() {
                let term = Term::Path { slot, field, deeper };
                let right = self.contract_term(right, names)?;
                output.push(Constraint { left: 0, left_field: None, left_term: Some(term), modulus: None, operator: operator.value, right });
                return Ok(());
            }
            let right = self.contract_term(right, names)?;
            output.push(Constraint { left: slot, left_field: Some(field), left_term: None, modulus: None, operator: operator.value, right });
            return Ok(());
        }
        let Expr::Name(name) = &left.value else {
            return Err(SourceCheckError::Unsupported);
        };
        self.token(left.span_id, name)?;
        let slot = names
            .iter()
            .position(|other| *other == name)
            .ok_or(SourceCheckError::UnknownName)?;
        let right = self.contract_term(right, names)?;
        output.push(Constraint { left: slot, left_field: None, left_term: None, modulus: None, operator: operator.value, right });
        Ok(())
    }

    /// `self.len` in a claim: the position of the field in the parameter's record, read from
    /// the type the parameter was just given.
    pub(super) fn path_term(
        &mut self,
        expression: &'a Expression,
        base: &'a Expression,
        field: &'a Name,
        names: &[&str],
    ) -> Result<Term> {
        self.child(expression.span_id, base.span_id)?;
        self.child(expression.span_id, field.span_id)?;
        self.name(field)?;
        // `self.storage.length`: the path to `storage`, then `length` in its type.
        if let Expr::Member { base: inner, name: inner_field } = &base.value {
            let Term::Path { slot, field: first, mut deeper } =
                self.path_term(base, inner, inner_field, names)?
            else {
                return Err(SourceCheckError::Malformed);
            };
            let ty = self.path_type(slot, first, &deeper)?;
            let more = self.resolve_member_path(ty, &field.value)?;
            deeper.extend(more);
            return Ok(Term::Path { slot, field: first, deeper });
        }
        let Expr::Name(root) = &base.value else {
            return Err(SourceCheckError::Unsupported);
        };
        self.token(base.span_id, root)?;
        let slot = names
            .iter()
            .position(|other| *other == root)
            .ok_or(SourceCheckError::UnknownName)?;
        // The parameters being declared are the first `names.len()` of `proof.names`.
        let known = self.proof.names.get(slot).filter(|(name, _)| *name == root.as_str());
        let Some((_, value)) = known else {
            // A claim written on a type's own field: `cell Cell and cell.count = 1`.
            if self.claim_schema.is_some() {
                return self.schema_path_term(slot, field);
            }
            return Err(SourceCheckError::Unsupported);
        };
        let SourceType::Aggregate { id } = value.ty else {
            return Err(SourceCheckError::Type);
        };
        let schema = self.module.instances[id as usize].schema();
        self.charge(self.module.layouts[schema].schema.fields.len())?;
        let layout = &self.module.layouts[schema].schema;
        let owner = layout.name.clone();
        let own_schema = self.proof.receiver_schema == Some(schema);
        // A field is read directly only by its own type; any other claim names an accessor.
        let mut written = field.value.clone();
        let direct = layout.fields.iter().position(|candidate| candidate.name == written);
        if direct.is_none() {
            self.charge(self.accessors.len())?;
            let unfolded = self
                .accessors
                .iter()
                .find(|(type_name, method, _)| *type_name == owner && *method == field.value)
                .map(|(_, _, returned)| returned.clone());
            written = unfolded.ok_or(SourceCheckError::UnknownName)?;
            // An accessor that reads deeper (`return self.storage.length`): the whole path.
            if let Some((head, rest)) = written.split_once('.') {
                let head = head.to_owned();
                let rest = rest.to_owned();
                let layout = &self.module.layouts[schema].schema;
                let first = layout
                    .fields
                    .iter()
                    .position(|candidate| candidate.name == head)
                    .ok_or(SourceCheckError::UnknownName)?;
                let ty = self.instance_fields(id)?[first];
                let mut deeper = Vec::new();
                let mut ty = ty;
                for segment in rest.split('.') {
                    let path = self.resolve_member_path(ty, segment)?;
                    for step in &path {
                        ty = self.step_type(ty, *step)?;
                    }
                    deeper.extend(path);
                }
                return Ok(Term::Path { slot, field: first as u32, deeper });
            }
        }
        let layout = &self.module.layouts[schema].schema;
        let position = layout
            .fields
            .iter()
            .position(|candidate| candidate.name == written)
            .ok_or(SourceCheckError::UnknownName)?;
        if layout.kind == AggregateKind::Domain {
            return Err(SourceCheckError::Privacy);
        }
        if direct.is_some() && layout.fields[position].private && !own_schema {
            return Err(SourceCheckError::Privacy);
        }
        Ok(Term::Path { slot, field: position as u32, deeper: Vec::new() })
    }

    /// The scalar a `Term::Number` stands for inside the method that declared it.
    pub(super) fn number_term_value(&mut self, index: u32) -> Result<Value> {
        self.proof
            .numbers
            .iter()
            .find(|number| number.index == index)
            .and_then(|number| number.value)
            .ok_or(SourceCheckError::Unsupported)
    }

    /// The field at `position` of the record `value`, as its own child.
    pub(super) fn field_child(&mut self, value: Value, position: u32) -> Result<Value> {
        let value = self.owner_view(value)?;
        if !matches!(value.ty, SourceType::Aggregate { .. }) {
            return Err(SourceCheckError::Type);
        }
        self.materialize_children(value)?;
        let owner = value.owner.ok_or(SourceCheckError::Type)?;
        self.proof.owners[owner]
            .children
            .get(position as usize)
            .copied()
            .ok_or(SourceCheckError::Type)
    }

    /// A linear domain value as the number it holds; any other value unchanged.
    pub(super) fn domain_number(&mut self, value: Value) -> Result<Value> {
        if self.domain_bounds_of(value.ty).is_none() {
            return Ok(value);
        }
        self.field_child(value, 0)
    }
}

impl<'a> Checker<'a> {
    /// The caller's number parameter whose scalar has this identity.
    pub(super) fn identity_value(&mut self, identity: u32) -> Result<Value> {
        let value = self
            .proof
            .numbers
            .iter()
            .filter_map(|number| number.value)
            .find(|value| value.identity == Some(identity))
            .ok_or(SourceCheckError::Unsupported)?;
        self.scalar_facts(value)
    }
}

/// Whether `name` is written as the length of an array type in some parameter or the result of
/// `binding`: `[element; name]`. A parameter of a free function so used is a compile-time
/// number, as a number parameter of a type is.
pub(super) fn names_array_length(binding: &Binding, name: &str) -> bool {
    fn within(expression: &Expression, name: &str, depth: u32) -> bool {
        if depth > 16 {
            return false;
        }
        match &expression.value {
            Expr::Array(array) => {
                array.length.as_ref().is_some_and(|length| {
                    matches!(&length.value.value, Expr::Name(other) if other == name)
                }) || array.elements.iter().any(|e| within(e, name, depth + 1))
            }
            Expr::PrefixedType(prefix) => within(&prefix.target, name, depth + 1),
            Expr::Group(inner) => within(inner, name, depth + 1),
            Expr::Call { arguments, .. } => arguments.value.iter().any(|argument| {
                matches!(&argument.value, Argument::Positional(e) if within(e, name, depth + 1))
            }),
            Expr::Sequence(sequence) => sequence.elements.iter().any(|e| within(e, name, depth + 1)),
            _ => false,
        }
    }
    let annotated = |annotation: &Spanned<Annotation>| match &annotation.value {
        Annotation::Expression(e) => within(e, name, 0),
        Annotation::Refined { base: Some(e), .. } => within(e, name, 0),
        _ => false,
    };
    binding
        .parameters
        .as_ref()
        .is_some_and(|parameters| {
            parameters
                .value
                .iter()
                .any(|p| p.value.annotation.as_ref().is_some_and(annotated))
        })
        || binding.annotation.as_ref().is_some_and(annotated)
}

impl<'a> Checker<'a> {
    /// `field.member` in a claim on a field of the type being declared. The field's type must
    /// be a record declared in this module (an accessor of an imported type is not exported
    /// yet); `member` is one of its fields, or an inline accessor that returns one. A private
    /// field is named only through its accessor.
    fn schema_path_term(&mut self, slot: usize, member: &'a Name) -> Result<Term> {
        let binding = *self.claim_fields.get(slot).ok_or(SourceCheckError::Unsupported)?;
        let annotation = binding.annotation.as_ref().ok_or(SourceCheckError::Type)?;
        let base = match &annotation.value {
            Annotation::Expression(expression) => expression,
            Annotation::Refined { base: Some(expression), .. } => expression,
            _ => return Err(SourceCheckError::Unsupported),
        };
        let head = match &base.value {
            Expr::Name(name) => name,
            Expr::Call { callee, .. } => match &callee.value {
                Expr::Name(name) => name,
                _ => return Err(SourceCheckError::Unsupported),
            },
            _ => return Err(SourceCheckError::Unsupported),
        };
        self.charge(self.module.schemas.len())?;
        // (name, private) of each field of the type, local or imported.
        let local = self
            .module
            .schemas
            .iter()
            .position(|declaration| declaration.name.value == *head && matches!(declaration.body.value, TypeBody::Has(_)));
        let fields: Vec<(String, bool)> = match local {
            Some(target) => self
                .fields_of(self.module.schemas[target])?
                .iter()
                .map(|field| {
                    let private = field
                        .attributes
                        .iter()
                        .any(|attribute| matches!(attribute.value, FieldAttribute::Private { .. }));
                    (field.name.value.clone(), private)
                })
                .collect(),
            None => {
                // An imported type: its checked layout lists the fields.
                self.charge(self.module.type_names.len())?;
                let (_, index) = self
                    .module
                    .type_names
                    .iter()
                    .find(|(name, _)| name == head)
                    .ok_or(SourceCheckError::Unsupported)?;
                let layout = &self.module.layouts[*index].schema;
                if layout.kind != AggregateKind::Record {
                    return Err(SourceCheckError::Unsupported);
                }
                layout.fields.iter().map(|field| (field.name.clone(), field.private)).collect()
            }
        };
        let direct = fields.iter().position(|(name, _)| *name == member.value);
        let written = match direct {
            Some(_) => member.value.clone(),
            None => self
                .accessors
                .iter()
                .find(|(owner, method, _)| owner == head && *method == member.value)
                .map(|(_, _, field)| field.clone())
                .ok_or(SourceCheckError::Unsupported)?,
        };
        let position = fields
            .iter()
            .position(|(name, _)| *name == written)
            .ok_or(SourceCheckError::UnknownName)?;
        let own = local.is_some() && local == self.claim_schema;
        if direct.is_some() && fields[position].1 && !own {
            return Err(SourceCheckError::Privacy);
        }
        Ok(Term::Path { slot, field: position as u32, deeper: Vec::new() })
    }
}

impl<'a> Checker<'a> {
    /// The type at `slot`'s `first` field and then `deeper` fields.
    fn path_type(&mut self, slot: usize, first: u32, deeper: &[u32]) -> Result<SourceType> {
        let (_, value) = *self.proof.names.get(slot).ok_or(SourceCheckError::Unsupported)?;
        let mut ty = self.step_type(value.ty, first)?;
        for next in deeper {
            ty = self.step_type(ty, *next)?;
        }
        Ok(ty)
    }

    /// The type of field `index` of the record type `ty`.
    fn step_type(&mut self, ty: SourceType, index: u32) -> Result<SourceType> {
        let SourceType::Aggregate { id } = ty else {
            return Err(SourceCheckError::Type);
        };
        self.instance_fields(id)?
            .get(index as usize)
            .copied()
            .ok_or(SourceCheckError::Type)
    }

    /// `member` of a value of type `ty`, as field positions: a field the claim may read (one that
    /// is not private to another type), or an accessor's path, unfolded (accessors may read
    /// further accessors).
    fn resolve_member_path(&mut self, ty: SourceType, member: &str) -> Result<Vec<u32>> {
        let mut pending = vec![member.to_owned()];
        let mut ty = ty;
        let mut path = Vec::new();
        let mut steps = 0;
        while let Some(member) = pending.pop() {
            steps += 1;
            if steps > 16 {
                return Err(SourceCheckError::Unsupported);
            }
            let SourceType::Aggregate { id } = ty else {
                return Err(SourceCheckError::Type);
            };
            let schema = self.module.instances[id as usize].schema();
            let layout = &self.module.layouts[schema].schema;
            if layout.kind == AggregateKind::Domain {
                return Err(SourceCheckError::Privacy);
            }
            if let Some(position) = layout.fields.iter().position(|field| field.name == member) {
                if layout.fields[position].private && self.proof.receiver_schema != Some(schema) {
                    return Err(SourceCheckError::Privacy);
                }
                path.push(position as u32);
                ty = self.step_type(ty, position as u32)?;
                continue;
            }
            let owner = layout.name.clone();
            self.charge(self.accessors.len())?;
            let target = self
                .accessors
                .iter()
                .find(|(type_name, method, _)| *type_name == owner && *method == member)
                .map(|(_, _, field)| field.clone())
                .ok_or(SourceCheckError::UnknownName)?;
            // Through the accessor the private field is the type's own reading of it.
            let mut segments: Vec<&str> = target.split('.').collect();
            let first = segments.remove(0);
            let layout = &self.module.layouts[schema].schema;
            let position = layout
                .fields
                .iter()
                .position(|field| field.name == first)
                .ok_or(SourceCheckError::UnknownName)?;
            path.push(position as u32);
            ty = self.step_type(ty, position as u32)?;
            for segment in segments.into_iter().rev() {
                pending.push(segment.to_owned());
            }
        }
        Ok(path)
    }
}
