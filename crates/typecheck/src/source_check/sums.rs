//! Bounded active-variant worlds. A sum never owns all its alternatives at once.
//!
//! Worlds are replayed only while verifying a body. Published outcomes contain flat
//! input guards and origins, not syntax; calls select feasible verified outcomes.
use super::*;
use contracts::{Layout, Origin};

/// How deep one pattern may nest others (`Done(Full(Inner(v)))` is two).
const PATTERN_NESTING_MAX: usize = 8;

/// A detached static-checking context. Hypothetical payload witnesses and facts
/// never enter a feasible world's output summary.
#[derive(Clone)]
pub(super) struct ValidationState<'a> {
    pub(super) names: Vec<(&'a str, Value)>,
    pub(super) owners: Vec<aggregates::Owner>,
    pub(super) facts: Vec<(u32, BinaryOperator, u32)>,
    pub(super) scalars: Vec<(u32, I256, I256)>,
    pub(super) integer_guards: Vec<(u32, I256)>,
    pub(super) multiples: Vec<(u32, I256)>,
    pub(super) next_identity: u32,
}

impl<'a> ProofState<'a> {
    pub(super) fn validation_snapshot(&self) -> ValidationState<'a> {
        ValidationState {
            names: self.names.clone(),
            owners: self.owners.clone(),
            facts: self.facts.clone(),
            scalars: self.scalars.clone(),
            integer_guards: self.integer_guards.clone(),
            multiples: self.multiples.clone(),
            next_identity: self.next_identity,
        }
    }

    pub(super) fn restore_validation(&mut self, state: ValidationState<'a>) {
        self.names = state.names;
        self.owners = state.owners;
        self.facts = state.facts;
        self.scalars = state.scalars;
        self.integer_guards = state.integer_guards;
        self.multiples = state.multiples;
        self.next_identity = state.next_identity;
        // Identities are reused after a restore: equations about the ones made since are gone.
        let next = self.next_identity;
        self.definitions.retain(|definition| definition.result < next);
        self.layout_ids.retain(|(_, _, identity)| *identity < next);
    }
}

/// The most worlds (outcome combinations that really differ) one function may have. Each world
/// re-checks the whole body, so the cost is exponential in the number of calls. Measured 2026-11-05: `draw` in `terminal_demo` has 1024,
/// `decimal` in `terminal.output` 320, `read_digits` in `core.io` 144, `enter` 49; the cap leaves
/// 2x headroom over the largest.
pub(super) const WORLDS_MAX: usize = 2048;

impl<'a> Checker<'a> {
    pub(super) fn head_type(&self, head: CheckedSourceInstanceHead, id: u32) -> SourceType {
        match head {
            CheckedSourceInstanceHead::Nominal { schema } => {
                self.instance_type(schema as usize, id)
            }
            CheckedSourceInstanceHead::Array { .. }
            | CheckedSourceInstanceHead::Slice
            | CheckedSourceInstanceHead::Sized => SourceType::Array { id },
            CheckedSourceInstanceHead::Sequence => SourceType::Sequence { id },
            CheckedSourceInstanceHead::Pointer => SourceType::Pointer { id },
        }
    }

    /// The declared sum whose members are exactly `members`, in any order.
    pub(super) fn sum_of_members(&self, members: &[SourceType]) -> Result<SourceType> {
        for (id, instance) in self.module.instances.iter().enumerate() {
            let CheckedSourceInstanceHead::Nominal { schema } = instance.head else {
                continue;
            };
            let Some(layout) = self.module.layouts.get(schema as usize) else {
                continue;
            };
            let variants = &layout.schema.variants;
            let matches = instance.arguments.is_empty()
                && variants.len() == members.len()
                && members.iter().all(|member| variants.contains(member));
            if matches && matches!(self.head_type(instance.head, id as u32), SourceType::Sum { .. }) {
                return Ok(self.head_type(instance.head, id as u32));
            }
        }
        Err(SourceCheckError::Unsupported)
    }

    /// The declared sum whose members are exactly `members`, also a generic one
    /// (`ReserveResult(x) is Vacant(x) or AllocError` for `Vacant(x) or AllocError`): its
    /// arguments are read from a member that is an application of the same arity.
    pub(super) fn generic_sum_of_members(&mut self, members: &[SourceType]) -> Result<SourceType> {
        if let Ok(found) = self.sum_of_members(members) {
            return Ok(found);
        }
        let mut candidates = Vec::new();
        for member in members {
            if let SourceType::Aggregate { id } | SourceType::Sum { id } = member {
                let arguments = self.module.instances[*id as usize].arguments.clone();
                if !arguments.is_empty() && !candidates.contains(&arguments) {
                    candidates.push(arguments);
                }
            }
        }
        // A sum member is its own members in a declared union (`Vacant(x) or AllocError` is
        // four variants when `AllocError` has three).
        let mut flat = Vec::new();
        for member in members {
            match member {
                SourceType::Sum { id } => flat.extend(self.sum_variants(*id)?),
                other => flat.push(*other),
            }
        }
        let members = flat.as_slice();
        if let Ok(found) = self.sum_of_members(members) {
            return Ok(found);
        }
        self.charge(self.module.layouts.len() * (candidates.len() + 1))?;
        for schema in 0..self.module.layouts.len() {
            let layout = &self.module.layouts[schema].schema;
            if layout.variants.len() != members.len() || layout.parameters.is_empty() {
                continue;
            }
            let arity = layout.parameters.len();
            for arguments in &candidates {
                if arguments.len() != arity {
                    continue;
                }
                let SourceType::Sum { id } = self.instance(schema, arguments.clone())? else {
                    continue;
                };
                let variants = self.sum_variants(id)?;
                if members.iter().all(|member| variants.contains(member)) {
                    return Ok(SourceType::Sum { id });
                }
            }
        }
        Err(SourceCheckError::Unsupported)
    }

    pub(super) fn instance_type(&self, schema: usize, id: u32) -> SourceType {
        let sum = if schema < self.module.schemas.len() {
            matches!(self.module.schemas[schema].body.value, TypeBody::Is(_))
        } else {
            !self.module.layouts[schema].schema.variants.is_empty()
        };
        if sum {
            SourceType::Sum { id }
        } else {
            SourceType::Aggregate { id }
        }
    }

    /// Every name in a union's `or` list declares a new variant of the union, owned by it: one
    /// that carries nothing (`Red`), wraps one payload (`User(User)`), or is encoded as a literal
    /// (`Debug = 'debug'`). An `or` list never refers to an existing type: a bare name that
    /// matches a type in scope is refused, with the wrapped form suggested, and a type applied
    /// to several arguments is refused the same way. There is no declaration to point at, so a
    /// variant is a layout past the declared ones, like an imported one, named `Union.Variant`.
    /// It is not in the module-wide type table: `Name.Red` and `Color.Red` are different types,
    /// found through their union, from the expected type of a bare name, or from the subject of
    /// a `when`.
    ///
    /// A declaration that is a single name (`Alias is Other`) or a single application is not an
    /// `or` list and keeps its old reading: an alias when the name is a type.
    ///
    /// Work is charged only for a variant actually created.
    pub(super) fn declare_union_variants(&mut self) -> diagnostics::DiagnosticResult<()> {
        for schema in 0..self.module.schemas.len() {
            let declaration = self.module.schemas[schema];
            let TypeBody::Is(root) = &declaration.body.value else {
                continue;
            };
            let listed = matches!(&root.value, Expr::Binary { operator, .. } if operator.value == BinaryOperator::Or);
            let mut pending = vec![root];
            let mut encoded = Vec::new();
            let mut plain = 0usize;
            let mut payloads = 0usize;
            while let Some(expression) = pending.pop() {
                let context = expression.span_id;
                match &expression.value {
                    Expr::Binary {
                        left,
                        operator,
                        right,
                    } if operator.value == BinaryOperator::Or => {
                        pending.push(right);
                        pending.push(left);
                    }
                    Expr::Name(name) => {
                        if listed {
                            self.listed_name(name, None)
                                .map_err(|error| self.failure(error, context))?;
                        }
                        let created = self.module.union_variants.len();
                        self.declare_variant(schema, name, listed)
                            .map_err(|error| self.failure(error, context))?;
                        plain += usize::from(self.module.union_variants.len() > created);
                    }
                    // `Debug = 'debug'`: a variant that carries nothing, encoded as the literal.
                    Expr::Binary { left, operator, right } if operator.value == BinaryOperator::Equal => {
                        let Expr::Name(name) = &left.value else {
                            let detail = "a discriminant is written `Variant = literal`".to_owned();
                            let error = self.explain(SourceCheckError::Type, detail);
                            return Err(self.failure(error, context));
                        };
                        self.listed_name(name, Some(()))
                            .map_err(|error| self.failure(error, context))?;
                        let value = self
                            .discriminant_literal(right)
                            .map_err(|error| self.failure(error, right.span_id))?;
                        self.declare_variant(schema, name, true)
                            .map_err(|error| self.failure(error, context))?;
                        let layout = self.own_variant(schema, name).ok_or_else(|| self.failure(SourceCheckError::Malformed, context))?;
                        encoded.push((layout, value, right.span_id));
                    }
                    // `Opened(Session)`: a new variant holding one value. Outside an `or` list,
                    // a type of that name is applied instead (an alias of the application).
                    Expr::Call { callee, arguments } => {
                        let single = match &callee.value {
                            Expr::Name(name) => payload_fields(&arguments.value).map(|fields| (name, fields)),
                            _ => None,
                        };
                        let Some((name, fields)) = single else {
                            if listed {
                                let error = self.wrap_advice(expression);
                                return Err(self.failure(error, context));
                            }
                            continue;
                        };
                        let created = self.module.union_variants.len();
                        self.declare_variant(schema, name, listed)
                            .map_err(|error| self.failure(error, context))?;
                        if self.module.union_variants.len() == created {
                            continue;
                        }
                        payloads += 1;
                        if let Some(layout) = self.own_variant(schema, name) {
                            self.module.variant_payloads.push((layout, fields));
                            // It ranges over the union's parameters, so it can be applied.
                            let names: Vec<String> = self.module.schemas[schema]
                                .parameters
                                .as_ref()
                                .map(|parameters| {
                                    parameters
                                        .value
                                        .iter()
                                        .map(|parameter| parameter.value.name.value.clone())
                                        .collect()
                                })
                                .unwrap_or_default();
                            self.module.layouts[layout].schema.parameters = names;
                        }
                    }
                    _ if listed => {
                        let error = self.wrap_advice(expression);
                        return Err(self.failure(error, context));
                    }
                    _ => {}
                }
            }
            if !encoded.is_empty() {
                let span = declaration.body.span_id;
                self.check_discriminants(&encoded, plain, payloads)
                    .map_err(|(error, at)| self.failure(error, at.unwrap_or(span)))?;
                for (layout, value, _) in encoded {
                    self.module.layouts[layout].schema.discriminant = Some(value);
                }
            }
        }
        Ok(())
    }

    /// A name written in an `or` list declares a variant. One that matches a type in scope used
    /// to make that type a member; now it is refused, and the wrapped form is suggested. A
    /// discriminant variant (`encoded`) carries nothing, so it cannot be wrapped: rename it.
    fn listed_name(&mut self, name: &str, encoded: Option<()>) -> Result<()> {
        if !name.starts_with(|first: char| first.is_ascii_uppercase()) {
            let detail = format!(
                "`{name}` in an `or` list is not a variant: a variant is a capitalized name, carrying nothing or wrapping one payload (`Variant({name})`)"
            );
            return Err(self.explain(SourceCheckError::Type, detail));
        }
        let resolves = self
            .module
            .type_names
            .binary_search_by(|(other, _)| other.as_str().cmp(name))
            .is_ok();
        if !resolves {
            return Ok(());
        }
        let detail = match encoded {
            None => format!(
                "`{name}` in an `or` list declares a new variant that carries nothing, but a type `{name}` is in scope; to make the type a member, wrap it: `{name}({name})`"
            ),
            Some(()) => format!(
                "`{name}` in an `or` list declares a new variant, but a type `{name}` is in scope; a variant with a value carries nothing, so give it another name"
            ),
        };
        Err(self.explain(SourceCheckError::Type, detail))
    }

    /// The sum a header written as variants returns when the variants are its own
    /// (`Allocated(List(element, alloc)) where (...) or Failed(ListFailure(alloc))`): a sum and a
    /// variant layout for each, over the function's type parameters, built the first time the
    /// header is read and kept by binding. `None` when the header writes no alternatives.
    pub(super) fn hidden_result(
        &mut self,
        binding: &'a Binding,
    ) -> Result<Option<(SourceType, Vec<SourceType>)>> {
        let key = binding as *const Binding as usize;
        if let Some((_, sum, members)) = self.module.hidden_results.iter().find(|(other, _, _)| *other == key) {
            return Ok(Some((*sum, members.clone())));
        }
        // A provider's result is the sum its trait's method declares, over the provider.
        if let Some(found) = self.provider_result(binding, key)? {
            return Ok(Some(found));
        }
        let generics: Vec<&'a str> = self.proof.generics.clone();
        let owner = match self.proof.receiver_schema {
            Some(schema) => format!("{}.{}", self.module.schemas[schema].name.value, binding.name.value),
            None => binding.name.value.clone(),
        };
        self.build_hidden(binding, key, &generics, owner)
    }

    /// The type parameters a method writes (not the receiver's): a bare name, `name has Trait`, or
    /// `name Type`.
    fn written_type_parameters(binding: &'a Binding) -> Vec<&'a str> {
        binding
            .parameters
            .iter()
            .flat_map(|parameters| parameters.value.iter())
            .filter(|parameter| {
                let parameter = &parameter.value;
                parameter.name.value != "self"
                    && match &parameter.annotation {
                        None => true,
                        Some(annotation) => match &annotation.value {
                            Annotation::Bound { .. } => true,
                            Annotation::Expression(e) => matches!(&e.value, Expr::Name(n) if n == "Type"),
                            _ => false,
                        },
                    }
            })
            .map(|parameter| parameter.value.name.value.as_str())
            .collect()
    }

    /// When `binding` is a method of a type that provides a trait declaring a method of that name with
    /// inline variants, the provider's result: the trait's sum over the provider. The header must
    /// repeat the trait's variants (names, payload fields and types) and `where` claims.
    fn provider_result(
        &mut self,
        binding: &'a Binding,
        key: usize,
    ) -> Result<Option<(SourceType, Vec<SourceType>)>> {
        if binding.alternatives.is_empty() {
            return Ok(None);
        }
        let (Some(schema), Some(receiver)) = (self.proof.receiver_schema, self.proof.receiver) else {
            return Ok(None);
        };
        let provided = self.module.layouts[schema].schema.provides.clone();
        let mut found: Option<(&'a TypeDeclaration, &'a Binding)> = None;
        for (declaration, _) in self.module.traits.clone() {
            if !provided.iter().any(|name| *name == declaration.name.value) {
                continue;
            }
            let TypeBody::Has(members) = &declaration.body.value else { continue };
            for member in members {
                if let TypeMember::Binding(method) = &member.value
                    && method.name.value == binding.name.value
                    && method.value.is_none()
                    && !method.alternatives.is_empty()
                {
                    found = Some((declaration, method));
                }
            }
        }
        let Some((declaration, method)) = found else {
            return Ok(None);
        };
        // The trait's sum ranges over what its bodiless method does: `Self`, the trait's own
        // parameters, then the type parameters the method writes.
        let trait_parameters: Vec<&'a str> = declaration
            .parameters
            .iter()
            .flat_map(|parameters| parameters.value.iter())
            .map(|parameter| parameter.value.name.value.as_str())
            .collect();
        // A trait with parameters takes them from the provision (`Thing has Maker(Code)`),
        // resolved in the provider's scope.
        let mut provision_arguments = Vec::new();
        if !trait_parameters.is_empty() {
            let has = self.module.schema_domain_has[schema].unwrap_or(self.module.schemas[schema]);
            let TypeBody::Has(members) = &has.body.value else {
                return Err(SourceCheckError::Malformed);
            };
            let written = members
                .iter()
                .filter_map(Self::provision_parts)
                .find(|(name, _, _)| *name == declaration.name.value)
                .map(|(_, _, arguments)| arguments);
            let Some(written) = written.filter(|arguments| arguments.len() == trait_parameters.len()) else {
                let detail = format!(
                    "the provision of `{}` must give its {} parameters where the type declares it",
                    declaration.name.value,
                    trait_parameters.len()
                );
                return Err(self.explain(SourceCheckError::Type, detail));
            };
            let scope = self.generic_scope();
            self.charge(written.len())?;
            for argument in written {
                let Argument::Positional(argument) = &argument.value else {
                    return Err(SourceCheckError::Type);
                };
                provision_arguments.push(self.resolve_type(argument, &scope)?);
            }
        }
        let trait_written = Self::written_type_parameters(method);
        let mut trait_generics: Vec<&'a str> = vec!["Self"];
        trait_generics.extend(trait_parameters.iter().copied());
        trait_generics.extend(trait_written.iter().copied());
        let owner = format!("{}.{}", declaration.name.value, method.name.value);
        let trait_key = method as *const Binding as usize;
        let built = self.build_hidden(method, trait_key, &trait_generics, owner);
        let Some((sum, _)) = built? else {
            return Ok(None);
        };
        // The provider's generics: the receiver's own parameters, then what the method writes.
        let written = Self::written_type_parameters(binding);
        if written.len() != trait_written.len() {
            let detail = "the method must write the type parameters the trait's method writes".to_owned();
            return Err(self.explain(SourceCheckError::Type, detail));
        }
        let implicit = self.proof.generics.len() - written.len();
        let mut arguments = vec![receiver];
        arguments.extend(provision_arguments);
        arguments.extend((implicit..self.proof.generics.len()).map(|index| SourceType::Generic { index: index as u32 }));
        let SourceType::Sum { id } = sum else {
            return Ok(None);
        };
        let layout = self.module.instances[id as usize].schema();
        let ty = self.instance(layout, arguments);
        let ty = ty?;
        let SourceType::Sum { id } = ty else {
            return Ok(None);
        };
        let members = self.sum_variants(id)?;
        self.check_provider_variants(binding, &members)?;
        self.module.hidden_results.push((key, ty, members.clone()));
        Ok(Some((ty, members)))
    }

    /// The provider's header must declare the trait's variants: the same names in the same order,
    /// the same payload fields, of the same types (resolved here, so an imported trait's respelled
    /// names compare by what they mean). Its `where` claims are compared with the trait's where the
    /// provision is read.
    fn check_provider_variants(&mut self, binding: &'a Binding, members: &[SourceType]) -> Result<()> {
        let written: Vec<&'a Expression> = binding
            .annotation
            .iter()
            .chain(binding.alternatives.iter().map(|alternative| &alternative.value.annotation))
            .filter_map(|annotation| match &annotation.value {
                Annotation::Expression(expression) => Some(expression),
                _ => None,
            })
            .collect();
        let mismatch = |checker: &mut Self, why: String| {
            let detail = format!("the result of `{}` must declare the variants its trait declares: {why}", binding.name.value);
            Err(checker.explain(SourceCheckError::Type, detail))
        };
        if written.len() != members.len() {
            return mismatch(self, format!("{} variants written, {} declared", written.len(), members.len()));
        }
        let scope: Vec<(&'a str, SourceType)> = self
            .proof
            .generics
            .iter()
            .enumerate()
            .map(|(index, name)| (*name, SourceType::Generic { index: index as u32 }))
            .collect();
        for (expression, member) in written.into_iter().zip(members) {
            let (name, fields): (&str, Vec<(&str, &Expression)>) = match &expression.value {
                Expr::Name(name) => (name.as_str(), Vec::new()),
                Expr::Call { callee, arguments } => match (&callee.value, payload_fields(&arguments.value)) {
                    (Expr::Name(name), Some(fields)) => (name.as_str(), fields),
                    _ => return mismatch(self, "a variant is a name or a name with payload fields".to_owned()),
                },
                _ => return mismatch(self, "a variant is a name or a name with payload fields".to_owned()),
            };
            let SourceType::Aggregate { id } = member else {
                return mismatch(self, "the trait declares a variant that is not a record".to_owned());
            };
            let schema = self.module.instances[*id as usize].schema();
            let declared = self.module.layouts[schema].schema.name.clone();
            if declared.rsplit('.').next() != Some(name) {
                return mismatch(self, format!("`{name}` where the trait has `{}`", declared.rsplit('.').next().unwrap_or("")));
            }
            let declared_fields: Vec<String> =
                self.module.layouts[schema].schema.fields.iter().map(|field| field.name.clone()).collect();
            if declared_fields.len() != fields.len()
                || declared_fields.iter().zip(&fields).any(|(have, (want, _))| have != want)
            {
                return mismatch(self, format!("the payload of `{name}` differs"));
            }
            let types = self.instance_fields(*id)?;
            for ((_, written_type), expected) in fields.iter().zip(types) {
                let ty = self.resolve_type(written_type, &scope)?;
                if ty != expected {
                    return mismatch(self, format!("the payload type of `{name}` differs"));
                }
            }
        }
        Ok(())
    }

    /// Build the sum `binding` declares, over the given type parameters, under `owner`'s name.
    fn build_hidden(
        &mut self,
        binding: &'a Binding,
        key: usize,
        generics: &[&'a str],
        owner: String,
    ) -> Result<Option<(SourceType, Vec<SourceType>)>> {
        if let Some((_, sum, members)) = self.module.hidden_results.iter().find(|(other, _, _)| *other == key) {
            return Ok(Some((*sum, members.clone())));
        }
        let written: Vec<&'a Annotation> = binding
            .annotation
            .iter()
            .chain(binding.alternatives.iter().map(|alternative| &alternative.value.annotation))
            .map(|annotation| &annotation.value)
            .collect();
        let mut shapes: Vec<(&'a str, Option<Vec<(&'a str, &'a Expression)>>, &'a Expression)> = Vec::new();
        for annotation in written {
            let Annotation::Expression(expression) = annotation else {
                continue;
            };
            match &expression.value {
                Expr::Name(name) => {
                    shapes.push((name, None, expression));
                }
                Expr::Call { callee, arguments } => {
                    if let Expr::Name(name) = &callee.value
                        && let Some(fields) = payload_fields(&arguments.value)
                    {
                        shapes.push((name, Some(fields), expression));
                    } else {
                        shapes.push(("", None, expression));
                    }
                }
                _ => {
                    shapes.push(("", None, expression));
                }
            }
        }
        if binding.alternatives.is_empty() || binding.annotation.is_none() {
            return Ok(None);
        }
        // From here every written member is a variant of the function's own sum.
        for (name, payload, expression) in &shapes {
            if name.is_empty() {
                return Err(self.wrap_advice(expression));
            }
            if payload.is_none() {
                self.listed_name(name, None)?;
            }
        }
        for (index, (name, _, _)) in shapes.iter().enumerate() {
            if shapes[..index].iter().any(|(other, _, _)| other == name) {
                return Err(SourceCheckError::Duplicate);
            }
        }
        let scope: Vec<(&'a str, SourceType)> = generics
            .iter()
            .enumerate()
            .map(|(index, name)| (*name, SourceType::Generic { index: index as u32 }))
            .collect();
        let names: Vec<String> = scope.iter().map(|(name, _)| (*name).to_owned()).collect();
        let arguments: Vec<SourceType> = scope.iter().map(|(_, ty)| *ty).collect();
        let origin = |checker: &Self| Origin {
            module: checker.module.origin.clone(),
            declaration: checker.module.schemas.len()
                + checker.module.union_variants.len()
                + (1 << 20)
                + checker.module.layouts.len(),
        };
        let blank = |name: String, parameters: Vec<String>| CheckedSourceAggregate {
            name,
            parameters,
            fields: Vec::new(),
            variants: Vec::new(),
            capability: Capability::Unrestricted,
            kind: AggregateKind::Record,
            wrapping: false,
            defaults: Vec::new(),
            bounds: Vec::new(),
            provides: Vec::new(),
            borrowing: Borrowing::None,
            numbers: Vec::new(),
            domain_claims: Vec::new(),
            discriminant: None,
        };
        self.charge(shapes.len() * (names.len() + 2) + self.module.layouts.len())?;
        let mut members = Vec::with_capacity(shapes.len());
        let mut all_free = true;
        for (name, payload, _) in &shapes {
            let payload = payload.as_ref();
            let mut schema = blank(format!("{owner}.{name}"), names.clone());
            if let Some(fields) = payload {
                let mut capability = Capability::Unrestricted;
                for (position, (field, written)) in fields.iter().enumerate() {
                    if !field.is_empty() && fields[..position].iter().any(|(other, _)| other == field) {
                        let detail = format!("the variant `{name}` declares the field `{field}` twice");
                        return Err(self.explain(SourceCheckError::Duplicate, detail));
                    }
                    let ty = self.resolve_type(written, &scope)?;
                    if self.capability_of(ty) != Capability::Unrestricted {
                        capability = Capability::Linear;
                    }
                    schema.fields.push(CheckedSourceField {
                        name: (*field).to_owned(),
                        ty,
                        private: false,
                        register: None,
                        borrow: None,
                        length: None,
                    });
                }
                schema.capability = capability;
            }
            all_free &= schema.capability == Capability::Unrestricted;
            let layout = self.module.layouts.len();
            let placed = origin(self);
            let stable = Some(format!("{owner}#result.{name}"));
            self.module.layouts.push(Layout { origin: placed, schema, constraints: Vec::new(), key: stable });
            self.module.hidden_variants.push(layout);
            self.module.variant_field_names.push((
                layout,
                payload.map_or_else(Vec::new, |fields| fields.iter().map(|(field, _)| *field).collect()),
            ));
            members.push(self.instance(layout, arguments.clone())?);
        }
        let stable = Some(format!("{owner}#result"));
        let mut sum = blank(owner, names);
        sum.kind = AggregateKind::Sum;
        sum.variants = members.clone();
        sum.capability = if all_free { Capability::Unrestricted } else { Capability::Linear };
        let layout = self.module.layouts.len();
        let placed = origin(self);
        self.module.layouts.push(Layout { origin: placed, schema: sum, constraints: Vec::new(), key: stable });
        let ty = self.instance(layout, arguments)?;
        self.module.hidden_results.push((key, ty, members.clone()));
        Ok(Some((ty, members)))
    }

    /// Whether `schema` is a variant a union or a function declared itself, whose first payload
    /// may be given without its label. Uncharged.
    pub(super) fn is_declared_variant(&self, schema: usize) -> bool {
        self.module.hidden_variants.contains(&schema)
            || self.module.union_variants.iter().any(|(_, _, layout)| *layout == schema)
    }

    /// The refusal for an `or` member that is a type expression rather than a variant: a type
    /// applied to several arguments, a field, a prefixed type.
    fn wrap_advice(&mut self, expression: &'a Expression) -> SourceCheckError {
        let written = self.text_of(expression.span_id).unwrap_or("Type").to_owned();
        let head = match &expression.value {
            Expr::Call { callee, .. } => match &callee.value {
                Expr::Name(name) => name.clone(),
                _ => "Variant".to_owned(),
            },
            _ => "Variant".to_owned(),
        };
        let detail = format!(
            "an `or` list declares variants, not types: `{written}` is a type; wrap it in a variant, such as `{head}({written})`"
        );
        self.explain(SourceCheckError::Type, detail)
    }

    /// The literal of `Variant = literal`: an integer (possibly negative) or a text.
    fn discriminant_literal(&mut self, written: &'a Expression) -> Result<Discriminant> {
        match &written.value {
            Expr::Literal(LiteralKind::Integer) => {
                let text = self.span(written.span_id)?;
                Ok(Discriminant::Integer(self.integer(text, false)?))
            }
            Expr::Unary { operator, operand }
                if operator.value == UnaryOperator::Negate
                    && matches!(operand.value, Expr::Literal(LiteralKind::Integer)) =>
            {
                let text = self.span(operand.span_id)?;
                Ok(Discriminant::Integer(self.integer(text, true)?))
            }
            Expr::Literal(LiteralKind::String) => {
                let text = self.span(written.span_id)?;
                let contents = quoted(text).ok_or(SourceCheckError::Unsupported)?;
                self.charge(contents.len() + 1)?;
                Ok(Discriminant::Text(contents.to_owned()))
            }
            _ => {
                let detail = "a discriminant is a literal: an integer or a text, not an expression or a constant".to_owned();
                Err(self.explain(SourceCheckError::Type, detail))
            }
        }
    }

    /// The rules of `=` values: only on unions with no payloads, on every variant or none, one
    /// literal kind, unique.
    fn check_discriminants(
        &mut self,
        encoded: &[(usize, Discriminant, SpanId)],
        plain: usize,
        payloads: usize,
    ) -> std::result::Result<(), (SourceCheckError, Option<SpanId>)> {
        if payloads > 0 {
            let detail = "a union whose variants carry payloads cannot give its variants values".to_owned();
            return Err((self.explain(SourceCheckError::Type, detail), None));
        }
        if plain > 0 {
            let detail = "every variant has a value or none does: there is no default value".to_owned();
            return Err((self.explain(SourceCheckError::Type, detail), None));
        }
        let integers = encoded.iter().filter(|(_, value, _)| matches!(value, Discriminant::Integer(_))).count();
        if integers != 0 && integers != encoded.len() {
            let detail = "the values of one union are all integers or all text".to_owned();
            return Err((self.explain(SourceCheckError::Type, detail), None));
        }
        self.charge(encoded.len() * encoded.len()).map_err(|error| (error, None))?;
        for (index, (_, value, span)) in encoded.iter().enumerate() {
            if encoded[..index].iter().any(|(_, other, _)| other == value) {
                let detail = "two variants of one union have the same value".to_owned();
                return Err((self.explain(SourceCheckError::Duplicate, detail), Some(*span)));
            }
        }
        Ok(())
    }

    /// Declare `name` as a variant of `union`. In an `or` list (`listed`) every name is one,
    /// whatever types are in scope; a lone name that is a type is an alias, not a variant.
    fn declare_variant(&mut self, union: usize, name: &'a str, listed: bool) -> Result<()> {
        let resolves = !listed
            && self
                .module
                .type_names
                .binary_search_by(|(other, _)| other.as_str().cmp(name))
                .is_ok();
        if resolves || !name.starts_with(|first: char| first.is_ascii_uppercase()) {
            return Ok(());
        }
        let owned = self.module.union_variants.len();
        if self.own_variant(union, name).is_some() {
            return Err(SourceCheckError::Duplicate);
        }
        self.charge(name.len() + owned + 1)?;
        let layout = self.module.layouts.len();
        let origin = Origin {
            module: self.module.layouts[union].origin.module.clone(),
            declaration: self.module.schemas.len() + owned,
        };
        let qualified = format!("{}.{name}", self.module.schemas[union].name.value);
        self.module.layouts.push(Layout {
            origin,
            schema: CheckedSourceAggregate {
                name: qualified,
                parameters: Vec::new(),
                fields: Vec::new(),
                variants: Vec::new(),
                // Nothing to own, so nothing to consume.
                capability: Capability::Unrestricted,
                kind: AggregateKind::Record,
                wrapping: false,
                defaults: Vec::new(),
                bounds: Vec::new(),
                provides: Vec::new(),
                borrowing: Borrowing::None,
                numbers: Vec::new(),
                domain_claims: Vec::new(),
                discriminant: None,
            },
            constraints: Vec::new(),
            key: None,
        });
        self.module.union_variants.push((union, name, layout));
        Ok(())
    }

    /// Give each payload variant its field, now that the types it names can be resolved.
    pub(super) fn resolve_variant_payloads(&mut self) -> diagnostics::DiagnosticResult<()> {
        for index in 0..self.module.variant_payloads.len() {
            let (layout, payloads) = self.module.variant_payloads[index].clone();
            let (union, _, _) = *self
                .module
                .union_variants
                .iter()
                .find(|(_, _, other)| *other == layout)
                .ok_or(SourceCheckError::Malformed)?;
            let names: Vec<&'a str> = self.module.schemas[union]
                .parameters
                .as_ref()
                .map(|parameters| {
                    parameters
                        .value
                        .iter()
                        .map(|parameter| parameter.value.name.value.as_str())
                        .collect()
                })
                .unwrap_or_default();
            // The union's own scope, with the bounds its parameters carry (`element has Release`),
            // so a payload `Box(element, alloc)` is a type the bound allows.
            let scope = self.schema_scope(union).map_err(|error| self.failure(error, self.module.schemas[union].name.span_id))?;
            let mut fields = Vec::with_capacity(payloads.len());
            for (position, (field, written)) in payloads.iter().enumerate() {
                if !field.is_empty() && payloads[..position].iter().any(|(other, _)| other == field) {
                    let detail = format!("a variant declares the field `{field}` twice");
                    let error = self.explain(SourceCheckError::Duplicate, detail);
                    return Err(self.failure(error, written.span_id));
                }
                let ty = self
                    .resolve_type(written, &scope)
                    .map_err(|error| self.failure(error, written.span_id))?;
                fields.push(CheckedSourceField {
                    // Empty for the one payload written without a name.
                    name: (*field).to_owned(),
                    ty,
                    private: false,
                    register: None,
                    borrow: None,
                    length: None,
                });
            }
            let capability = self.module.layouts[union].schema.capability;
            self.module.variant_field_names.push((layout, payloads.iter().map(|(field, _)| *field).collect()));
            let schema = &mut self.module.layouts[layout].schema;
            schema.parameters = names.iter().map(|name| (*name).to_owned()).collect();
            schema.fields = fields;
            // A payload is owned unless the union says otherwise.
            schema.capability = capability;
        }
        Ok(())
    }

    /// The layout of the variant `union` itself declared as `name`. Uncharged.
    pub(super) fn own_variant(&self, union: usize, name: &str) -> Option<usize> {
        self.module
            .union_variants
            .iter()
            .find(|(owner, other, _)| *owner == union && *other == name)
            .map(|(_, _, layout)| *layout)
    }

    /// The variant of `variants` written `name`: a variant the union declared is named
    /// `Union.name`, a type it merely references keeps its own name. Charged.
    pub(super) fn variant_named(
        &mut self,
        variants: &[SourceType],
        name: &str,
    ) -> Result<Option<usize>> {
        self.charge(variants.len() + 1)?;
        Ok(self.variant_schema(variants, name))
    }

    /// The schema `name` means where one of `variants` is expected. The module-wide type of
    /// that name comes first, charged as it always was, when the sum holds it; only
    /// otherwise is a variant the union itself declared looked for, so a program without
    /// scoped variants costs what it cost before.
    pub(super) fn resolve_variant(
        &mut self,
        variants: &[SourceType],
        name: &'a str,
    ) -> Result<usize> {
        let global = self.lookup_symbol(name, false)?;
        if let Some(schema) = global
            && variants.iter().any(|variant| {
                matches!(variant, SourceType::Aggregate { id }
                    if self.module.instances[*id as usize].schema() == schema)
            })
        {
            return Ok(schema);
        }
        match self.variant_named(variants, name)? {
            Some(schema) => Ok(schema),
            None => {
                global.ok_or(SourceCheckError::UnknownName)
            }
        }
    }

    /// Uncharged form of [`Self::variant_named`], for recording.
    pub(super) fn variant_schema(&self, variants: &[SourceType], name: &str) -> Option<usize> {
        variants.iter().find_map(|variant| {
            let SourceType::Aggregate { id } = variant else {
                return None;
            };
            let schema = self.module.instances[*id as usize].schema();
            let written = self.module.layouts[schema].schema.name.as_str();
            let last = written.rsplit('.').next().unwrap_or(written);
            (last == name).then_some(schema)
        })
    }

    /// `Union.name` written as a type or value: the variant's type. A union validated in
    /// this module is looked up by what it declared, since its layout is not installed
    /// while schemas are still being resolved; any other, by its installed variants.
    pub(super) fn qualified_variant(&mut self, union: usize, name: &str) -> Result<SourceType> {
        if let Some(layout) = self.own_variant(union, name) {
            // A variant of a generic union ranges over the union's parameters; which instance is
            // meant is read from the expected type where it is built (`constructor_type`).
            let arguments = (0..self.module.layouts[layout].schema.parameters.len())
                .map(|index| SourceType::Generic { index: index as u32 })
                .collect();
            return self.instance(layout, arguments);
        }
        let variants = self.module.layouts[union].schema.variants.clone();
        let schema = self
            .variant_named(&variants, name)?
            .ok_or(SourceCheckError::UnknownName)?;
        self.instance(schema, Vec::new())
    }

    pub(super) fn written_variants(
        &mut self,
        schema: usize,
        scope: &[(&'a str, SourceType)],
    ) -> Result<Vec<SourceType>> {
        let TypeBody::Is(root) = &self.module.schemas[schema].body.value else {
            return Ok(Vec::new());
        };
        self.child(self.module.schemas[schema].body.span_id, root.span_id)?;
        let mut pending = vec![root];
        let mut variants = Vec::new();
        while let Some(expression) = pending.pop() {
            self.charge(1)?;
            match &expression.value {
                Expr::Binary {
                    left,
                    operator,
                    right,
                } if operator.value == BinaryOperator::Or => {
                    self.child(expression.span_id, left.span_id)?;
                    self.child(expression.span_id, right.span_id)?;
                    self.child(expression.span_id, operator.span_id)?;
                    self.token(operator.span_id, "or")?;
                    self.charge(2)?;
                    pending.push(right);
                    pending.push(left);
                }
                Expr::Name(name) if self.own_variant(schema, name).is_some() => {
                    let layout = self.own_variant(schema, name).expect("checked above");
                    variants.push(self.instance(layout, Vec::new())?);
                }
                // `Debug = 'debug'`: the variant; its value was recorded when it was declared.
                Expr::Binary { left, operator, right }
                    if operator.value == BinaryOperator::Equal
                        && matches!(&left.value, Expr::Name(name) if self.own_variant(schema, name).is_some()) =>
                {
                    let Expr::Name(name) = &left.value else {
                        unreachable!("matched above")
                    };
                    self.child(expression.span_id, left.span_id)?;
                    self.child(expression.span_id, operator.span_id)?;
                    self.child(expression.span_id, right.span_id)?;
                    self.token(operator.span_id, "=")?;
                    let layout = self.own_variant(schema, name).expect("checked above");
                    variants.push(self.instance(layout, Vec::new())?);
                }
                // A variant with a payload ranges over the union's own parameters.
                Expr::Call { callee, arguments }
                    if matches!(&callee.value, Expr::Name(name) if self.own_variant(schema, name).is_some()) =>
                {
                    let Expr::Name(name) = &callee.value else {
                        unreachable!("matched above")
                    };
                    let layout = self.own_variant(schema, name).expect("checked above");
                    self.child(expression.span_id, callee.span_id)?;
                    self.child(expression.span_id, arguments.span_id)?;
                    let generic = scope
                        .iter()
                        .map(|(_, ty)| *ty)
                        .collect::<Vec<_>>();
                    variants.push(self.instance(layout, generic)?);
                }
                _ => variants.push(self.resolve_type(expression, scope)?),
            }
        }
        if variants.is_empty() {
            return Err(SourceCheckError::Type);
        }
        Ok(variants)
    }

    pub(super) fn validate_sums(&mut self) -> Result<()> {
        for schema in 0..self.module.layouts.len() {
            self.charge(self.module.layouts[schema].schema.variants.len())?;
            let mut pending = self.module.layouts[schema].schema.variants.clone();
            let mut heads = Vec::new();
            let mut variants = Vec::new();
            while let Some(variant) = pending.pop() {
                self.charge(heads.len() + 1)?;
                match variant {
                    SourceType::Sum { id } => pending.extend(self.sum_variants(id)?),
                    SourceType::Aggregate { id } => {
                        let head = self.module.instances[id as usize].schema();
                        if heads.contains(&head) {
                            return Err(SourceCheckError::Duplicate);
                        }
                        heads.push(head);
                        variants.push(variant);
                    }
                    _ => return Err(SourceCheckError::Type),
                }
            }
            variants.reverse();
            self.module.layouts[schema].schema.variants = variants;
        }
        Ok(())
    }

    pub(super) fn sum_variants(&mut self, id: u32) -> Result<Vec<SourceType>> {
        let instance = &self.module.instances[id as usize];
        let schema = instance.schema();
        self.charge(instance.arguments.len() + self.module.layouts[schema].schema.variants.len())?;
        let arguments = self.module.instances[id as usize].arguments.clone();
        let mut variants = Vec::new();
        for index in 0..self.module.layouts[schema].schema.variants.len() {
            let template = self.module.layouts[schema].schema.variants[index];
            variants.push(self.substitute(template, &arguments)?);
        }
        Ok(variants)
    }

    pub(super) fn choose(&mut self, count: usize) -> Result<usize> {
        self.choose_kind(count, false)
    }

    /// A choice among the outcomes of a callee, which multiplies worlds without adding a path
    /// through this function's own code.
    pub(super) fn choose_outcome(&mut self, count: usize) -> Result<usize> {
        self.choose_kind(count, true)
    }

    fn choose_kind(&mut self, count: usize, outcome: bool) -> Result<usize> {
        self.charge_flow()?;
        self.charge(1)?;
        if count == 0 {
            return Err(SourceCheckError::Type);
        }
        let index = self.replay.choice_cursor;
        self.replay.choice_cursor += 1;
        if index == self.replay.choices.len() {
            self.replay.choices.push((0, count));
            self.replay.outcome_flags.resize(index, false);
            self.replay.outcome_flags.push(outcome);
        }
        // The call whose outcome differs in a world past the cap is where the refusal points.
        if outcome && index == self.world_advanced && self.world_count > WORLDS_MAX {
            let name = self.world_function.unwrap_or("this function");
            let detail = format!(
                "function `{name}` makes more than {WORLDS_MAX} outcome combinations: split it or make the unused call's result explicit"
            );
            return Err(self.explain(SourceCheckError::WorldLimit, detail));
        }
        let (choice, previous_count) = self.replay.choices[index];
        assert_eq!(previous_count, count);
        assert!(choice < count);
        Ok(choice)
    }

    pub(super) fn function_worlds(
        &mut self,
        binding: &'a Binding,
        parameters_max: u32,
    ) -> Result<calls::Function> {
        self.world_function = Some(binding.name.value.as_str());
        self.world_count = 0;
        self.world_advanced = usize::MAX;
        self.replay.begin_function();
        self.body = None;
        self.conversions_all_worlds = 0;
        self.conversion_events.clear();
        self.unmatched_recording = None;
        self.recorded_decline = None;
        self.recorded_folds.clear();
        self.query_values.clear();
        let mut function: Option<calls::Function> = None;
        loop {
            self.world_count += 1;
            self.charge_flow()?;
            self.replay.begin_world();
            let (parameters, result) = self.function(binding, parameters_max)?;
            let outcome = self.publish_function(binding, parameters, result)?;
            if let Some(function) = &mut function {
                // Static validation may explore hypothetical nested sum fields.
                // Those choices can repeat a real outcome, but cannot add one.
                let summary = &outcome.summaries[0];
                let mut duplicate = false;
                for previous in &mut function.summaries {
                    if summary.same(previous, &mut self.remaining)? {
                        previous.absorb_ranges(summary);
                        duplicate = true;
                        break;
                    }
                }
                if !duplicate {
                    function.summaries.extend(outcome.summaries);
                }
                Self::merge_effects(&mut function.effects, outcome.effects);
            } else {
                function = Some(outcome);
            }
            self.replay.choices.truncate(self.replay.choice_cursor);
            self.replay.outcome_flags.truncate(self.replay.choice_cursor);
            let mut advanced = false;
            while !self.replay.choices.is_empty() {
                self.charge_flow()?;
                let (choice, count) = self.replay.choices.last_mut().unwrap();
                if *choice + 1 < *count {
                    *choice += 1;
                    self.world_advanced = self.replay.choices.len() - 1;
                    advanced = true;
                    break;
                }
                self.replay.choices.pop();
                self.replay.outcome_flags.pop();
            }
            if !advanced {
                break;
            }
        }
        self.replay.finish_function();
        self.world_function = None;
        // A conversion in a branch a world does not take is counted by the world that does: the
        // recording holds every branch, so it matches the worlds together.
        if std::env::var_os("DBGCONV").is_some() { eprintln!("DBGWORLDS {} all={} events={:?} unmatched={:?}", binding.name.value, self.conversions_all_worlds, self.conversion_events, self.unmatched_recording.as_ref().map(|u| u.1)); }
        if self.body.is_none()
            && let Some((body, recorded)) = self.unmatched_recording.take()
            && recorded == self.conversions_all_worlds
        {
            self.body = Some(body);
            self.recorded_decline = None;
        }
        let mut function = function.expect("at least one verified world");
        function.body = self.body.take().map(std::sync::Arc::new);
        function.decline = self.recorded_decline.take();
        function.folds = std::mem::take(&mut self.recorded_folds);
        Ok(function)
    }

    /// The bounds of a linear domain type, or `None` for any other type.
    pub(super) fn domain_bounds_of(&self, ty: SourceType) -> Option<(I256, I256)> {
        let SourceType::Aggregate { id } = ty else {
            return None;
        };
        let layout = &self.module.layouts[self.module.instances[id as usize].schema()];
        match layout.schema.fields.first().map(|field| field.ty) {
            Some(SourceType::Integer { lower, upper })
                if layout.schema.kind == AggregateKind::Domain =>
            {
                Some((lower, upper))
            }
            _ => None,
        }
    }

    fn domain_wraps(&self, ty: SourceType) -> bool {
        let SourceType::Aggregate { id } = ty else {
            return false;
        };
        let schema = self.module.instances[id as usize].schema();
        self.module.layouts[schema].schema.wrapping
    }

    /// The integer a value becomes inside `lower..=upper` when reduced modulo its width:
    /// the exact interval when the reduction keeps it contiguous, else the whole range.
    fn wrapped_into(&mut self, value: Value, lower: I256, upper: I256) -> Result<Value> {
        let spans_all = upper
            .checked_sub(lower)
            .zip(value.upper.checked_sub(value.lower))
            .is_none_or(|(width, spread)| spread >= width);
        let (low, high) = if spans_all {
            (lower, upper)
        } else {
            match (
                super::eval::wrap_into(value.lower, lower, upper),
                super::eval::wrap_into(value.upper, lower, upper),
            ) {
                (Some(a), Some(b)) if a <= b => (a, b),
                _ => (lower, upper),
            }
        };
        self.fresh_value(SourceType::Integer {
            lower: low,
            upper: high,
        })
    }

    /// Between an integer and a linear domain, the only conversions: a domain's own
    /// methods read their receiver as the integer it holds, and an integer whose
    /// interval is proved inside a domain becomes that domain's value. Nothing converts
    /// a domain back to an integer elsewhere.
    fn convert_domain(&mut self, mut value: Value, expected: SourceType) -> Result<Value> {
        // An `#unrestricted` number-like domain read as a number: its number, anywhere.
        // (Inside the domain's own methods the receiver's conversion below is the one recorded.)
        if matches!(expected, SourceType::Integer { .. })
            && matches!(value.ty, SourceType::Aggregate { .. })
            && self.is_open_domain(value.ty)
            && !(self.receiver_is_domain() && self.proof.receiver == Some(value.ty))
        {
            value = self.open_number(value)?;
        }
        if self.receiver_is_domain()
            && self.proof.receiver == Some(value.ty)
            && value.ty != expected
            && let Some((lower, upper)) = self.domain_bounds_of(value.ty)
            && matches!(
                expected,
                SourceType::Integer { .. } | SourceType::Aggregate { .. }
            )
        {
            self.materialize_children(value)?;
            self.release(value)?;
            let owner = value.owner.ok_or(SourceCheckError::Type)?;
            let mut inner = self.proof.owners[owner].children[0];
            inner.lower = lower;
            inner.upper = upper;
            self.count_conversion();
            value = inner;
        }
        if let (Some((lower, upper)), SourceType::Integer { .. }) =
            (self.domain_bounds_of(expected), value.ty)
        {
            if value.lower < lower || value.upper > upper {
                if self.domain_wraps(expected) {
                    // A wrapping domain takes the congruent value inside its range.
                    value = self.wrapped_into(value, lower, upper)?;
                } else {
                    // Overlapping but unproved, versus disjoint and plainly wrong.
                    return Err(if value.lower <= upper && value.upper >= lower {
                        SourceCheckError::Unproved
                    } else {
                        SourceCheckError::Type
                    });
                }
            }
            if !self.domain_wraps(expected) {
                value = self.prove_claims(value, expected)?;
            }
            let converted = self.fresh_value(expected)?;
            let owner = converted.owner.ok_or(SourceCheckError::Type)?;
            self.proof.owners[owner].children = vec![value];
            self.proof.owners[owner].materialized = true;
            self.count_conversion();
            return Ok(converted);
        }
        Ok(value)
    }

    pub(super) fn view_as(&mut self, mut value: Value, expected: SourceType) -> Result<Value> {
        if value.ty == SourceType::Text
            && let Some(view) = self.text_as_bytes(value, expected)?
        {
            return Ok(view);
        }
        value = self.scalar_facts(value)?;
        value = self.convert_domain(value, expected)?;
        self.compatible(expected, value.ty)?;
        if let SourceType::Integer { lower, upper } = expected {
            value.integer_domain = Some((lower, upper));
        }
        value.sum = match expected {
            SourceType::Sum { id } => Some(id),
            _ => None,
        };
        // A known variant of a union whose variants all carry nothing takes the union's own
        // capability: the variant is free by nature, the union is linear unless `#unrestricted`.
        if let SourceType::Sum { id } = expected
            && matches!(value.ty, SourceType::Aggregate { .. })
            && let Some(owner) = value.owner
            && self.proof.owners[owner].unrestricted
            && self.capability_of(expected) == Capability::Linear
        {
            // Read uncharged, as `bind_pattern` reads the declared variants.
            let sum = self.module.instances[id as usize].schema();
            let carries = self.module.layouts[sum].schema.variants.iter().any(|variant| {
                matches!(variant, SourceType::Aggregate { id }
                    if !self.module.layouts[self.module.instances[*id as usize].schema()].schema.fields.is_empty())
            });
            if !carries {
                self.proof.owners[owner].unrestricted = false;
            }
        }
        value.structural = match expected {
            SourceType::Array { .. } | SourceType::Sequence { .. } => Some(expected),
            _ => None,
        };
        Ok(value)
    }

    pub(super) fn bind_pattern(
        &mut self,
        pattern: &'a VariantPattern,
        span: SpanId,
        value: Value,
    ) -> Result<()> {
        let value = self.inspect_value(value)?;
        self.apply_claims(value)?;
        // A value checked on its own, as an unselected arm is, carries no sum: its own type
        // is then the only variant its pattern can name.
        // Only names are read, so the sum's declared variants serve, unsubstituted.
        let variants = match value.sum {
            Some(id) => {
                let sum = self.module.instances[id as usize].schema();
                self.module.layouts[sum].schema.variants.clone()
            }
            None => vec![value.ty],
        };
        let schema = self.pattern_schema(pattern, span, &variants)?;
        let SourceType::Aggregate { id } = value.ty else {
            return Err(SourceCheckError::Type);
        };
        if self.module.instances[id as usize].schema() != schema {
            return Err(SourceCheckError::Type);
        }
        // A bare member name on a member with fields narrows the subject, which stays itself.
        if self.narrows(pattern, schema) {
            return Ok(());
        }
        self.materialize_children(value)?;
        self.release(value)?;
        if let Some(fields) = &pattern.fields {
            let owner = value.owner.ok_or(SourceCheckError::Type)?;
            if fields.binders.value.len() != self.proof.owners[owner].children.len() {
                return Err(SourceCheckError::Type);
            }
            let schema = self.module.instances[id as usize].schema();
            let order = self.pattern_order(schema, fields)?;
            for (index, binder) in fields.binders.value.iter().enumerate() {
                let name = self.name(binder)?;
                let child = self.proof.owners[owner].children[order[index]];
                if let Some(Some(inner)) = fields.nested.get(index) {
                    self.bind_nested(inner, child)?;
                    continue;
                }
                if name == "_" {
                    // A field that is dropped must be droppable: a linear value is never
                    // silently discarded.
                    if self.holds_linear(child) {
                        return Err(SourceCheckError::Unconsumed);
                    }
                    continue;
                }
                self.insert(name, child)?;
            }
        }
        Ok(())
    }

    /// For each binder of a pattern, the index of the field it takes. Positional when no binder
    /// names a field; otherwise a binder names its field (`binder: field`, or a bare `field`)
    /// and every field is named exactly once.
    pub(super) fn pattern_order(
        &self,
        schema: usize,
        fields: &PatternFields,
    ) -> Result<Vec<usize>> {
        let count = fields.binders.value.len();
        if fields.selectors.iter().all(Option::is_none) {
            return Ok((0..count).collect());
        }
        let declared = &self.module.layouts[schema].schema.fields;
        if count != declared.len() {
            return Err(SourceCheckError::Type);
        }
        let mut order = Vec::with_capacity(count);
        for (binder, selector) in fields.binders.value.iter().zip(&fields.selectors) {
            let name = selector.as_ref().unwrap_or(binder).value.as_str();
            let index = declared
                .iter()
                .position(|field| field.name == name)
                .ok_or(SourceCheckError::Type)?;
            if order.contains(&index) {
                return Err(SourceCheckError::Duplicate);
            }
            order.push(index);
        }
        Ok(order)
    }

    /// A pattern that names a whole union among the subject's members (`AllocError(error)`
    /// where the subject's union includes `AllocError`): the union, and the variants of the
    /// subject it covers. `None` when the name is one of the subject's own variants or no union.
    pub(super) fn nested_union(
        &mut self,
        pattern: &'a VariantPattern,
        variants: &[SourceType],
    ) -> Result<Option<(SourceType, Vec<usize>)>> {
        let name = pattern.constructor.value.as_str();
        if self.variant_named(variants, name)?.is_some() {
            return Ok(None);
        }
        let Some(schema) = self.lookup_symbol(name, false)? else {
            return Ok(None);
        };
        if schema >= self.module.layouts.len()
            || self.module.layouts[schema].schema.variants.is_empty()
            || !self.module.layouts[schema].schema.parameters.is_empty()
        {
            return Ok(None);
        }
        let ty = self.instance(schema, Vec::new())?;
        let SourceType::Sum { id } = ty else {
            return Ok(None);
        };
        let mut covered = Vec::new();
        for inner in self.sum_variants(id)? {
            let SourceType::Aggregate { id } = inner else {
                return Err(SourceCheckError::Type);
            };
            covered.push(self.module.instances[id as usize].schema());
        }
        Ok(Some((ty, covered)))
    }

    pub(super) fn pattern_schema(
        &mut self,
        pattern: &'a VariantPattern,
        span: SpanId,
        variants: &[SourceType],
    ) -> Result<usize> {
        self.child(span, pattern.constructor.span_id)?;
        let name = self.name(&pattern.constructor)?;
        self.check_constructor_scope(name)?;
        // Among the subject's own variants, so `Red` means the variant of its union.
        let schema = self.resolve_variant(variants, name)?;
        if !self.module.layouts[schema].schema.variants.is_empty() {
            return Err(SourceCheckError::Type);
        }
        // Binding fields takes the record apart, which needs access to its representation.
        // A test that binds nothing (`attempt is Vacant`) only narrows the subject to the
        // member, so another module's private record may be tested this way.
        if pattern.fields.is_some() {
            self.schema_access(schema)?;
        }
        let binders = if let Some(fields) = &pattern.fields {
            self.child(span, fields.open)?;
            self.child(span, fields.close)?;
            self.child(span, fields.binders.span_id)?;
            self.token(fields.open, "(")?;
            self.token(fields.close, ")")?;
            self.charge(fields.binders.value.len())?;
            let mut names = Vec::new();
            for binder in &fields.binders.value {
                self.child(fields.binders.span_id, binder.span_id)?;
                if let Some(selector) = &fields.selectors[names.len()] {
                    self.child(fields.binders.span_id, selector.span_id)?;
                    self.name(selector)?;
                }
                let name = self.name(binder)?;
                self.charge_product(names.len(), name.len() + 1)?;
                // `_` discards its field, so any number may stand in one pattern.
                if name != "_" && names.contains(&name) {
                    return Err(SourceCheckError::Duplicate);
                }
                names.push(name);
            }
            fields.binders.value.len()
        } else {
            0
        };
        // `Vacant` with no parentheses binds nothing: it narrows the subject to the member,
        // whatever fields the member has. With parentheses every field is named.
        if pattern.fields.is_some() && binders != self.module.layouts[schema].schema.fields.len() {
            return Err(SourceCheckError::Type);
        }
        Ok(schema)
    }

    /// The nested patterns of an arm's pattern, outermost first, as a path: at each level the
    /// field of the enclosing variant the pattern takes, the variant of that field's union it
    /// names, and how many variants that union has. Empty for a flat pattern. One nested
    /// position per level.
    fn nested_path(
        &mut self,
        pattern: &'a VariantPattern,
        schema: usize,
        variants: &[SourceType],
    ) -> Result<Vec<(usize, usize, usize)>> {
        let mut path = Vec::new();
        let Some(SourceType::Aggregate { id }) = variants
            .iter()
            .copied()
            .find(|variant| matches!(variant, SourceType::Aggregate { id } if self.module.instances[*id as usize].schema() == schema))
        else {
            return Err(SourceCheckError::Type);
        };
        let (mut pattern, mut schema, mut id) = (pattern, schema, id);
        for _ in 0..=PATTERN_NESTING_MAX {
            let Some(fields) = &pattern.fields else { return Ok(path) };
            let mut found = fields.nested.iter().enumerate().filter_map(|(at, inner)| inner.as_deref().map(|inner| (at, inner)));
            let Some((position, inner)) = found.next() else { return Ok(path) };
            if found.next().is_some() {
                return Err(self.explain(
                    SourceCheckError::Unsupported,
                    "a pattern nests another pattern in one place only".to_owned(),
                ));
            }
            let order = self.pattern_order(schema, fields)?;
            let types = self.instance_fields(id)?;
            let Some(SourceType::Sum { id: payload }) = types.get(order[position]).copied() else {
                let detail = format!("`{}` holds no union to match a pattern inside", pattern.constructor.value);
                return Err(self.explain(SourceCheckError::Type, detail));
            };
            let inner_variants = self.sum_variants(payload)?;
            let inner_schema = self.pattern_schema(&inner.value, inner.span_id, &inner_variants)?;
            let Some(SourceType::Aggregate { id: inner_id }) = inner_variants.iter().copied().find(|variant| {
                matches!(variant, SourceType::Aggregate { id } if self.module.instances[*id as usize].schema() == inner_schema)
            }) else {
                return Err(SourceCheckError::Type);
            };
            self.charge(1)?;
            path.push((order[position], inner_schema, inner_variants.len()));
            (pattern, schema, id) = (&inner.value, inner_schema, inner_id);
        }
        Err(SourceCheckError::Budget)
    }

    /// Whether the nested arms of one outer variant (their paths) cover it: at each level every
    /// variant of the union is named by an arm that stops there or is covered one level down.
    /// The arms that branch at one level take the same field.
    fn paths_cover(&mut self, paths: &[Vec<(usize, usize, usize)>], depth: usize) -> Result<bool> {
        self.charge(paths.len() + 1)?;
        let Some(first) = paths.iter().find(|path| path.len() > depth) else {
            return Ok(false);
        };
        let (field, _, count) = first[depth];
        if paths.iter().any(|path| path.len() > depth && path[depth].0 != field) {
            return Err(self.explain(
                SourceCheckError::Unsupported,
                "the arms of one variant nest their patterns in the same place".to_owned(),
            ));
        }
        let mut covered = Vec::new();
        let mut heads: Vec<usize> = paths.iter().filter(|path| path.len() > depth).map(|path| path[depth].1).collect();
        heads.sort_unstable();
        heads.dedup();
        for head in heads {
            let below: Vec<Vec<(usize, usize, usize)>> =
                paths.iter().filter(|path| path.len() > depth && path[depth].1 == head).cloned().collect();
            if below.iter().any(|path| path.len() == depth + 1) || self.paths_cover(&below, depth + 1)? {
                covered.push(head);
            }
        }
        Ok(covered.len() == count)
    }

    /// Whether this world's payloads are the variants `path` names, level by level from the
    /// (inspected) `subject`. Each level is chosen once, where it is first read.
    fn active_path(&mut self, subject: Value, path: &[(usize, usize, usize)]) -> Result<bool> {
        let mut value = subject;
        for &(field, schema, _) in path {
            self.materialize_children(value)?;
            let owner = value.owner.ok_or(SourceCheckError::Type)?;
            let child = *self.proof.owners[owner].children.get(field).ok_or(SourceCheckError::Type)?;
            let child = self.inspect_value(child)?;
            let SourceType::Aggregate { id } = child.ty else {
                return Err(SourceCheckError::Type);
            };
            if self.module.instances[id as usize].schema() != schema {
                return Ok(false);
            }
            value = child;
        }
        Ok(true)
    }

    /// Bind the pattern written inside another: the payload is taken apart by the variant the
    /// inner pattern names. A payload no world has chosen yet (an arm checked on its own) is
    /// viewed as that variant.
    fn bind_nested(&mut self, inner: &'a Spanned<VariantPattern>, child: Value) -> Result<()> {
        let mut child = self.owner_view(child)?;
        if let SourceType::Sum { id } = child.ty {
            let variants = self.sum_variants(id)?;
            let schema = self.pattern_schema(&inner.value, inner.span_id, &variants)?;
            let position = variants
                .iter()
                .position(|variant| matches!(variant, SourceType::Aggregate { id } if self.module.instances[*id as usize].schema() == schema))
                .ok_or(SourceCheckError::Type)?;
            let owner = child.owner.ok_or(SourceCheckError::Type)?;
            self.proof.owners[owner].active = Some(variants[position]);
            child.ty = variants[position];
            child.sum = child.sum.or(Some(id));
        }
        self.bind_pattern(&inner.value, inner.span_id, child)
    }

    pub(super) fn match_arm(
        &mut self,
        expression: &'a MatchExpression,
        subject: Value,
    ) -> Result<Vec<&'a MatchArm>> {
        self.token(expression.when_keyword, "when")?;
        self.token(expression.is_keyword, "is")?;
        let variants = if let Some(id) = subject.sum {
            self.sum_variants(id)?
        } else {
            vec![subject.ty]
        };
        let mut heads = Vec::new();
        for variant in variants.iter().copied() {
            let SourceType::Aggregate { id } = variant else {
                return Err(SourceCheckError::Type);
            };
            heads.push(self.module.instances[id as usize].schema());
        }
        let SourceType::Aggregate { id } = subject.ty else {
            return Err(SourceCheckError::Type);
        };
        let active = self.module.instances[id as usize].schema();
        let mut seen = Vec::new();
        // Per outer variant with nested arms: where they nest and the inner variants named.
        let mut nested: Vec<(usize, Vec<Vec<(usize, usize, usize)>>)> = Vec::new();
        // The arms for the active member in order: any with a guard, then the one without.
        let mut selected = Vec::new();
        // The member whose guarded arm still awaits its unguarded fallback, as the next arm.
        let mut owed: Option<(usize, Vec<(usize, usize, usize)>)> = None;
        let mut separated: Vec<(usize, Vec<(usize, usize, usize)>)> = Vec::new();
        for arm in &expression.arms.value {
            self.child(expression.arms.span_id, arm.span_id)?;
            self.child(arm.span_id, arm.pattern.span_id)?;
            // A guard does not cover its member: only an arm without one counts below.
            let guarded = arm.guard.is_some();
            if let Some(guard) = &arm.guard {
                self.child(arm.span_id, guard.and_keyword)?;
                self.keyword(guard.and_keyword, "and")?;
                self.child(arm.span_id, guard.condition.span_id)?;
            }
            self.child(arm.span_id, arm.then_keyword)?;
            self.child(arm.span_id, arm.body.span_id)?;
            self.token(arm.then_keyword, "then")?;
            let MatchPattern::Variant(pattern) = &arm.pattern.value else {
                return Err(SourceCheckError::Type);
            };
            if self.nested_union(pattern, &variants)?.is_some() {
                // The arm that names a whole inner union (`AllocError(error)`) is withdrawn: a
                // payload union is matched through its wrapper (`Flaw(error)`).
                let name = pattern.constructor.value.clone();
                let detail = format!(
                    "`{name}(...)` names an inner union as a member; that arm is withdrawn: match it through the variant that wraps it (`Flaw(error)`)"
                );
                return Err(self.explain(SourceCheckError::Type, detail));
            }
            let schema = self.pattern_schema(pattern, arm.pattern.span_id, &variants)?;
            self.charge(heads.len() + seen.len() + 1)?;
            if !heads.contains(&schema) {
                return Err(SourceCheckError::Type);
            }
            if seen.contains(&schema) {
                return Err(SourceCheckError::Duplicate);
            }
            // `Flaw(OutOfMemory)`: the arm names a variant of the payload union too. The outer
            // variant is covered once every inner variant has an arm.
            let path = self.nested_path(pattern, schema, &variants)?;
            // A guarded arm sits directly before the unguarded fallback of its member (the same
            // variant and the same inner variants): anything else leaves it unreachable or
            // separated from its fallback, flat or nested alike.
            // One with no fallback at all is reported as an uncovered member below instead.
            if let Some((owed_schema, owed_path)) = owed.take()
                && (owed_schema != schema || owed_path != path)
            {
                separated.push((owed_schema, owed_path));
            }
            if guarded {
                let closed = nested
                    .iter()
                    .any(|(outer, paths)| *outer == schema && paths.iter().any(|other| *other == path));
                if closed {
                    let detail = "a guarded arm must sit directly before the unguarded arm of its variant, not after it".to_owned();
                    return Err(self.explain(SourceCheckError::Duplicate, detail));
                }
                owed = Some((schema, path.clone()));
            } else {
                owed = None;
            }
            if !path.is_empty() {
                let entry = match nested.iter().position(|(outer, _)| *outer == schema) {
                    Some(entry) => entry,
                    None => {
                        nested.push((schema, Vec::new()));
                        nested.len() - 1
                    }
                };
                // A guarded arm does not cover its inner variant: an arm without one must follow.
                if !guarded {
                    // The same path twice, or one inside another's (`Done(Full(_))` beside
                    // `Done(Full(Inner(v)))`), names some payloads twice.
                    let overlaps = nested[entry].1.iter().any(|other: &Vec<(usize, usize, usize)>| {
                        let shared = other.len().min(path.len());
                        other[..shared] == path[..shared]
                    });
                    if overlaps {
                        return Err(SourceCheckError::Duplicate);
                    }
                    nested[entry].1.push(path.clone());
                }
                // Checked as the arms come, so a different field is refused at the arm.
                let paths = nested[entry].1.clone();
                if !paths.is_empty() {
                    self.paths_cover(&paths, 0)?;
                }
                if schema == active && self.active_path(subject, &path)? {
                    selected.push(&arm.value);
                }
                continue;
            }
            if nested.iter().any(|(outer, _)| *outer == schema) {
                return Err(SourceCheckError::Duplicate);
            }
            if !guarded {
                seen.push(schema);
            }
            if schema == active {
                selected.push(&arm.value);
            }
        }
        for (schema, path) in separated {
            let fallback = if path.is_empty() {
                seen.contains(&schema)
            } else {
                nested.iter().any(|(outer, paths)| outer == &schema && paths.contains(&path))
            };
            if fallback {
                let detail = "a guarded arm must sit directly before the unguarded arm of its variant".to_owned();
                return Err(self.explain(SourceCheckError::Duplicate, detail));
            }
        }
        for (schema, paths) in nested.clone() {
            if self.paths_cover(&paths, 0)? {
                seen.push(schema);
            }
        }
        if seen.len() != heads.len() || selected.is_empty() {
            return Err(SourceCheckError::Type);
        }
        Ok(selected)
    }

    pub(super) fn match_tasks(
        &mut self,
        case: &'a MatchExpression,
        subject: Value,
        expected: Option<SourceType>,
        pending: &mut Vec<aggregates::EvalTask<'a>>,
    ) -> Result<()> {
        use aggregates::EvalTask;
        if (matches!(subject.ty, SourceType::Integer { .. }) || subject.ty == SourceType::Text)
            && case.arms.value.iter().any(|arm| arm.guard.is_some())
        {
            return Err(self.explain(
                SourceCheckError::Unsupported,
                "a guard on an arm of a case over numbers or text is not supported".to_owned(),
            ));
        }
        if matches!(subject.ty, SourceType::Integer { .. }) {
            return self.integer_match_tasks(case, subject, expected, pending);
        }
        if subject.ty == SourceType::Text {
            return self.text_match_tasks(case, expected, pending);
        }
        // Whether this `when` is where the subject's member was chosen. If it was, every
        // other member has a world of its own in which its arm is the selected one and is
        // checked completely, so checking those arms again here, inside this world, only
        // multiplies the worlds of every arm together. If the member was already fixed
        // (a constructed value, or one an earlier `when` narrowed), no world will ever
        // select the other arms and they need the detached check. `inspect_value` is the
        // only place a member is fixed, and it fixes one by choosing.
        let cursor = self.replay.choice_cursor;
        let subject = self.inspect_value(subject)?;
        let chosen_here = self.replay.choice_cursor > cursor;
        let selected = self.match_arm(case, subject)?;
        match selected[..] {
            [only] if only.guard.is_none() => {
                pending.push(EvalTask::Arm(only, subject, expected));
            }
            _ => pending.push(EvalTask::Candidates(selected.clone(), subject, expected)),
        }
        let variants = match subject.sum {
            Some(id) => self.sum_variants(id)?,
            None => vec![subject.ty],
        };
        for arm in &case.arms.value {
            if selected.iter().any(|chosen| std::ptr::eq(&arm.value, *chosen)) {
                continue;
            }
            let MatchPattern::Variant(pattern) = &arm.pattern.value else {
                return Err(SourceCheckError::Type);
            };
            if chosen_here {
                continue;
            }
            if let Some((_, covered)) = self.nested_union(pattern, &variants)? {
                // Any variant the arm covers stands in for the union value.
                let first = *covered.first().ok_or(SourceCheckError::Type)?;
                let ty = variants
                    .iter()
                    .find(|ty| {
                        matches!(ty, SourceType::Aggregate { id }
                        if self.module.instances[*id as usize].schema() == first)
                    })
                    .copied()
                    .ok_or(SourceCheckError::Type)?;
                pending.push(EvalTask::ValidateArm(&arm.value, ty, expected, subject.owner));
                continue;
            }
            let schema = self.resolve_variant(&variants, &pattern.constructor.value)?;
            self.charge(variants.len() + 1)?;
            let ty = variants
                .iter()
                .find(|ty| {
                    matches!(ty, SourceType::Aggregate { id }
                    if self.module.instances[*id as usize].schema() == schema)
                })
                .copied()
                .ok_or(SourceCheckError::Type)?;
            pending.push(EvalTask::ValidateArm(&arm.value, ty, expected, subject.owner));
        }
        Ok(())
    }

    /// A case over compile-time text. Which text the subject is is not known to the checker (the
    /// recorder knows it and picks the arm), so every arm is a possible world and each is checked.
    /// Text is an open set, so the last arm is `else`.
    fn text_match_tasks(
        &mut self,
        case: &'a MatchExpression,
        expected: Option<SourceType>,
        pending: &mut Vec<aggregates::EvalTask<'a>>,
    ) -> Result<()> {
        use aggregates::EvalTask;
        self.token(case.when_keyword, "when")?;
        self.token(case.is_keyword, "is")?;
        let arms = &case.arms.value;
        let mut seen: Vec<&str> = Vec::new();
        for (position, arm) in arms.iter().enumerate() {
            self.child(case.arms.span_id, arm.span_id)?;
            self.child(arm.span_id, arm.pattern.span_id)?;
            self.child(arm.span_id, arm.body.span_id)?;
            self.charge(seen.len() + 1)?;
            match &arm.pattern.value {
                MatchPattern::Text(literal) => {
                    self.child(arm.span_id, arm.then_keyword)?;
                    self.token(arm.then_keyword, "then")?;
                    let written = self.span(literal.span_id)?;
                    let text = quoted(written).ok_or(SourceCheckError::Unsupported)?;
                    if seen.contains(&text) {
                        return Err(SourceCheckError::Duplicate);
                    }
                    seen.push(text);
                }
                MatchPattern::Else { else_keyword } => {
                    self.child(arm.span_id, *else_keyword)?;
                    self.token(*else_keyword, "else")?;
                    // The default is the last arm and the only one.
                    if position + 1 != arms.len() {
                        return Err(SourceCheckError::Type);
                    }
                }
                _ => return Err(SourceCheckError::Type),
            }
        }
        if !matches!(
            arms.last().map(|arm| &arm.pattern.value),
            Some(MatchPattern::Else { .. })
        ) {
            return Err(SourceCheckError::Type);
        }
        self.charge(arms.len())?;
        // `when #target.triple is ...`: the target is known, so one arm runs. The others are still
        // checked, as code that never runs on this target (their conversions are not the body's).
        if let Expr::Member { base, name } = &case.subject.value
            && matches!(&base.value, Expr::Query { .. })
            && name.value == "triple"
        {
            let triple = self.target.triple.to_owned();
            let mut taken = None;
            for (position, arm) in arms.iter().enumerate() {
                let matches = match &arm.pattern.value {
                    MatchPattern::Text(literal) => {
                        quoted(self.span(literal.span_id)?).is_some_and(|text| text == triple)
                    }
                    _ => false,
                };
                if matches {
                    taken = Some(position);
                    break;
                }
            }
            let selected = taken.unwrap_or(arms.len() - 1);
            for (position, arm) in arms.iter().enumerate() {
                if position != selected {
                    self.check_arm_unreached(
                        arm.body.span_id,
                        None,
                        |this| this.evaluate(&arm.body, expected),
                    )?;
                }
            }
            pending.push(EvalTask::Expression(&arms[selected].body, expected, false));
            return Ok(());
        }
        let selected = self.choose(arms.len())?;
        pending.push(EvalTask::Expression(&arms[selected].body, expected, false));
        Ok(())
    }

    pub(super) fn bare_value(
        &mut self,
        expression: &'a Expression,
        expected: Option<SourceType>,
    ) -> Result<Value> {
        if let Some(function) = self.call_target(expression)? {
            let interface = self.function_interface(function)?;
            if !interface.parameters.is_empty() {
                return Err(SourceCheckError::Type);
            }
            let value = self.apply_outcomes(&interface.summaries, &[], &[])?;
            return self.view_as(value, interface.signature.result);
        }
        let (schema, context) = self.constructor_type(expression, expected)?;
        self.schema_access(schema)?;
        if !self.module.layouts[schema].schema.fields.is_empty() {
            return Err(SourceCheckError::Type);
        }
        let ty = match context {
            Some(ty) => ty,
            None => self.instance(schema, Vec::new())?,
        };
        // A variant of a generic sum (`Fine` over `Self`): the recorder reads its type from here.
        self.note_construct(expression, ty);
        self.fresh_value(ty)
    }

    pub(super) fn block_tasks(
        &mut self,
        expression: &'a Expression,
        block: &'a Block,
        expected: Option<SourceType>,
        pending: &mut Vec<aggregates::EvalTask<'a>>,
    ) -> Result<()> {
        use aggregates::EvalTask;
        self.block_syntax(expression, block)?;
        self.charge(block.statements.len())?;
        let base = pending.len();
        let returns = block.statements.last().is_some_and(|s| {
            matches!(s.value, Statement::Return(_)) || self.is_tail(&block.statements, base)
        });
        // A block arm of a value `when` ends in its value (surface-syntax decision 1): its last
        // line, written without `return`, is the arm's value when a value is wanted.
        if !returns
            && let Some((last, rest)) = block.statements.split_last()
            && let Statement::Expression(value) = &last.value
            && expected.is_some_and(|ty| ty != SourceType::Unit)
        {
            self.child(expression.span_id, last.span_id)?;
            self.child(last.span_id, value.span_id)?;
            pending.push(EvalTask::Expression(value, expected, false));
            return self.statement_tasks(expression.span_id, rest, expected, base, pending);
        }
        if !returns {
            pending.push(EvalTask::Unit);
        }
        self.statement_tasks(expression.span_id, &block.statements, expected, base, pending)
    }

    /// Queue `statements` so the first runs first. `base` is the length of `pending` when the
    /// function's own block began; it is zero only there.
    pub(super) fn statement_tasks(
        &mut self,
        parent: SpanId,
        statements: &'a [Spanned<Statement>],
        expected: Option<SourceType>,
        base: usize,
        pending: &mut Vec<aggregates::EvalTask<'a>>,
    ) -> Result<()> {
        use aggregates::EvalTask;
        // A `return` inside a loop is typed against the function's result, not the loop's unit.
        let expected = if self.proof.loop_depth > 0 || base != 0 {
            self.proof.result_expected
        } else {
            expected
        };
        for (index, statement) in statements.iter().enumerate().rev() {
            self.child(parent, statement.span_id)?;
            match &statement.value {
                Statement::Return(ret) => {
                    if index + 1 != statements.len() {
                        return Err(SourceCheckError::Unsupported);
                    }
                    // A `return` in a loop, or in a block inside another expression (a case arm),
                    // leaves the function; only the function's own last statement is its value.
                    if self.proof.loop_depth > 0 || base != 0 {
                        pending.push(EvalTask::Returned);
                    }
                    self.return_tasks(ret, statement.span_id, expected, pending)?;
                }
                Statement::Binding(local) => {
                    pending.push(EvalTask::Local(local, statement.span_id))
                }
                Statement::Destructure(pattern) => {
                    self.destructure_tasks(pattern, statement.span_id, pending)?;
                }
                Statement::Assign(assign) => {
                    self.child(statement.span_id, assign.place.span_id)?;
                    if let Some(compound) = &assign.compound {
                        self.child(statement.span_id, compound.span_id)?;
                    }
                    self.child(statement.span_id, assign.operator)?;
                    self.child(statement.span_id, assign.value.span_id)?;
                    match &assign.compound {
                        Some(compound) => {
                            let infix = InfixOperator::from_binary(compound.value)
                                .ok_or(SourceCheckError::Malformed)?;
                            self.token(compound.span_id, infix.symbol())?;
                            self.token(assign.operator, ":")?;
                        }
                        None => self.token(assign.operator, "::")?,
                    }
                    pending.push(EvalTask::AssignStart(assign));
                }
                Statement::Expression(value) if index + 1 == statements.len() && self.is_tail(statements, base) => {
                    // The last expression of a function's body is its result.
                    self.child(statement.span_id, value.span_id)?;
                    pending.push(EvalTask::Expression(value, expected, false));
                }
                Statement::Expression(value) => {
                    self.child(statement.span_id, value.span_id)?;
                    pending.push(EvalTask::Discard);
                    pending.push(EvalTask::Expression(value, None, false));
                }
                Statement::WhenIs(when) => {
                    // The guard form: the block returns, so the rest of the function is the other
                    // case, and the statement starts the function's block with something after
                    // it. A block that falls through is a one-armed case: the world where the
                    // subject is the member runs it and goes on, the others pass it over
                    // (`case_guards.rs`).
                    let returns = guards::returns(&when.then_block.value);
                    if when.else_branch.is_some()
                        || (returns
                            && (base != 0
                                || (index + 1 == statements.len() && self.proof.loop_depth == 0)))
                    {
                        return Err(SourceCheckError::Unsupported);
                    }
                    pending.push(EvalTask::CaseGuard(when, statement.span_id, expected, base, false));
                }
                Statement::When(when) if !guards::returns(&when.then_block.value) => {
                    // Both branches fall through: the paths join after the statement.
                    pending.push(EvalTask::Join(when, statement.span_id, expected, false));
                }
                Statement::When(when) => {
                    // A guard that returns ends the function's path, so it can only start a
                    // function's own block, and something must follow it.
                    if base != 0 || (index + 1 == statements.len() && self.proof.loop_depth == 0) {
                        return Err(SourceCheckError::Unsupported);
                    }
                    let stage = guards::GuardStage {
                        leaf: 0,
                        visited: false,
                    };
                    pending.push(EvalTask::Guard(when, statement.span_id, expected, base, stage));
                }
                _ => return Err(SourceCheckError::Unsupported),
            }
        }
        Ok(())
    }

    /// Whether the last statement of `statements` is the value of the function's own body: an
    /// expression there, outside any loop.
    fn is_tail(&self, statements: &[Spanned<Statement>], base: usize) -> bool {
        base == 0
            && self.proof.loop_depth == 0
            && matches!(
                statements.last().map(|statement| &statement.value),
                Some(Statement::Expression(_))
            )
    }

    /// Public AST nodes cannot omit written statements or disguise a return as an expression.
    pub(super) fn block_syntax(
        &mut self,
        expression: &'a Expression,
        block: &'a Block,
    ) -> Result<()> {
        self.span(expression.span_id)?;
        let range = self.range_of(expression.span_id);
        let mut cursor = range.start;
        self.charge(block.statements.len())?;
        for statement in &block.statements {
            self.child(expression.span_id, statement.span_id)?;
            let written = self.range_of(statement.span_id);
            self.syntax_trivia(written.file, cursor, written.start)?;
            if let Statement::Expression(child) = &statement.value {
                self.child(statement.span_id, child.span_id)?;
                let child = self.range_of(child.span_id);
                self.syntax_trivia(child.file, written.start, child.start)?;
                self.syntax_trivia(child.file, child.end, written.end)?;
            }
            cursor = written.end;
        }
        self.syntax_trivia(range.file, cursor, range.end)
    }

    /// A local named as a place: its current value, without moving it, reading the place at
    /// `path` below the local, so a loan on another field of it does not conflict.
    pub(super) fn local_place_at(&mut self, expression: &'a Expression, path: &[String]) -> Result<Value> {
        let Expr::Name(name) = &expression.value else {
            return Err(SourceCheckError::Malformed);
        };
        self.span(expression.span_id)?;
        self.token(expression.span_id, name)?;
        let index = self
            .local_position(name)?
            .ok_or(SourceCheckError::UnknownName)?;
        let value = self.proof.names[index].1;
        self.array_complete(value)?;
        if value.owner.is_some() {
            self.charge_flow()?;
            if !value.available {
                return Err(SourceCheckError::UseAfterMove);
            }
            self.read_guard_at(value, path)?;
        }
        self.scalar_facts(value)
    }

    /// A local, or a field path `a.b.c` rooted at one, as a place: the value found there,
    /// still owned by whatever owns the root. Nothing is moved or consumed.
    pub(super) fn project_place(&mut self, expression: &'a Expression) -> Result<Value> {
        self.project_place_at(expression, &place_fields(expression))
    }

    /// [`Self::project_place`] where `path` is the whole path being read, not just the part
    /// of it below this call's base.
    pub(super) fn project_place_at(&mut self, expression: &'a Expression, path: &[String]) -> Result<Value> {
        match &expression.value {
            Expr::Name(_) => self.local_place_at(expression, path),
            Expr::Member { base, name } => {
                self.span(expression.span_id)?;
                self.child(expression.span_id, base.span_id)?;
                self.child(expression.span_id, name.span_id)?;
                self.name(name)?;
                let parent = self.project_place_at(base, path)?;
                let child = self.projection_value(parent, name)?;
                self.scalar_facts(child)
            }
            Expr::Group(inner) => {
                self.group_syntax(expression, inner)?;
                self.project_place_at(inner, path)
            }
            _ => Err(SourceCheckError::Unsupported),
        }
    }

    /// Whether `expression` names a place that can be lent: a local, or a field path of one.
    /// A method called without parentheses is a call, not a place.
    pub(super) fn is_place(&mut self, expression: &'a Expression) -> Result<bool> {
        match &expression.value {
            Expr::Name(name) => Ok(self.local_position(name)?.is_some()),
            Expr::Member { base, name } => {
                if self.method_target(expression)?.is_some() {
                    return Ok(false);
                }
                // A derived `.value` is computed, not stored.
                if name.value == "value" && self.discriminant_union(base).is_some() {
                    return Ok(false);
                }
                self.is_place(base)
            }
            Expr::Group(inner) => self.is_place(inner),
            _ => Ok(false),
        }
    }

    /// Lend a place to a `ref` (shared) or `mut` (exclusive) parameter without moving it.
    pub(super) fn borrow_place(
        &mut self,
        expression: &'a Expression,
        exclusive: bool,
    ) -> Result<Value> {
        let mut root = expression;
        while let Expr::Member { base, .. } = &root.value {
            root = base;
        }
        let path = place_fields(expression);
        let root_value = self.local_place_at(root, &path)?;
        if let Expr::Name(root_name) = &root.value {
            self.lend_guard(root_value, root_name, exclusive, &path)?;
        }
        if let Some(owner) = root_value.owner {
            // Shared access cannot be lent out as exclusive access, whole or in part.
            let held = &self.proof.owners[owner];
            if exclusive && held.passing == calls::Passing::Ref && !held.unrestricted {
                return Err(SourceCheckError::BorrowConflict);
            }
        }
        if std::ptr::eq(root, expression) {
            return Ok(root_value);
        }
        self.project_place(expression)
    }

    /// Whether `name` is a borrowed receiver of the domain whose method is being checked.
    /// Reading its hidden integer copies an unrestricted number and moves nothing.
    pub(super) fn is_borrowed_domain_read(&mut self, name: &str) -> Result<bool> {
        if !self.receiver_is_domain() {
            return Ok(false);
        }
        let Some(index) = self.local_position(name)? else {
            return Ok(false);
        };
        let value = self.proof.names[index].1;
        Ok(self.proof.receiver == Some(value.ty)
            && value.owner.is_some_and(|owner| {
                let held = &self.proof.owners[owner];
                held.passing.borrows() && !held.unrestricted
            }))
    }

    pub(super) fn evaluate_action(
        &mut self,
        task: aggregates::EvalTask<'a>,
        pending: &mut Vec<aggregates::EvalTask<'a>>,
        values: &mut Vec<Value>,
    ) -> Result<()> {
        use aggregates::EvalTask;
        match task {
            EvalTask::Expression(..) => unreachable!("expression handled by evaluator"),
            EvalTask::LoopNext(frame) => self.loop_next(frame, pending, values)?,
            EvalTask::LoopEnd(iteration) => self.loop_end(iteration, pending, values)?,
            EvalTask::Sequence(count, expected) => {
                if values.len() < count {
                    return Err(SourceCheckError::Malformed);
                }
                let children = values.split_off(values.len() - count);
                values.push(self.sequence_pack(children, expected)?);
            }
            EvalTask::Discard => {
                let value = values.pop().ok_or(SourceCheckError::Malformed)?;
                if let Some(owner) = value.owner {
                    self.charge_flow()?;
                    self.obligate(owner);
                }
            }
            EvalTask::RangeLoop(frame) => {
                let end = values.pop().ok_or(SourceCheckError::Malformed)?;
                let start = values.pop().ok_or(SourceCheckError::Malformed)?;
                self.range_loop(&frame, start, end)?;
                values.push(self.proof.returned_value.take().unwrap_or_else(Value::unit));
            }
            EvalTask::AssignStart(assign) if Self::place_is_indexed(&assign.place) => {
                self.element_write(assign)?;
            }
            EvalTask::AssignStart(assign) => {
                self.require_assignable(&assign.place)?;
                let declared = self.place_declared_type(&assign.place)?;
                pending.push(EvalTask::Assign(assign, declared));
                if assign.compound.is_some() {
                    // The place is read first, as the left operand of `place op value`.
                    // A linear place is never overwritten, so it is not read either.
                    if self.capability_of(declared) == Capability::Linear {
                        return Err(SourceCheckError::OverwriteLinear);
                    }
                    pending.push(EvalTask::Expression(&assign.value, None, false));
                    pending.push(EvalTask::Expression(&assign.place, None, false));
                } else {
                    pending.push(EvalTask::Expression(&assign.value, Some(declared), false));
                }
            }
            EvalTask::Assign(assign, declared) => {
                let mut value = values.pop().ok_or(SourceCheckError::Malformed)?;
                if let Some(compound) = &assign.compound {
                    let left = values.pop().ok_or(SourceCheckError::Malformed)?;
                    value = self.compound_value(compound.value, left, value)?;
                }
                // The value must fit what the place is declared to hold.
                let value = self.view_as(value, declared)?;
                self.store_guard(&assign.place, value)?;
                self.assign(assign, value)?;
            }
            EvalTask::Borrow(expression, exclusive) => {
                values.push(self.borrow_place(expression, exclusive)?);
            }
            EvalTask::Handle(expression, exclusive) => {
                values.push(self.field_handle(expression, exclusive)?);
            }
            EvalTask::Unit => values.push(Value::unit()),
            EvalTask::Returned => {
                if let Some(value) = values.last() {
                    self.return_guard(*value)?;
                }
                self.proof.returned = true;
            }
            EvalTask::CaseGuard(when, span, expected, base, visited) => {
                self.case_guard_tasks(when, span, expected, base, visited, pending, values)?;
            }
            EvalTask::Unnarrow(name, before) => {
                self.charge(self.proof.names.len())?;
                if let Some(index) = self.local_position(name)? {
                    let available = self.proof.names[index].1.available;
                    self.proof.names[index].1 = Value { available, ..before };
                }
            }
            EvalTask::DomainBounds(domain) => self.domain_bounds_task(domain, values)?,
            EvalTask::Join(when, span, expected, visited) => {
                self.join_tasks(when, span, expected, visited, pending, values)?;
            }
            EvalTask::JoinThen(frame) => self.join_then(*frame, pending)?,
            EvalTask::JoinElse(frame) => self.join_else(*frame)?,
            EvalTask::Guard(when, span, expected, base, stage) => {
                self.guard_tasks(when, span, expected, base, stage, pending, values)?;
            }
            EvalTask::Local(local, parent) => {
                let expected = self.local_type(local, parent)?;
                pending.push(EvalTask::Bind(local, expected));
                pending.push(EvalTask::Expression(
                    &local
                        .value
                        .as_ref()
                        .ok_or(SourceCheckError::MissingBody)?
                        .expression,
                    expected,
                    false,
                ));
            }
            EvalTask::Bind(local, expected) => {
                let mut value = values.pop().ok_or(SourceCheckError::Malformed)?;
                if let Some(expected) = expected {
                    value = self.view_as(value, expected)?;
                    self.refinements(local.annotation.as_ref().unwrap(), value)?;
                }
                let name = self.name(&local.name)?;
                self.insert(name, value)?;
                // A `:` local can be reassigned; a `:=` local is fixed once bound.
                if local
                    .value
                    .as_ref()
                    .is_some_and(|bound| bound.operator.value == BindingOperator::Colon)
                {
                    self.mark_assignable(name);
                }
            }
            EvalTask::BindView(local, expected) => {
                let mut value = values.pop().ok_or(SourceCheckError::Malformed)?;
                if let Some(expected) = expected {
                    value = self.view_as(value, expected)?;
                    self.refinements(local.annotation.as_ref().unwrap(), value)?;
                }
                values.push(value);
            }
            EvalTask::Destructure(pattern) => {
                let value = values.pop().ok_or(SourceCheckError::Malformed)?;
                self.bind_pattern(&pattern.value, pattern.span_id, value)?;
            }
            EvalTask::Arm(arm, subject, expected) if arm.guard.is_some() => {
                self.guarded_arm(arm, subject, expected, None, pending)?;
            }
            EvalTask::Arm(arm, subject, expected) => {
                self.arm_tasks(arm, subject, expected, pending)?;
            }
            EvalTask::ValidateArm(arm, ty, expected, real) => {
                let state = self.validation_state()?;
                let subject = self.input_value(ty)?;
                // In the world where this variant was returned its claims hold.
                if let (Some(real), Some(fresh)) = (real, subject.owner) {
                    let own: Vec<_> =
                        self.proof.owners[real].claims.iter().filter(|(member, _, _)| *member == ty).cloned().collect();
                    self.proof.owners[fresh].claims = own;
                }
                pending.push(EvalTask::ValidationEnd(state, expected, values.len()));
                pending.push(EvalTask::Arm(arm, subject, expected));
            }
            EvalTask::ValidateIntegerArm(arm, subject, number, expected) => {
                let state = self.validation_state()?;
                let subject = self.hypothetical_integer(subject, number)?;
                pending.push(EvalTask::ValidationEnd(state, expected, values.len()));
                pending.push(EvalTask::Arm(arm, subject, expected));
            }
            EvalTask::ValidationEnd(state, expected, base) => {
                assert_eq!(values.len(), base + 1);
                let value = values.pop().ok_or(SourceCheckError::Malformed)?;
                self.end_validation(state, value, expected)?;
            }
            EvalTask::Scope(saved, hidden) => self.end_arm_scope(saved, &hidden)?,
            EvalTask::Chain(expression, index, expected) => {
                self.chain_tasks(expression, index, expected, pending)?;
            }
            EvalTask::Cond(frame) => self.cond_step(*frame, pending, values)?,
            EvalTask::Candidates(arms, subject, expected) => {
                self.candidate_tasks(arms, subject, expected, pending)?;
            }
        }
        Ok(())
    }

    /// The local names a pattern binds, the patterns inside it included.
    fn pattern_names(&mut self, pattern: &'a VariantPattern, names: &mut Vec<&'a str>) -> Result<()> {
        let Some(fields) = &pattern.fields else { return Ok(()) };
        for (index, binder) in fields.binders.value.iter().enumerate() {
            match fields.nested.get(index) {
                Some(Some(inner)) => self.pattern_names(&inner.value, names)?,
                _ => names.push(self.name(binder)?),
            }
        }
        Ok(())
    }

    pub(super) fn arm_tasks(
        &mut self,
        arm: &'a MatchArm,
        subject: Value,
        expected: Option<SourceType>,
        pending: &mut Vec<aggregates::EvalTask<'a>>,
    ) -> Result<()> {
        use aggregates::EvalTask;
        self.charge(self.proof.names.len())?;
        // The claims of the variant this arm takes hold in it, and nowhere else.
        self.apply_claims(subject)?;
        let saved = self.proof.names.clone();
        let mut hidden = Vec::new();
        if let MatchPattern::Variant(pattern) = &arm.pattern.value
            && let Some(fields) = &pattern.fields
        {
            self.charge(fields.binders.value.len())?;
            self.pattern_names(pattern, &mut hidden)?;
        }
        self.charge_product(self.proof.names.len(), hidden.len() + 1)?;
        self.proof.names.retain(|(name, _)| !hidden.contains(name));
        match &arm.pattern.value {
            MatchPattern::Variant(pattern) => {
                let variants = match subject.sum {
                    Some(id) => self.sum_variants(id)?,
                    None => vec![subject.ty],
                };
                if let Some((union, _)) = self.nested_union(pattern, &variants)? {
                    // The whole union value, whichever of its variants the subject is.
                    let SourceType::Sum { id } = union else {
                        return Err(SourceCheckError::Type);
                    };
                    let mut kept = false;
                    if let Some(fields) = &pattern.fields
                        && let Some(binder) = fields.binders.value.first()
                    {
                        let name = self.name(binder)?;
                        if name != "_" {
                            let mut value = subject;
                            value.sum = Some(id);
                            self.insert(name, value)?;
                            kept = true;
                        }
                    }
                    // An arm that names no binder ends the value, as unpacking one does.
                    if !kept {
                        self.materialize_children(subject)?;
                        self.release(subject)?;
                    }
                } else {
                    self.bind_pattern(pattern, arm.pattern.span_id, subject)?;
                }
            }
            MatchPattern::Integer(pattern) => {
                let number = self.integer_pattern(pattern, arm.pattern.span_id)?;
                self.guard_integer(subject, number)?;
            }
            // A case has no arm for a domain (`is in` is a condition), and a text or `else`
            // arm belongs to a case over text, which does not come through here.
            MatchPattern::Domain { .. } | MatchPattern::Text(_) | MatchPattern::Else { .. } => {
                return Err(SourceCheckError::Type);
            }
        }
        pending.push(EvalTask::Scope(saved, hidden));
        pending.push(EvalTask::Expression(&arm.body, expected, false));
        Ok(())
    }

    pub(super) fn end_arm_scope(
        &mut self,
        mut saved: Vec<(&'a str, Value)>,
        hidden: &[&'a str],
    ) -> Result<()> {
        self.charge_product(saved.len(), self.proof.names.len() + hidden.len() + 1)?;
        for (name, value) in &mut saved {
            if !hidden.contains(name)
                && let Some((_, current)) = self.proof.names.iter().find(|(n, _)| n == name)
            {
                *value = *current;
            }
        }
        self.proof.names = saved;
        Ok(())
    }

    fn end_validation(
        &mut self,
        state: ValidationState<'a>,
        value: Value,
        expected: Option<SourceType>,
    ) -> Result<()> {
        if let Some(expected) = expected {
            self.view_as(value, expected)?;
        }
        self.release(value)?;
        self.charge(self.proof.owners.len() - state.owners.len())?;
        if self.proof.owners[state.owners.len()..]
            .iter()
            .any(|owner| owner.live)
        {
            return Err(SourceCheckError::Unconsumed);
        }
        self.proof.restore_validation(state);
        Ok(())
    }

    pub(super) fn validation_state(&mut self) -> Result<ValidationState<'a>> {
        self.charge(
            self.proof.names.len()
                + self.proof.owners.len()
                + self.proof.facts.len()
                + self.proof.scalars.len()
                + self.proof.integer_guards.len(),
        )?;
        for index in 0..self.proof.owners.len() {
            self.charge(
                self.proof.owners[index].children.len()
                    + self.proof.owners[index].slots.len()
                    + self.proof.owners[index].moved.len(),
            )?;
        }
        Ok(self.proof.validation_snapshot())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn populated(source: &SourceFile) -> Checker<'_> {
        let mut checker = Checker::new(source, crate::Limits::default());
        let ty = checker
            .array_type(SourceType::Generic { index: 0 }, 3)
            .unwrap();
        let array = checker.input_value(ty).unwrap();
        checker.array_child(array, 1).unwrap();
        checker.insert("array", array).unwrap();
        checker.proof.owners[array.owner.unwrap()].moved.push(1);
        let scalar = checker
            .fresh_value(Value::integer(I256::ZERO, I256::ONE).ty)
            .unwrap();
        checker.insert("number", scalar).unwrap();
        checker.guard_integer(scalar, I256::ONE).unwrap();
        let identity = scalar.identity.unwrap();
        checker
            .proof
            .facts
            .push((identity, BinaryOperator::Equal, identity));
        checker.proof.generics.push("element");
        checker.proof.parameter_roles.push(calls::Parameter::Type);
        let inputs = checker.summary_inputs().unwrap();
        checker.proof.summary = Some(checker.summarize(&inputs, Some(array)).unwrap());
        checker
    }

    #[test]
    fn hypothetical_snapshot_prepays_exact_subset_and_nested_owner_storage() {
        let source = parser::parse_strict("").unwrap();
        let mut checker = populated(&source);
        // Five vector lengths: 2 names + 2 owners + 1 fact + 1 scalar + 1 guard.
        // Nested storage: 1 child + 1 slot + 1 moved slot.
        let cost = 10;
        let flow = checker.flow_remaining;
        let identity = checker.proof.next_identity;
        checker.remaining = cost - 1;
        assert!(matches!(
            checker.validation_state(),
            Err(SourceCheckError::Budget)
        ));
        // Original charge ordering: the outer seven entries were paid before
        // failing the first owner's three-entry nested storage prepayment.
        assert_eq!(checker.remaining, 2);
        assert_eq!(checker.proof.names.len(), 2);
        assert_eq!(checker.proof.owners.len(), 2);
        assert_eq!(checker.proof.next_identity, identity);
        assert_eq!(checker.flow_remaining, flow);
        checker.remaining = cost;
        let state = checker.validation_state().unwrap();
        assert_eq!(checker.remaining, 0);
        assert_eq!(checker.flow_remaining, flow);
        assert_eq!(state.names.len(), 2);
        assert_eq!(state.owners[0].children.len(), 1);
        assert_eq!(state.owners[0].slots, [1]);
        assert_eq!(state.owners[0].moved, [1]);
        assert_eq!(state.next_identity, identity);
    }

    #[test]
    fn hypothetical_restore_excludes_summary_binders_module_replay_and_accounting() {
        let source = parser::parse_strict("").unwrap();
        let mut checker = populated(&source);
        let state = checker.validation_state().unwrap();
        let identity = state.next_identity;
        let number_identity = state.names[1].1.identity;
        let child_identity = state.owners[0].children[0].identity;
        checker.proof.names.clear();
        checker.proof.owners[0].children.clear();
        checker.proof.owners[0].slots.clear();
        checker.proof.owners[0].moved.clear();
        checker.proof.owners[0].live = false;
        checker.proof.facts.clear();
        checker.proof.scalars.clear();
        checker.proof.integer_guards.clear();
        // Validation does not save these fields, nor refund their work.
        checker.proof.summary = None;
        checker.proof.generics.push("other");
        checker.proof.parameter_roles.push(calls::Parameter::Type);
        checker
            .fresh_value(SourceType::Generic { index: 1 })
            .unwrap();
        let new_type = checker.sequence_type(vec![SourceType::Unit]).unwrap();
        let instances = checker.module.instances.len();
        checker.replay.begin_function();
        checker.cache_source_type((11, false), new_type).unwrap();
        assert_eq!(checker.choose(2).unwrap(), 0);
        let before_end = (checker.remaining, checker.flow_remaining);

        checker.end_validation(state, Value::unit(), None).unwrap();

        assert_eq!(checker.proof.names.len(), 2);
        assert_eq!(checker.proof.names[0].0, "array");
        assert_eq!(checker.proof.names[1].1.identity, number_identity);
        assert_eq!(checker.proof.owners.len(), 2);
        assert!(checker.proof.owners[0].live);
        assert_eq!(checker.proof.owners[0].children[0].identity, child_identity);
        assert_eq!(checker.proof.owners[0].slots, [1]);
        assert_eq!(checker.proof.owners[0].moved, [1]);
        assert_eq!(checker.proof.facts.len(), 1);
        assert_eq!(checker.proof.scalars.len(), 1);
        assert_eq!(checker.proof.integer_guards.len(), 1);
        assert_eq!(checker.proof.next_identity, identity);
        assert!(checker.proof.summary.is_none());
        assert_eq!(checker.proof.generics, ["element", "other"]);
        assert_eq!(checker.proof.parameter_roles.len(), 2);
        assert_eq!(checker.module.instances.len(), instances);
        assert_eq!(checker.replay.choices, [(0, 2)]);
        assert_eq!(checker.replay.choice_cursor, 1);
        assert_eq!(
            checker.replay.type_cache.as_ref().unwrap(),
            &[((11, false), new_type)]
        );
        // Unit release costs one work unit, then one discarded witness owner.
        assert_eq!(checker.remaining, before_end.0 - 2);
        assert_eq!(checker.flow_remaining, before_end.1);
    }
}

/// The field names of a place `a.b.c`, below its root local: `["b", "c"]`.
pub(super) fn place_fields(expression: &Expression) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = expression;
    loop {
        match &current.value {
            Expr::Member { base, name } => {
                fields.push(name.value.clone());
                current = base;
            }
            Expr::Group(inner) => current = inner,
            _ => break,
        }
    }
    fields.reverse();
    fields
}

impl<'a> Checker<'a> {
    /// `attempt is Vacant` with no parentheses, `Vacant` a member with fields: the test narrows
    /// `attempt` to that member (flow typing) instead of taking it apart, so the subject is the
    /// same value afterwards. A member without fields keeps the old meaning (it is matched).
    pub(super) fn narrows(&self, pattern: &VariantPattern, schema: usize) -> bool {
        pattern.fields.is_none() && !self.module.layouts[schema].schema.fields.is_empty()
    }
}

/// `.value` of a union that gives its variants values (`Debug = 'debug'`): derived, never
/// written. Read from a local holding the union, or from a variant written out
/// (`LogLevel.Debug.value`).
impl<'a> Checker<'a> {
    /// The schema of the union `base` holds, when `base.value` is a derived discriminant read:
    /// `base` is a local of a union whose variants have values, or `Union.Variant` of one.
    /// Uncharged; the read itself charges.
    pub(super) fn discriminant_union(&self, base: &'a Expression) -> Option<usize> {
        let variant_union = |schema: usize| -> Option<usize> {
            self.module.layouts.get(schema)?.schema.discriminant.as_ref()?;
            self.module
                .union_variants
                .iter()
                .find(|(_, _, layout)| *layout == schema)
                .map(|(union, _, _)| *union)
                .or_else(|| {
                    // An imported variant: its union is the sum that lists it.
                    (0..self.module.layouts.len()).find(|&union| {
                        self.module.layouts[union].schema.variants.iter().any(|variant| {
                            matches!(variant, SourceType::Aggregate { id }
                                if self.module.instances[*id as usize].schema() == schema)
                        })
                    })
                })
        };
        let union_of_type = |ty: SourceType| -> Option<usize> {
            match ty {
                SourceType::Sum { id } => {
                    let union = self.module.instances[id as usize].schema();
                    let first = *self.module.layouts[union].schema.variants.first()?;
                    let SourceType::Aggregate { id } = first else { return None };
                    let variant = self.module.instances[id as usize].schema();
                    self.module.layouts[variant].schema.discriminant.as_ref().map(|_| union)
                }
                SourceType::Aggregate { id } => variant_union(self.module.instances[id as usize].schema()),
                _ => None,
            }
        };
        match &base.value {
            Expr::Name(name) => {
                let index = self.proof.names.iter().position(|(other, _)| *other == name.as_str())?;
                let value = self.proof.names[index].1;
                value
                    .sum
                    .and_then(|id| union_of_type(SourceType::Sum { id }))
                    .or_else(|| union_of_type(value.ty))
            }
            Expr::Member { base: union, name } => {
                let Expr::Name(union_name) = &union.value else { return None };
                if self.proof.names.iter().any(|(other, _)| *other == union_name.as_str()) {
                    return None;
                }
                let found = self
                    .module
                    .type_names
                    .binary_search_by(|(other, _)| other.as_str().cmp(union_name))
                    .ok()?;
                let schema = self.module.type_names[found].1;
                let layout = self.own_variant(schema, &name.value)?;
                self.module.layouts[layout].schema.discriminant.as_ref().map(|_| schema)
            }
            Expr::Group(inner) => self.discriminant_union(inner),
            _ => None,
        }
    }

    /// `base.value`: the active variant's literal, as an integer or a text whose byte length is
    /// known (so it may become a byte view, as a literal does).
    pub(super) fn discriminant_value(&mut self, expression: &'a Expression, base: &'a Expression, name: &'a Name) -> Result<Value> {
        self.child(expression.span_id, base.span_id)?;
        self.child(expression.span_id, name.span_id)?;
        self.name(name)?;
        self.charge(2)?;
        let held = if self.is_place(base)? {
            self.project_place(base)?
        } else {
            self.bare_value(base, None)?
        };
        let held = self.inspect_value(held)?;
        let SourceType::Aggregate { id } = held.ty else {
            return Err(SourceCheckError::Type);
        };
        let schema = self.module.instances[id as usize].schema();
        let value = self.module.layouts[schema]
            .schema
            .discriminant
            .clone()
            .ok_or(SourceCheckError::Type)?;
        match value {
            Discriminant::Integer(number) => Ok(Value::integer(number, number)),
            Discriminant::Text(text) => {
                self.charge(text.len() + 1)?;
                let mut value = self.fresh_value(SourceType::Text)?;
                let length = I256::from(text.len() as u64);
                (value.lower, value.upper, value.integer_domain) = (length, length, Some((length, length)));
                Ok(value)
            }
        }
    }
}

/// The fields a variant declares in `Name(...)`: the payload written without a name (field `""`),
/// or `name Type` per field, the unnamed one only first. `None` for any other argument list.
pub(super) fn payload_fields(arguments: &[Spanned<Argument>]) -> Option<Vec<(&str, &Expression)>> {
    let mut fields: Vec<(&str, &Expression)> = Vec::new();
    for (position, argument) in arguments.iter().enumerate() {
        match &argument.value {
            Argument::Positional(written) if position == 0 => fields.push(("", written)),
            Argument::Typed { name, annotation } => fields.push((name.value.as_str(), annotation)),
            _ => return None,
        }
    }
    let unnamed_only = fields.len() == 1;
    (!fields.is_empty() && (unnamed_only || fields.iter().skip(1).all(|(name, _)| !name.is_empty()))).then_some(fields)
}
