//! Acyclic nominal type applications and a flat ledger of fully initialized values.
//!
//! A root owns its existing child IDs, not freshly synthesized child obligations.
//! Binding availability records moves independently of the ledger's live frontier:
//! an alias activates the same root, but never makes the old binding available again.
use super::*;

#[derive(Debug, Clone)]
pub(super) struct Instance {
    pub(super) head: CheckedSourceInstanceHead,
    pub(super) arguments: Vec<SourceType>,
}

impl Instance {
    pub(super) fn schema(&self) -> usize {
        let CheckedSourceInstanceHead::Nominal { schema } = self.head else {
            unreachable!("nominal type has a nominal instance head")
        };
        schema as usize
    }
}

/// Directive lines seen since the last declaration, awaiting the type they mark.
#[derive(Default)]
struct Markers {
    wrap: Option<SpanId>,
    unrestricted: Option<SpanId>,
    /// The string of an `#instruction` line awaiting its function.
    instruction: Option<SpanId>,
    /// A `#load` or `#store` line awaiting its function.
    memory: Option<(SpanId, MemoryOp)>,
}

#[derive(Clone)]
pub(super) struct Owner {
    pub(super) live: bool,
    /// Unrestricted values carry no obligation: they are never live and never move.
    pub(super) unrestricted: bool,
    /// How the value was passed in. A `ref` or `mut` borrow leaves responsibility with
    /// its owner, so this owner is never live and cannot be moved, ended or returned.
    pub(super) passing: calls::Passing,
    pub(super) children: Vec<Value>,
    /// Sorted original array slots parallel to children; records remain dense.
    pub(super) slots: Vec<u32>,
    pub(super) moved: Vec<u32>,
    /// The active nominal type is shared by all aliases of this owner.
    pub(super) active: Option<SourceType>,
    /// Empty records and uninspected records must remain distinguishable.
    pub(super) materialized: bool,
    /// Arrays have no independent ending authority until every owned slot moved.
    pub(super) drained: bool,
    /// Loans taken on this owner by handles (`loans.rs`). Cloned with the owner, so branch
    /// and loop snapshots carry them.
    pub(super) lent: Vec<loans::Loan>,
    /// A borrowing record that owns nothing: it carries no obligation and may be dropped.
    pub(super) view: bool,
    /// For an array of run-time length: the identity of the number it is as long as.
    pub(super) extent: Option<u32>,
    /// The claims of the variants a call's result may be (`where` after each written variant), with
    /// the call's argument values: assumed when the variant is the one in play (`apply_claims`),
    /// not at the call, so that an arm that cannot be taken is not checked under them.
    pub(super) claims: Vec<(SourceType, Vec<contracts::Constraint>, Vec<Value>)>,
}

pub(super) enum EvalTask<'a> {
    Expression(&'a Expression, Option<SourceType>, bool),
    Sequence(usize, Option<SourceType>),
    LoopNext(Box<loops::Frame<'a>>),
    LoopEnd(Box<loops::Iteration<'a>>),
    Discard,
    Local(&'a Binding, SpanId),
    Bind(&'a Binding, Option<SourceType>),
    /// `(name in Domain: value)`: the value, viewed in the domain; the name is not kept.
    BindView(&'a Binding, Option<SourceType>),
    Destructure(&'a Spanned<VariantPattern>),
    Scope(Vec<(&'a str, Value)>, Vec<&'a str>),
    Arm(&'a MatchArm, Value, Option<SourceType>),
    /// A detached check of an arm no world selects: the arm, the variant its pattern names, the
    /// declared result, and the owner of the real subject, whose claims for that variant hold in
    /// the hypothetical world.
    ValidateArm(&'a MatchArm, SourceType, Option<SourceType>, Option<usize>),
    ValidateIntegerArm(&'a MatchArm, Value, I256, Option<SourceType>),
    ValidationEnd(super::sums::ValidationState<'a>, Option<SourceType>, usize),
    /// `for i in start to end do body`: both bounds are evaluated; check the body once.
    RangeLoop(Box<loops::RangeFrame<'a>>),
    /// `place :: value`: work out the place's declared type, then evaluate the value to it.
    AssignStart(&'a AssignStatement),
    /// The value of an assignment is ready; fit it to the place's type and store it there.
    Assign(&'a AssignStatement, SourceType),
    /// Lend a local to a `ref` (shared) or `mut` (exclusive) parameter without moving it.
    Borrow(&'a Expression, bool),
    /// The handle given to a `ref` (false) or `mut` (true) field of a borrowing record.
    Handle(&'a Expression, bool),
    /// `when c then <block ending in return>` at the start of a function's body: the statement,
    /// its span, the declared result, the length `pending` had when the body began, and whether
    /// which comparison of the condition is next and whether its operands are evaluated.
    Guard(&'a WhenStatement, SpanId, Option<SourceType>, usize, guards::GuardStage),
    /// `when c then ...` whose branches fall through: the statement, its span, the declared
    /// result and whether the operands of the comparison are evaluated.
    Join(&'a WhenStatement, SpanId, Option<SourceType>, bool),
    /// The bounds of the domain after `is in`, pushed lower then upper.
    DomainBounds(&'a Expression),
    /// `when subject is Pattern then ... return`: the statement, its span, the declared result,
    /// the length `pending` had when the function's block began, and whether the subject is
    /// evaluated.
    CaseGuard(&'a WhenIsStatement, SpanId, Option<SourceType>, usize, bool),
    /// The arm of a one-armed case that falls through has run: the subject, narrowed to the member
    /// inside the arm, is what it was before.
    Unnarrow(&'a str, Value),
    /// The then-block has run; the else-block is next.
    JoinThen(Box<joins::JoinFrame<'a>>),
    /// The else-block has run; the two ends are joined.
    JoinElse(Box<joins::JoinFrame<'a>>),
    /// A `return` inside a loop body has computed its value: the path ends here.
    Returned,
    Unit,
    /// Arm `index` of a condition chain, with the earlier conditions assumed false.
    Chain(&'a Expression, usize, Option<SourceType>),
    /// A compound condition, a term at a time.
    Cond(Box<chains::CondFrame<'a>>),
    /// The arms a member can take, the first whose guard holds being the one that runs.
    Candidates(Vec<&'a MatchArm>, Value, Option<SourceType>),
}

impl<'a> Checker<'a> {
    pub(super) fn collect(&mut self) -> diagnostics::DiagnosticResult<()> {
        let files = self.files;
        for file in files {
            let mut pending = Markers::default();
            for item in &file.items {
                self.collect_item(item, &mut pending)
                    .map_err(|error| self.failure(error, item.span_id))?;
            }
            if let Some(marker) = pending.instruction {
                return Err(self.failure(SourceCheckError::InstructionTarget, marker));
            }
            if let Some((marker, _)) = pending.memory {
                return Err(self.failure(SourceCheckError::InstructionTarget, marker));
            }
            if let Some(marker) = pending.wrap {
                return Err(self.failure(SourceCheckError::WrapTarget, marker));
            }
            if let Some(marker) = pending.unrestricted {
                return Err(self.failure(SourceCheckError::UnrestrictedTarget, marker));
            }
        }
        Ok(())
    }

    /// `#instruction` marks the body-less function that follows. Whether `item` was the
    /// marker or the function it marks, whose collection is then finished here.
    fn collect_instruction(
        &mut self,
        item: &'a Spanned<Item>,
        pending: &mut Markers,
    ) -> Result<bool> {
        if let Item::Load | Item::Store = item.value {
            let op = if item.value == Item::Load { MemoryOp::Load } else { MemoryOp::Store };
            if pending.instruction.is_some() || pending.memory.replace((item.span_id, op)).is_some() {
                return Err(SourceCheckError::Duplicate);
            }
            return Ok(true);
        }
        if let Some((_, op)) = pending.memory.take() {
            return match &item.value {
                Item::Binding(binding) if binding.value.is_none() => {
                    self.module.memory.push((binding, op));
                    Ok(true)
                }
                _ => Err(SourceCheckError::InstructionTarget),
            };
        }
        if let Item::Instruction { text } = item.value {
            if pending.instruction.replace(text).is_some() {
                return Err(SourceCheckError::Duplicate);
            }
            return Ok(true);
        }
        let Some(text) = pending.instruction.take() else {
            return Ok(false);
        };
        let mnemonic = self.instruction_text(text)?;
        match &item.value {
            Item::Binding(binding) if binding.value.is_none() => {
                self.module.instructions.push((binding, mnemonic));
                Ok(true)
            }
            _ => Err(SourceCheckError::InstructionTarget),
        }
    }

    /// `#wrap` must be followed directly by the integer domain it marks; anything else it
    /// could mark (a record, a sum, a function, nothing) is a `WrapTarget` flaw.
    /// `#unrestricted` must likewise precede a non-generic domain or record, else it is an
    /// `UnrestrictedTarget` flaw. The two markers may appear in either order.
    fn collect_item(&mut self, item: &'a Spanned<Item>, pending: &mut Markers) -> Result<()> {
        self.span(item.span_id)?;
        if self.collect_instruction(item, pending)? {
            return Ok(());
        }
        let marker = match item.value {
            Item::Wrap => Some((&mut pending.wrap, "#wrap")),
            Item::Unrestricted => Some((&mut pending.unrestricted, "#unrestricted")),
            _ => None,
        };
        if let Some((slot, text)) = marker {
            self.token(item.span_id, text)?;
            if slot.replace(item.span_id).is_some() {
                return Err(SourceCheckError::Duplicate);
            }
            return Ok(());
        }
        let wrap = pending.wrap.take().is_some();
        let unrestricted = pending.unrestricted.take().is_some();
        let Item::Type(declaration) = &item.value else {
            return if wrap {
                Err(SourceCheckError::WrapTarget)
            } else if unrestricted {
                Err(SourceCheckError::UnrestrictedTarget)
            } else {
                Ok(())
            };
        };
        self.child(item.span_id, declaration.name.span_id)?;
        let name = self.name(&declaration.name)?;
        self.charge_product(self.item_count(), name.len() + 1)?;
        let count = self
            .files
            .iter()
            .flat_map(|file| &file.items)
            .filter(|item| match &item.value {
                Item::Type(other) => other.name.value == name,
                Item::Binding(other) => other.name.value == name,
                _ => false,
            })
            .count();
        // A domain or union and the `has` block that gives it methods share one name.
        let members_of = if matches!(declaration.body.value, TypeBody::Has(_)) {
            self.paired_schema(name)
        } else {
            None
        };
        // The domain or union itself may share its name with exactly one `has` block.
        let has_partner = matches!(
            declaration.body.value,
            TypeBody::Domain(_) | TypeBody::Is(_)
        ) && self
            .files
            .iter()
            .flat_map(|file| &file.items)
            .filter(|item| {
                matches!(&item.value, Item::Type(other)
                        if other.name.value == name && matches!(other.body.value, TypeBody::Has(_)))
            })
            .count()
            == 1;
        if self.is_alias(declaration) {
            if count != 1 {
                return Err(SourceCheckError::Duplicate);
            }
            if wrap {
                return Err(SourceCheckError::WrapTarget);
            }
            if unrestricted {
                return Err(SourceCheckError::UnrestrictedTarget);
            }
            self.child(item.span_id, declaration.body.span_id)?;
            return self.declare_alias(declaration, item.span_id);
        }
        if count != 1 && !(count == 2 && (members_of.is_some() || has_partner)) {
            return Err(SourceCheckError::Duplicate);
        }
        self.child(item.span_id, declaration.body.span_id)?;
        if let Some(parameters) = &declaration.parameters {
            self.child(item.span_id, parameters.span_id)?;
        }
        self.declaration_syntax(item.span_id, declaration)?;
        if let Some(schema) = members_of {
            if wrap || unrestricted {
                return Err(SourceCheckError::UnrestrictedTarget);
            }
            // The `has` block of a generic union or domain names the same parameters.
            let declared = self.module.schemas[schema].parameters.as_ref().map(|p| p.value.len());
            if declared != declaration.parameters.as_ref().map(|p| p.value.len()) {
                return Err(SourceCheckError::Type);
            }
            self.charge(1)?;
            self.module.schema_domain_has[schema] = Some(declaration);
            return Ok(());
        }
        if let TypeBody::Domain(domain) = &declaration.body.value {
            return self.collect_domain(declaration, domain, wrap, unrestricted);
        }
        if wrap {
            return Err(SourceCheckError::WrapTarget);
        }
        // The marker applies to records and sums, generic or not. A generic one is
        // unrestricted only for unrestricted arguments, which instantiation enforces.
        if unrestricted && !matches!(declaration.body.value, TypeBody::Has(_) | TypeBody::Is(_)) {
            return Err(SourceCheckError::UnrestrictedTarget);
        }
        if !matches!(declaration.body.value, TypeBody::Has(_) | TypeBody::Is(_)) {
            return Err(SourceCheckError::Unsupported);
        }
        let body = self.span(declaration.body.span_id)?;
        let keyword = if matches!(declaration.body.value, TypeBody::Has(_)) {
            "has"
        } else {
            "is"
        };
        if !body
            .strip_prefix(keyword)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
        {
            return Err(SourceCheckError::Malformed);
        }
        self.charge(1)?;
        if self.is_trait(declaration) {
            // A trait is a source of members, never a type: no value has it.
            if unrestricted {
                return Err(SourceCheckError::UnrestrictedTarget);
            }
            self.module.traits.push((declaration, item.span_id));
            return Ok(());
        }
        self.module.schemas.push(declaration);
        self.module
            .schema_capability
            .push(Capability::from_marker(unrestricted));
        self.module.schema_wrap.push(false);
        self.module.schema_domain_has.push(None);
        Ok(())
    }

    /// A `has` block of methods with no fields, no parameters and no provisions.
    fn is_trait(&self, declaration: &TypeDeclaration) -> bool {
        let TypeBody::Has(members) = &declaration.body.value else {
            return false;
        };
        let method = |member: &Spanned<TypeMember>| matches!(&member.value, TypeMember::Binding(binding) if diagnostics::is_method(binding));
        !members.is_empty() && members.iter().all(method)
    }

    /// Whether the method being checked belongs to a linear domain.
    pub(super) fn receiver_is_domain(&self) -> bool {
        let Some(SourceType::Aggregate { id }) = self.proof.receiver else {
            return false;
        };
        let schema = self.module.instances[id as usize].schema();
        self.module.layouts[schema].schema.kind == AggregateKind::Domain
    }

    /// The schema index of the domain or union called `name`, declared earlier: the type a
    /// `has` block of that name gives methods to.
    pub(super) fn paired_schema(&self, name: &str) -> Option<usize> {
        self.module.schemas.iter().position(|declaration| {
            declaration.name.value == name
                && matches!(
                    declaration.body.value,
                    TypeBody::Domain(_) | TypeBody::Is(_)
                )
        })
    }

    /// `Word is in 0 to 255`: a name for an integer type. Only a plain range with a
    /// written `in` is a domain; refinements and parameters are not supported.
    fn collect_domain(
        &mut self,
        declaration: &'a TypeDeclaration,
        domain: &'a Spanned<TypeDomain>,
        wrap: bool,
        unrestricted: bool,
    ) -> Result<()> {
        // A domain may take type parameters that only tag it (`Count(unit)`): the instances of
        // different arguments are different types of the same range.
        if domain.in_keyword.is_none() {
            return Err(SourceCheckError::Unsupported);
        }
        let body = self.span(declaration.body.span_id)?;
        if !body
            .strip_prefix("is")
            .is_some_and(|rest| rest.starts_with(char::is_whitespace))
        {
            return Err(SourceCheckError::Malformed);
        }
        self.charge(1)?;
        // Every named domain is a distinct nominal type of its own, copyable only by
        // `#unrestricted`; `#wrap` decides how an out-of-range value converts into it.
        self.module.schemas.push(declaration);
        self.module
            .schema_capability
            .push(Capability::from_marker(unrestricted));
        self.module.schema_wrap.push(wrap);
        self.module.schema_domain_has.push(None);
        Ok(())
    }

    pub(super) fn validate_layouts(&mut self) -> diagnostics::DiagnosticResult<()> {
        let mut edges = Vec::new();
        for schema in 0..self.module.layouts.len() {
            let count = self.module.layouts[schema].schema.fields.len();
            self.charge(count)?;
            let mut dependencies = Vec::new();
            for index in 0..count {
                let ty = self.module.layouts[schema].schema.fields[index].ty;
                self.dependencies(ty, &mut dependencies)?;
            }
            let count = self.module.layouts[schema].schema.variants.len();
            for index in 0..count {
                let ty = self.module.layouts[schema].schema.variants[index];
                self.dependencies(ty, &mut dependencies)?;
            }
            edges.push(dependencies);
        }
        self.check_cycles(&edges)?;
        self.validate_sums()?;
        for schema in 0..self.module.schemas.len() {
            self.validate_schema_layout(schema)
                .map_err(|error| self.failure(error, self.module.schemas[schema].body.span_id))?;
        }
        self.proof.finish_layout_validation();
        self.replay.finish_layout_validation();
        Ok(())
    }

    fn validate_schema_layout(&mut self, schema: usize) -> Result<()> {
        let scope = self.schema_scope(schema)?;
        self.charge(scope.len())?;
        let arguments = scope.iter().map(|(_, ty)| *ty).collect();
        let ty = self.instance(schema, arguments)?;
        // Declaration contracts are checked even when no body inspects a value.
        // Each schema validates its own immediate fields; no payload expansion
        // or variant selection is needed to validate nested declarations.
        if matches!(ty, SourceType::Aggregate { .. }) {
            let value = self.input_value(ty)?;
            self.materialize_children(value)?;
        }
        Ok(())
    }

    fn check_cycles(&mut self, edges: &[Vec<usize>]) -> Result<()> {
        for root in 0..edges.len() {
            self.charge(edges.len() + 1)?;
            let mut seen = vec![false; edges.len()];
            let mut pending = vec![root];
            while let Some(node) = pending.pop() {
                self.charge(1)?;
                if seen[node] {
                    continue;
                }
                seen[node] = true;
                self.charge(edges[node].len())?;
                for child in &edges[node] {
                    if *child == root {
                        return Err(SourceCheckError::Type);
                    }
                    pending.push(*child);
                }
            }
        }
        Ok(())
    }

    fn dependencies(&mut self, ty: SourceType, output: &mut Vec<usize>) -> Result<()> {
        self.charge(1)?;
        let mut pending = vec![ty];
        while let Some(ty) = pending.pop() {
            self.charge(1)?;
            if let SourceType::Aggregate { id }
            | SourceType::Sum { id }
            | SourceType::Array { id }
            | SourceType::Sequence { id } = ty
            {
                let instance = &self.module.instances[id as usize];
                let count = instance.arguments.len();
                self.charge(count + 1)?;
                if let CheckedSourceInstanceHead::Nominal { schema } =
                    self.module.instances[id as usize].head
                {
                    output.push(schema as usize);
                }
                pending.extend_from_slice(&self.module.instances[id as usize].arguments);
            }
        }
        Ok(())
    }

    /// The types an application of `schema` omitting trailing parameters is given: each
    /// parameter's default, resolved with the parameters before it in scope.
    pub(super) fn parameter_defaults(&mut self, schema: usize) -> Result<Vec<Option<SourceType>>> {
        let Some(parameters) = self.module.schemas[schema].parameters.as_ref() else {
            return Ok(Vec::new());
        };
        if parameters.value.iter().all(|parameter| parameter.value.default.is_none()) {
            return Ok(Vec::new());
        }
        let mut defaults = Vec::new();
        let mut scope: Vec<(&'a str, SourceType)> = Vec::new();
        for parameter in &parameters.value {
            defaults.push(match &parameter.value.default {
                Some(default) => Some(self.resolve_type(default, &scope)?),
                None => None,
            });
            scope.push((
                parameter.value.name.value.as_str(),
                SourceType::Generic {
                    index: scope.len() as u32,
                },
            ));
        }
        Ok(defaults)
    }

    /// The bounds of the type parameters of the function being checked, by generic index.
    pub(super) fn checked_type_bounds(&self) -> Vec<Option<String>> {
        if self.proof.bounds.is_empty() {
            return Vec::new();
        }
        // Several bounds on one parameter (the type's, then a method's) are joined by commas.
        (0..self.proof.generics.len())
            .map(|index| {
                let names: Vec<&str> = self
                    .proof
                    .bounds
                    .iter()
                    .filter(|(generic, _)| *generic as usize == index)
                    .filter_map(|(_, bound)| bound_trait_name(bound))
                    .collect();
                (!names.is_empty()).then(|| names.join(","))
            })
            .collect()
    }

    /// Each argument given for a bounded type parameter must provide its trait.
    pub(super) fn check_type_bounds(
        &mut self,
        bounds: &[Option<String>],
        arguments: &[SourceType],
    ) -> Result<()> {
        for (bound, argument) in bounds.iter().zip(arguments) {
            for trait_name in bound.iter().flat_map(|bound| bound.split(',')) {
                if !self.provides(*argument, trait_name)? {
                    let detail = format!("the type given here does not provide `{trait_name}`");
                    return Err(self.explain(SourceCheckError::Type, detail));
                }
            }
        }
        Ok(())
    }

    /// The trait each parameter of `schema` must provide, by name.

    pub(super) fn parameter_bounds(&self, schema: usize) -> Vec<Option<String>> {
        let Some(parameters) = self.module.schemas[schema].parameters.as_ref() else {
            return Vec::new();
        };
        let bounds: Vec<Option<String>> = parameters
            .value
            .iter()
            .map(|parameter| match parameter.value.annotation.as_ref().map(|a| &a.value) {
                Some(Annotation::Bound { bound, .. }) => bound_trait_name(bound).map(str::to_owned),
                _ => None,
            })
            .collect();
        if bounds.iter().all(Option::is_none) {
            return Vec::new();
        }
        bounds
    }

    /// The traits a type lists: the provisions of its own `has` block, or of the block that
    /// gives a domain or union its methods.
    pub(super) fn provided_traits(&self, schema: usize) -> Vec<String> {
        let mut traits = Vec::new();
        let blocks = [
            Some(self.module.schemas[schema]),
            self.module.schema_domain_has[schema],
        ];
        for declaration in blocks.into_iter().flatten() {
            let TypeBody::Has(members) = &declaration.body.value else {
                continue;
            };
            for member in members {
                match &member.value {
                    TypeMember::Provision(name) => traits.push(name.value.clone()),
                    TypeMember::ProvisionApplied(applied) => {
                        if let Some(name) = bound_trait_name(applied) {
                            traits.push(name.to_owned());
                        }
                    }
                    _ => {}
                }
            }
        }
        traits
    }

    /// Whether `ty`, supplied for a parameter bounded by `trait_name`, provides it: a nominal type
    /// listing the trait, or a type parameter of the caller bounded by the same trait.
    pub(super) fn provides(&mut self, ty: SourceType, trait_name: &str) -> Result<bool> {
        self.charge(self.proof.bounds.len() + 1)?;
        match ty {
            // `Self` of a trait's own signature stands for each provider in turn, and the provider is
            // checked against what the signature asks of it (`Copy`'s `CopyResult(Self)` needs `Release`).
            SourceType::Generic { index: 0 } if self.proof.abstract_trait.is_some() => Ok(true),
            SourceType::Generic { index } => Ok(self.proof.bounds.iter().any(|(generic, bound)| {
                *generic == index && bound_trait_name(bound) == Some(trait_name)
            })),
            SourceType::Aggregate { id } | SourceType::Sum { id } => {
                let schema = self.module.instances[id as usize].schema();
                // A type of this module is read from its declaration: a field of another type
                // can name it before its layout is compiled.
                let provided = if schema < self.module.schemas.len() {
                    self.provided_traits(schema)
                } else {
                    self.module.layouts[schema].schema.provides.clone()
                };
                self.charge(provided.len())?;
                Ok(provided.iter().any(|provided| provided == trait_name))
            }
            _ => Ok(false),
        }
    }

    /// `arguments` completed with the defaults of `schema`'s trailing parameters.
    pub(super) fn with_defaults(
        &mut self,
        schema: usize,
        mut arguments: Vec<SourceType>,
    ) -> Result<Vec<SourceType>> {
        let count = self.module.layouts[schema].schema.parameters.len();
        if arguments.len() >= count {
            return Ok(arguments);
        }
        let defaults = if schema < self.module.schemas.len() {
            self.parameter_defaults(schema)?
        } else {
            self.module.layouts[schema].schema.defaults.clone()
        };
        self.charge(count)?;
        while arguments.len() < count {
            let at = arguments.len();
            let Some(Some(default)) = defaults.get(at).copied() else {
                return Ok(arguments);
            };
            // A default naming earlier parameters is applied to the arguments given so far.
            arguments.push(self.substitute(default, &arguments.clone())?);
        }
        Ok(arguments)
    }

    pub(super) fn schema_scope(&mut self, schema: usize) -> Result<Vec<(&'a str, SourceType)>> {
        let mut scope = Vec::new();
        self.proof.bounds.clear();
        self.proof.numbers.clear();
        if let Some(parameters) = &self.module.schemas[schema].parameters {
            self.span(parameters.span_id)?;
            self.charge(parameters.value.len())?;
            if parameters.value.len() > self.parameters_max as usize {
                return Err(SourceCheckError::Budget);
            }
            for parameter in &parameters.value {
                self.child(parameters.span_id, parameter.span_id)?;
                self.child(parameter.span_id, parameter.name.span_id)?;
                let name = self.name(&parameter.name)?;
                // `element has Release` is allowed: the bound is read where methods use it.
                let bounded = matches!(
                    parameter.annotation.as_ref().map(|a| &a.value),
                    Some(Annotation::Bound { .. })
                );
                let number = parameter
                    .annotation
                    .as_ref()
                    .is_some_and(|a| Self::number_shaped(&a.value));
                if (parameter.annotation.is_some() && !bounded && !number)
                    || parameter.convention.is_some()
                {
                    return Err(SourceCheckError::Unsupported);
                }
                if number {
                    self.proof.numbers.push(numbers::NumberParam {
                        index: scope.len() as u32,
                        lower: I256::ZERO,
                        upper: I256::ZERO,
                        value: None,
                    });
                }
                if let Some(annotation) = &parameter.annotation {
                    self.child(parameter.span_id, annotation.span_id)?;
                }
                if let Some(default) = &parameter.default {
                    self.child(parameter.span_id, default.span_id)?;
                }
                // Inside the declaration the parameter provides what it is bounded by.
                if let Some(Annotation::Bound { bound, .. }) =
                    parameter.annotation.as_ref().map(|a| &a.value)
                {
                    self.proof.bounds.push((scope.len() as u32, bound));
                }
                self.charge_product(scope.len(), name.len() + 1)?;
                if scope.iter().any(|(other, _)| *other == name) {
                    return Err(SourceCheckError::Duplicate);
                }
                scope.push((
                    name,
                    SourceType::Generic {
                        index: scope.len() as u32,
                    },
                ));
            }
        }
        Ok(scope)
    }

    pub(super) fn metatype(&mut self, annotation: &'a Spanned<Annotation>) -> Result<bool> {
        self.span(annotation.span_id)?;
        let Annotation::Expression(expression) = &annotation.value else {
            return Ok(false);
        };
        self.child(annotation.span_id, expression.span_id)?;
        let Expr::Name(name) = &expression.value else {
            return Ok(false);
        };
        self.span(expression.span_id)?;
        if name != "Type" {
            return Ok(false);
        }
        self.charge(self.proof.names.len() + self.proof.generics.len())?;
        let shadowed = self.proof.names.iter().any(|(name, _)| *name == "Type")
            || self.proof.generics.contains(&"Type")
            || self.lookup_symbol("Type", false)?.is_some()
            || self.lookup_symbol("Type", true)?.is_some();
        if shadowed {
            return Ok(false);
        }
        self.token(expression.span_id, "Type")?;
        Ok(true)
    }

    pub(super) fn annotation_type(
        &mut self,
        annotation: &'a Spanned<Annotation>,
    ) -> Result<SourceType> {
        // Domain bounds can read scalar aliases whose values vary by world.
        // Their source pointer alone is not a semantic cache key.
        let cacheable = matches!(annotation.value, Annotation::Expression(_));
        let key = (annotation as *const _ as usize, false);
        if cacheable && let Some(ty) = self.cached_source_type(key)? {
            return Ok(ty);
        }
        self.charge(self.proof.generics.len())?;
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
        let ty = self.resolve_annotation(annotation, &scope)?;
        if cacheable {
            self.cache_source_type(key, ty)?;
        }
        Ok(ty)
    }

    pub(super) fn resolve_annotation(
        &mut self,
        annotation: &'a Spanned<Annotation>,
        scope: &[(&'a str, SourceType)],
    ) -> Result<SourceType> {
        self.span(annotation.span_id)?;
        match &annotation.value {
            Annotation::Expression(expression) => {
                self.child(annotation.span_id, expression.span_id)?;
                match self.resolve_type(expression, scope) {
                    Err(SourceCheckError::UnknownName) => {
                        let detail = "a name in this type annotation is not defined or imported".to_owned();
                        Err(self.explain(SourceCheckError::Unsupported, detail))
                    }
                    other => other,
                }
            }
            Annotation::Domain { .. } => Ok(self.bounds(annotation)?.ty),
            Annotation::Refined {
                base: Some(base), ..
            } => {
                let end = base.span_id;
                let start = annotation.span_id;
                self.child(start, end)?;
                match self.resolve_type(base, scope) {
                    Err(SourceCheckError::UnknownName) => {
                        let detail = "a name in this type annotation is not defined or imported".to_owned();
                        Err(self.explain(SourceCheckError::Unsupported, detail))
                    }
                    other => other,
                }
            }
            _ => Err(SourceCheckError::Unsupported),
        }
    }

    pub(super) fn resolve_type(
        &mut self,
        root: &'a Expression,
        scope: &[(&'a str, SourceType)],
    ) -> Result<SourceType> {
        let key = (root as *const _ as usize, true);
        if let Some(ty) = self.cached_source_type(key)? {
            return Ok(ty);
        }
        self.charge(1)?;
        let mut tasks = vec![(root, false, 0)];
        let mut values = Vec::new();
        while let Some((expression, visited, depth)) = tasks.pop() {
            self.charge(1)?;
            if depth > 64 {
                return Err(SourceCheckError::Budget);
            }
            self.span(expression.span_id)?;
            match &expression.value {
                Expr::Group(child) => {
                    self.group_syntax(expression, child)?;
                    self.charge(1)?;
                    tasks.push((child, false, depth + 1));
                }
                Expr::PrefixedType(prefix) if prefix.keyword.value == TypePrefixKind::Pointer => {
                    self.pointer_syntax(expression, prefix)?;
                    if visited {
                        let pointee = values.pop().ok_or(SourceCheckError::Malformed)?;
                        values.push(self.pointer_type(pointee)?);
                    } else {
                        self.charge(2)?;
                        tasks.push((expression, true, depth));
                        tasks.push((&prefix.target, false, depth + 1));
                    }
                }
                Expr::Sequence(sequence) => {
                    self.sequence_syntax(expression, sequence)?;
                    let count = sequence.elements.len();
                    if visited {
                        if values.len() < count {
                            return Err(SourceCheckError::Malformed);
                        }
                        let elements = values.split_off(values.len() - count);
                        values.push(self.sequence_type(elements)?);
                    } else {
                        self.charge(count + 1)?;
                        tasks.push((expression, true, depth));
                        for element in sequence.elements.iter().rev() {
                            tasks.push((element, false, depth + 1));
                        }
                    }
                }
                Expr::Array(array) if self.symbolic_extent(expression, array, scope)?.is_some() => {
                    // `[element; capacity]` with `capacity` a number parameter.
                    let number = self
                        .symbolic_extent(expression, array, scope)?
                        .ok_or(SourceCheckError::Malformed)?;
                    if visited {
                        let element = values.pop().ok_or(SourceCheckError::Malformed)?;
                        values.push(self.sized_array(element, number)?);
                    } else {
                        self.charge(2)?;
                        tasks.push((expression, true, depth));
                        tasks.push((&array.elements[0], false, depth + 1));
                    }
                }
                Expr::Array(array) => {
                    let extent = self.array_type_extent(expression, array)?;
                    if visited {
                        let element = values.pop().ok_or(SourceCheckError::Malformed)?;
                        values.push(self.array_type(element, extent)?);
                    } else {
                        self.charge(2)?;
                        tasks.push((expression, true, depth));
                        tasks.push((&array.elements[0], false, depth + 1));
                    }
                }
                Expr::Name(name) => {
                    self.token(expression.span_id, name)?;
                    self.charge_product(scope.len(), name.len() + 1)?;
                    if let Some((_, ty)) = scope.iter().find(|(other, _)| *other == name) {
                        values.push(*ty);
                    } else if name == "Self"
                        && let Some(receiver) = self.proof.receiver
                    {
                        // Inside a type's methods, `Self` is the receiver type.
                        values.push(receiver);
                    } else if let Some(alias) = self.alias_type(name)? {
                        values.push(alias);
                    } else if name == "Text" && self.lookup_symbol(name, false)?.is_none() {
                        // Compile-time text: a field of it makes a compile-time-only record.
                        values.push(SourceType::Text);
                    } else if let Some(schema) = self.lookup_symbol(name, false)? {
                        let args = self.with_defaults(schema, Vec::new())?;
                        values.push(self.instance(schema, args)?);
                    } else if self.lookup_symbol(name, true)?.is_some()
                        || self.constant_exists(name)?
                    {
                        // A constant where a type argument goes: `ArrayView(Byte, size)`.
                        values.push(self.number_argument(expression)?);
                    } else {
                        return Err(SourceCheckError::UnknownName);
                    }
                }
                Expr::Member {
                    base,
                    name: variant,
                } => {
                    // `Union.Variant`: the variant a union declared or references.
                    self.child(expression.span_id, base.span_id)?;
                    self.child(expression.span_id, variant.span_id)?;
                    let Expr::Name(union) = &base.value else {
                        return Err(SourceCheckError::Unsupported);
                    };
                    self.token(base.span_id, union)?;
                    let variant = self.name(variant)?;
                    let schema = self.schema(union)?;
                    values.push(self.qualified_variant(schema, variant)?);
                }
                Expr::Call { callee, arguments } => {
                    self.child(expression.span_id, callee.span_id)?;
                    self.child(expression.span_id, arguments.span_id)?;
                    let Expr::Name(name) = &callee.value else {
                        return Err(SourceCheckError::Type);
                    };
                    self.token(callee.span_id, name)?;
                    self.charge_product(scope.len(), name.len() + 1)?;
                    if scope.iter().any(|(other, _)| *other == name) {
                        return Err(SourceCheckError::Type);
                    }
                    let schema = self.schema(name)?;
                    self.type_arguments(expression, arguments, visited, depth, &mut tasks)?;
                    if visited {
                        let count = arguments.value.len();
                        if values.len() < count {
                            return Err(SourceCheckError::Malformed);
                        }
                        let args = values.split_off(values.len() - count);
                        let args = self.with_defaults(schema, args)?;
                        self.check_number_arguments(schema, &args)?;
                        let bounds = if schema < self.module.schemas.len() {
                            self.parameter_bounds(schema)
                        } else {
                            self.module.layouts[schema].schema.bounds.clone()
                        };
                        self.check_type_bounds(&bounds, &args)?;
                        values.push(self.instance(schema, args)?);
                    }
                }
                // A range where a type argument goes: `Range(domain of Byte)`, `Range(1 to 9)`.
                Expr::Unary { operator, operand } if operator.value == UnaryOperator::DomainOf => {
                    self.child(expression.span_id, operand.span_id)?;
                    if visited {
                        let ty = values.pop().ok_or(SourceCheckError::Malformed)?;
                        values.push(self.domain_type(ty)?);
                    } else {
                        self.charge(2)?;
                        tasks.push((expression, true, depth));
                        tasks.push((operand, false, depth + 1));
                    }
                }
                Expr::Range {
                    start,
                    to_keyword,
                    end,
                } => values.push(self.range_type(expression, start, *to_keyword, end)?),
                // A number where a type argument goes: `ArrayView(Byte, 16)`.
                Expr::Literal(LiteralKind::Integer) | Expr::Unary { .. } => {
                    values.push(self.number_argument(expression)?);
                }
                _ => return Err(SourceCheckError::Unsupported),
            }
        }
        if values.len() != 1 {
            return Err(SourceCheckError::Malformed);
        }
        self.cache_source_type(key, values[0])?;
        Ok(values[0])
    }

    fn type_arguments(
        &mut self,
        expression: &'a Expression,
        arguments: &'a Spanned<Vec<Spanned<Argument>>>,
        visited: bool,
        depth: u32,
        tasks: &mut Vec<(&'a Expression, bool, u32)>,
    ) -> Result<()> {
        if visited {
            return Ok(());
        }
        self.charge(arguments.value.len() + 1)?;
        tasks.push((expression, true, depth));
        for argument in arguments.value.iter().rev() {
            self.child(arguments.span_id, argument.span_id)?;
            let Argument::Positional(value) = &argument.value else {
                return Err(SourceCheckError::Type);
            };
            self.child(argument.span_id, value.span_id)?;
            tasks.push((value, false, depth + 1));
        }
        Ok(())
    }

    pub(super) fn schema(&mut self, name: &'a str) -> Result<usize> {
        self.lookup_symbol(name, false)?
            .ok_or(SourceCheckError::UnknownName)
    }

    pub(super) fn instance(
        &mut self,
        schema: usize,
        arguments: Vec<SourceType>,
    ) -> Result<SourceType> {
        self.intern_instance(
            CheckedSourceInstanceHead::Nominal {
                schema: schema as u32,
            },
            arguments,
        )
    }

    pub(super) fn intern_instance(
        &mut self,
        head: CheckedSourceInstanceHead,
        arguments: Vec<SourceType>,
    ) -> Result<SourceType> {
        let count = match head {
            CheckedSourceInstanceHead::Nominal { schema } => {
                self.module.layouts[schema as usize].schema.parameters.len()
            }
            CheckedSourceInstanceHead::Array { .. }
            | CheckedSourceInstanceHead::Slice
            | CheckedSourceInstanceHead::Pointer => 1,
            CheckedSourceInstanceHead::Sized => 2,
            CheckedSourceInstanceHead::Sequence => {
                if arguments.is_empty() {
                    return Ok(SourceType::Unit);
                }
                arguments.len()
            }
        };
        if count != arguments.len() {
            return Err(SourceCheckError::Type);
        }
        if head == CheckedSourceInstanceHead::Sized
            && let Some(applied) = self.apply_sized(&arguments)?
        {
            return Ok(applied);
        }
        // An `#unrestricted` generic type may hold only unrestricted things, so a linear
        // type cannot be passed to it. An argument still generic here is checked when the
        // enclosing signature is instantiated with concrete types.
        if let CheckedSourceInstanceHead::Nominal { schema } = head
            && self.module.layouts[schema as usize].schema.capability == Capability::Unrestricted
            && let Some(offender) = arguments.iter().find(|argument| {
                !matches!(argument, SourceType::Generic { .. })
                    && self.capability_of(**argument) != Capability::Unrestricted
            })
        {
            let detail = format!(
                "type argument `{}` cannot be passed to `{}`: it is `#unrestricted`, so everything it holds must be unrestricted too",
                self.type_label(*offender),
                self.module.layouts[schema as usize].schema.name,
            );
            return Err(self.explain(SourceCheckError::UnrestrictedArgument, detail));
        }
        self.charge_product(
            (usize::BITS - self.module.instance_index.len().leading_zeros()) as usize + 1,
            arguments.len() + 1,
        )?;
        let found = self.module.instance_index.binary_search_by(|id| {
            let instance = &self.module.instances[*id];
            instance
                .head
                .cmp(&head)
                .then_with(|| instance.arguments.cmp(&arguments))
        });
        match found {
            Ok(index) => Ok(self.head_type(head, self.module.instance_index[index] as u32)),
            Err(index) => {
                self.charge(self.module.instance_index.len() + 1)?;
                let id = self.module.instances.len();
                self.module.instances.push(Instance { head, arguments });
                self.module.instance_index.insert(index, id);
                Ok(self.head_type(head, id as u32))
            }
        }
    }

    pub(super) fn instance_fields(&mut self, id: u32) -> Result<Vec<SourceType>> {
        let schema = self.module.instances[id as usize].schema();
        let count = self.module.layouts[schema].schema.fields.len();
        self.charge(count + self.module.instances[id as usize].arguments.len())?;
        let arguments = self.module.instances[id as usize].arguments.clone();
        let mut output = Vec::new();
        for index in 0..count {
            let template = self.module.layouts[schema].schema.fields[index].ty;
            output.push(self.substitute(template, &arguments)?);
        }
        Ok(output)
    }

    pub(super) fn compatible(&mut self, expected: SourceType, actual: SourceType) -> Result<()> {
        let mut pending = vec![(expected, actual)];
        while let Some((expected, actual)) = pending.pop() {
            self.charge(1)?;
            if expected == actual {
                continue;
            }
            match (expected, actual) {
                (SourceType::Array { .. }, SourceType::Array { .. }) => {
                    let (left, count_left) = self.array_layout(expected)?;
                    let (right, count_right) = self.array_layout(actual)?;
                    if count_left != count_right || self.sized_mismatch(expected, actual) {
                        return Err(SourceCheckError::Type);
                    }
                    pending.push((left, right));
                }
                (SourceType::Sequence { id: left }, SourceType::Sequence { id: right }) => {
                    let left = &self.module.instances[left as usize].arguments;
                    let right = &self.module.instances[right as usize].arguments;
                    prepay(&mut self.remaining, left.len())?;
                    if left.len() != right.len() {
                        return Err(SourceCheckError::Type);
                    }
                    pending.extend(left.iter().copied().zip(right.iter().copied()));
                }
                _ => self.compatible_leaf(expected, actual)?,
            }
        }
        Ok(())
    }

    fn compatible_leaf(&mut self, expected: SourceType, actual: SourceType) -> Result<()> {
        if let SourceType::Sum { id } = expected {
            let variants = self.sum_variants(id)?;
            if let SourceType::Sum { id } = actual {
                let actual = self.sum_variants(id)?;
                self.charge_product(actual.len(), variants.len())?;
                if actual.iter().all(|variant| variants.contains(variant)) {
                    return Ok(());
                }
            } else if variants.contains(&actual) {
                return Ok(());
            }
        }
        if let (SourceType::Integer { lower, upper }, SourceType::Integer { lower: a, upper: b }) =
            (expected, actual)
            && a >= lower
            && b <= upper
        {
            return Ok(());
        }
        Err(SourceCheckError::Type)
    }

    pub(super) fn scalar(&self, value: Value) -> Result<()> {
        if matches!(value.ty, SourceType::Integer { .. }) {
            Ok(())
        } else {
            Err(SourceCheckError::Type)
        }
    }

    pub(super) fn fresh_value(&mut self, ty: SourceType) -> Result<Value> {
        self.charge(1)?;
        let identity = self.proof.next_identity;
        self.proof.next_identity = identity.checked_add(1).ok_or(SourceCheckError::Budget)?;
        let (lower, upper) = match ty {
            SourceType::Integer { lower, upper } => (lower, upper),
            _ => (I256::ZERO, I256::ZERO),
        };
        if matches!(ty, SourceType::Integer { .. }) {
            self.charge(1)?;
            self.proof.scalars.push((identity, lower, upper));
        }
        let owner = if matches!(
            ty,
            SourceType::Generic { .. }
                | SourceType::Aggregate { .. }
                | SourceType::Sum { .. }
                | SourceType::Array { .. }
                | SourceType::Sequence { .. }
        ) {
            self.charge_flow()?;
            let owner = self.proof.owners.len();
            let unrestricted = self.capability_of(ty) == Capability::Unrestricted;
            let view = self.is_view(ty);
            self.proof.owners.push(Owner {
                live: false,
                unrestricted,
                passing: calls::Passing::Owner,
                children: Vec::new(),
                slots: Vec::new(),
                moved: Vec::new(),
                active: None,
                materialized: false,
                drained: false,
                lent: Vec::new(),
                view,
                extent: None,
                claims: Vec::new(),
            });
            self.bind_sized_extent(owner, ty);
            Some(owner)
        } else {
            None
        };
        Ok(Value {
            lower,
            upper,
            identity: Some(identity),
            ty,
            owner,
            available: true,
            sum: None,
            structural: match ty {
                SourceType::Array { .. } | SourceType::Sequence { .. } => Some(ty),
                _ => None,
            },
            integer_domain: match ty {
                SourceType::Integer { lower, upper } => Some((lower, upper)),
                _ => None,
            },
            affine: matches!(ty, SourceType::Integer { .. }).then_some((identity, I256::ZERO)),
            borrow: None,
        })
    }

    pub(super) fn input_value(&mut self, ty: SourceType) -> Result<Value> {
        self.fresh_value(ty)
    }

    /// Reveal only this place, retaining the owner's original identity.
    pub(super) fn inspect_value(&mut self, value: Value) -> Result<Value> {
        let mut value = self.owner_view(value)?;
        if let SourceType::Sum { id } = value.ty {
            let variants = self.sum_variants(id)?;
            let selected = self.choose(variants.len())?;
            let owner = value.owner.ok_or(SourceCheckError::Type)?;
            assert!(self.proof.owners[owner].active.is_none());
            self.proof.owners[owner].active = Some(variants[selected]);
            value.ty = variants[selected];
            value.sum = value.sum.or(Some(id));
            // Known to be a member that may be dropped (`Lost` of `Held or Lost`), the value
            // owes nothing in this world.
            // A union whose variants all carry nothing has no member to relax: it is linear
            // unless marked `#unrestricted`, whichever variant is held.
            let carries = variants.iter().any(|variant| {
                matches!(variant, SourceType::Aggregate { id }
                    if !self.module.layouts[self.module.instances[*id as usize].schema()].schema.fields.is_empty())
            });
            if carries && self.capability_of(variants[selected]) == Capability::Unrestricted {
                self.proof.owners[owner].unrestricted = true;
                self.proof.owners[owner].live = false;
            }
        }
        Ok(value)
    }

    pub(super) fn owner_view(&mut self, mut value: Value) -> Result<Value> {
        self.charge(1)?;
        if let Some(owner) = value.owner
            && let Some(active) = self.proof.owners[owner].active
        {
            if let SourceType::Sum { id } = value.ty {
                value.sum = value.sum.or(Some(id));
            }
            value.ty = active;
        }
        Ok(value)
    }

    /// Allocate one level of stable child origins, never alternative payloads.
    pub(super) fn materialize_children(&mut self, value: Value) -> Result<()> {
        let owner = value.owner.ok_or(SourceCheckError::Type)?;
        self.charge(1)?;
        if self.proof.owners[owner].materialized {
            return Ok(());
        }
        let SourceType::Aggregate { id } = value.ty else {
            return Err(SourceCheckError::Type);
        };
        let fields = self.instance_fields(id)?;
        self.charge(fields.len())?;
        let mut children = Vec::new();
        for ty in fields {
            children.push(self.input_value(ty)?);
        }
        self.field_requirements(id, &mut children, true)?;
        // What a borrowed record holds is borrowed with it, never owned by the borrower.
        let passing = self.proof.owners[owner].passing;
        if passing.borrows() {
            for child in &children {
                if let Some(child_owner) = child.owner {
                    self.proof.owners[child_owner].passing = passing;
                }
            }
        }
        self.mark_borrowed_children(id, &children);
        self.set_input_extents(id, &children);
        self.proof.owners[owner].children = children;
        self.proof.owners[owner].materialized = true;
        Ok(())
    }

    /// Start an ownership obligation; unrestricted owners never have one.
    pub(super) fn obligate(&mut self, owner: usize) {
        let owner = &mut self.proof.owners[owner];
        owner.live = !owner.unrestricted && !owner.passing.borrows() && !owner.view;
    }

    /// A short name for `ty` in a message: the nominal type's name, else its general kind.
    pub(super) fn type_label(&self, ty: SourceType) -> String {
        match ty {
            SourceType::Aggregate { id } | SourceType::Sum { id } => self
                .module
                .instances
                .get(id as usize)
                .and_then(|instance| self.module.layouts.get(instance.schema()))
                .map_or_else(|| "a type".to_owned(), |layout| layout.schema.name.clone()),
            SourceType::Integer { .. } => "an integer".to_owned(),
            SourceType::Unit => "unit".to_owned(),
            SourceType::Text => "text".to_owned(),
            SourceType::Generic { .. } => "a generic type".to_owned(),
            SourceType::Array { .. } => "an array".to_owned(),
            SourceType::Sequence { .. } => "a sequence".to_owned(),
            SourceType::Pointer { .. } => "a pointer".to_owned(),
        }
    }

    /// What a value of `ty` may do. This is the single place that decides: a type is linear
    /// unless it is a scalar or a nominal type that opted in with `#unrestricted`. Anything
    /// not positively known to be unrestricted stays linear.
    pub(super) fn capability_of(&self, ty: SourceType) -> Capability {
        match ty {
            SourceType::Integer { .. } | SourceType::Unit | SourceType::Text => {
                Capability::Unrestricted
            }
            SourceType::Aggregate { id } | SourceType::Sum { id } => self
                .module
                .instances
                .get(id as usize)
                .and_then(|instance| self.module.layouts.get(instance.schema()))
                .map_or(Capability::Linear, |layout| layout.schema.capability),
            // A pointer is a capability to storage, never a plain number; arrays,
            // sequences and generic values would need conditional rules not defined yet.
            SourceType::Pointer { .. }
            | SourceType::Generic { .. }
            | SourceType::Array { .. }
            | SourceType::Sequence { .. } => Capability::Linear,
        }
    }

    /// An unrestricted record is a plain value: it may be copied and discarded, so every
    /// field must be unrestricted too.
    pub(super) fn validate_unrestricted(&mut self) -> diagnostics::DiagnosticResult<()> {
        // Only marked schemas are charged: an unmarked module does no extra work here.
        for schema in 0..self.module.schemas.len() {
            if self.module.layouts[schema].schema.capability != Capability::Unrestricted {
                continue;
            }
            let context = self.module.schemas[schema].body.span_id;
            self.charge(self.module.layouts[schema].schema.fields.len() + 1)
                .map_err(|error| self.failure(error, context))?;
            if let Some(error) = self.exclusive_field_offender(schema) {
                return Err(self.failure(error, context));
            }
            let generic = !self.module.layouts[schema].schema.parameters.is_empty();
            let holds = |ty: SourceType| {
                // A generic field is as unrestricted as its argument, which every
                // instantiation of an `#unrestricted` type is required to be.
                (generic && matches!(ty, SourceType::Generic { .. }))
                    || self.capability_of(ty) == Capability::Unrestricted
            };
            let layout = &self.module.layouts[schema].schema;
            let offender = layout
                .fields
                .iter()
                // A `ref` field holds no value, so it never stops its record being unrestricted.
                .map(|field| {
                    let ty = if field.borrow.is_some() { SourceType::Unit } else { field.ty };
                    (format!("`{}`", field.name), ty)
                })
                .chain(
                    layout
                        .variants
                        .iter()
                        .map(|variant| ("one of its variants".to_owned(), *variant)),
                )
                .find(|(_, ty)| !holds(*ty));
            // A variant the union declared itself holds its payload: an `#unrestricted` union
            // may not carry a linear one.
            let offender = offender.or_else(|| {
                let layout = &self.module.layouts[schema].schema;
                layout.variants.iter().find_map(|variant| {
                    let SourceType::Aggregate { id } = *variant else { return None };
                    let variant_schema = self.module.instances.get(id as usize)?.schema();
                    if !self.is_declared_variant(variant_schema) {
                        return None;
                    }
                    let variant_layout = &self.module.layouts[variant_schema].schema;
                    variant_layout.fields.iter().find(|field| !holds(field.ty)).map(|field| {
                        (format!("the payload of `{}`", variant_layout.name), field.ty)
                    })
                })
            });
            if let Some((name, ty)) = offender {
                let detail = format!(
                    "`{}` is `#unrestricted` but {name} holds {}, which is not unrestricted",
                    layout.name,
                    self.type_label(ty),
                );
                let error = self.explain(SourceCheckError::UnrestrictedField, detail);
                return Err(self.failure(error, context));
            }
        }
        Ok(())
    }

    pub(super) fn release(&mut self, value: Value) -> Result<()> {
        self.charge(1)?;
        if let Some(owner) = value.owner {
            self.charge_flow()?;
            self.proof.owners[owner].live = false;
        }
        Ok(())
    }

    pub(super) fn assume(
        &mut self,
        operator: BinaryOperator,
        left: Value,
        right: Value,
    ) -> Result<()> {
        self.scalar(left)?;
        self.scalar(right)?;
        let possible = match operator {
            BinaryOperator::Equal => left.lower <= right.upper && right.lower <= left.upper,
            BinaryOperator::GreaterOrEqual => left.upper >= right.lower,
            BinaryOperator::Greater => left.upper > right.lower,
            BinaryOperator::LessOrEqual => left.lower <= right.upper,
            BinaryOperator::Less => left.lower < right.upper,
            _ => return Err(SourceCheckError::Unsupported),
        };
        if !possible {
            return Err(SourceCheckError::Type);
        }
        let left = left.identity.ok_or(SourceCheckError::Type)?;
        let right = right.identity.ok_or(SourceCheckError::Type)?;
        self.charge(1)?;
        self.proof.facts.push((left, operator, right));
        self.consistent_facts()?;
        Ok(())
    }

    pub(super) fn field_requirements(
        &mut self,
        id: u32,
        values: &mut [Value],
        assume: bool,
    ) -> Result<()> {
        let schema = self.module.instances[id as usize].schema();
        self.charge(self.module.layouts[schema].constraints.len())?;
        let mut constraints = self.module.layouts[schema].constraints.clone();
        // A claim about the size of a parameter is about the size of the argument given.
        let arguments = self.module.instances[id as usize].arguments.clone();
        if !arguments.is_empty() {
            for constraint in &mut constraints {
                constraint.right = self.substitute_term(&constraint.right, &arguments)?;
                if let Some(term) = constraint.left_term.take() {
                    constraint.left_term = Some(self.substitute_term(&term, &arguments)?);
                }
            }
        }
        self.constraints(&constraints, values, assume)
    }

    pub(super) fn destructure_tasks(
        &mut self,
        pattern: &'a DestructureStatement,
        parent: SpanId,
        pending: &mut Vec<EvalTask<'a>>,
    ) -> Result<()> {
        self.child(parent, pattern.pattern.span_id)?;
        self.child(parent, pattern.operator.span_id)?;
        self.token(pattern.operator.span_id, ":=")?;
        if pattern.operator.value != BindingOperator::ColonEquals {
            return Err(SourceCheckError::Malformed);
        }
        self.child(parent, pattern.value.span_id)?;
        pending.push(EvalTask::Destructure(&pattern.pattern));
        pending.push(EvalTask::Expression(&pattern.value, None, false));
        Ok(())
    }

    /// `when condition then A else B`. Each world takes one feasible branch, with the condition
    /// (or its negation) assumed for the rest of that world, so a bound proved by the condition is
    /// available inside the branch and after it on that path. The condition is one relation of
    /// numbers, with `not` and `is 0` seen through, or one variant test.
    fn conditional_tasks(
        &mut self,
        expression: &'a Expression,
        case: &'a ConditionalExpression,
        expected: Option<SourceType>,
        visited: bool,
        pending: &mut Vec<EvalTask<'a>>,
        values: &mut Vec<Value>,
    ) -> Result<()> {
        // A condition of several comparisons (`a < 0 or a >= b`) is tried term by term.
        let several = guards::conditions(&case.condition)
            .is_some_and(|(_, leaves)| leaves.len() > 1);
        if guards::variant_test(&case.condition).is_none()
            && (several || chains::mentions_variant(&case.condition))
        {
            return self.compound_conditional(expression, case, expected, pending);
        }
        if let Some((negated, test)) = guards::variant_test(&case.condition) {
            return self.variant_conditional(
                expression, case, negated, test, expected, visited, pending, values,
            );
        }
        let (_, leaves) =
            guards::conditions(&case.condition).ok_or(SourceCheckError::Unsupported)?;
        let [leaf] = leaves[..] else {
            return Err(SourceCheckError::Unsupported);
        };
        let relation = leaf.relation;
        if !visited {
            self.child(expression.span_id, case.condition.span_id)?;
            self.child(expression.span_id, case.then_branch.span_id)?;
            self.child(expression.span_id, case.else_branch.span_id)?;
            self.token(case.when_keyword, "when")?;
            self.token(case.then_keyword, "then")?;
            self.token(case.else_keyword, "else")?;
            self.condition_syntax(&case.condition)?;
            self.charge(3)?;
            pending.push(EvalTask::Expression(expression, expected, true));
            pending.push(leaf.right.task());
            pending.push(EvalTask::Expression(leaf.left, None, false));
            return Ok(());
        }
        let right = self.pop_operand(relation, values)?;
        let left = values.pop().ok_or(SourceCheckError::Malformed)?;
        // An unrestricted domain compares as the number it holds.
        let left = self.condition_number(left)?;
        self.scalar(left)?;
        let left = self.ensure_identity(left)?;
        let then_possible = relation.possible(left, right);
        let else_possible = relation.negate().possible(left, right);
        // A branch the condition rules out never runs, but is still checked and reported.
        for (possible, branch, detail) in [
            (
                then_possible,
                &case.then_branch,
                "this branch never runs: the condition is never true",
            ),
            (
                else_possible,
                &case.else_branch,
                "this branch never runs: the condition is always true",
            ),
        ] {
            if !possible {
                self.check_unreachable(branch.span_id, detail, |this| {
                    this.evaluate(branch, expected)
                })?;
            }
        }
        let mut branches = Vec::new();
        if then_possible {
            branches.push(true);
        }
        if else_possible {
            branches.push(false);
        }
        let selected = self.choose(branches.len())?;
        if branches[selected] {
            self.assume_relation(relation, left, right)?;
            pending.push(EvalTask::Expression(&case.then_branch, expected, false));
        } else {
            self.assume_relation(relation.negate(), left, right)?;
            pending.push(EvalTask::Expression(&case.else_branch, expected, false));
        }
        Ok(())
    }

    pub(super) fn comparison_possible(operator: BinaryOperator, left: Value, right: Value) -> bool {
        match operator {
            BinaryOperator::Equal => left.lower <= right.upper && right.lower <= left.upper,
            BinaryOperator::GreaterOrEqual => left.upper >= right.lower,
            BinaryOperator::Greater => left.upper > right.lower,
            BinaryOperator::LessOrEqual => left.lower <= right.upper,
            BinaryOperator::Less => left.lower < right.upper,
            _ => false,
        }
    }

    /// A number the facts can talk about needs an identity; a literal or computed interval
    /// gets one standing for exactly the numbers it may be.
    pub(super) fn ensure_identity(&mut self, value: Value) -> Result<Value> {
        if value.identity.is_some() {
            return Ok(value);
        }
        let fresh = self.fresh_value(SourceType::Integer {
            lower: value.lower,
            upper: value.upper,
        })?;
        Ok(Value {
            identity: fresh.identity,
            ..value
        })
    }

    pub(super) fn evaluate(
        &mut self,
        root: &'a Expression,
        expected: Option<SourceType>,
    ) -> Result<Value> {
        self.charge(1)?;
        let mut pending = vec![EvalTask::Expression(root, expected, false)];
        let mut values = Vec::new();
        while let Some(task) = pending.pop() {
            self.charge(1)?;
            if self.proof.returned {
                // A `return` ran: the path ends with its value. Tasks are dropped down to the
                // end of a hypothetical check, if one is open: that check is a detached world
                // for an arm no real path takes, and its `return` ends only itself.
                let value = values.pop().ok_or(SourceCheckError::Malformed)?;
                pending.push(task);
                let mut boundary = None;
                while let Some(next) = pending.pop() {
                    if matches!(next, EvalTask::ValidationEnd(..)) {
                        boundary = Some(next);
                        break;
                    }
                }
                match boundary {
                    Some(end) => {
                        self.proof.returned = false;
                        values.push(value);
                        pending.push(end);
                        continue;
                    }
                    None => {
                        values.clear();
                        values.push(value);
                        break;
                    }
                }
            }
            let (expression, expected, visited) = match task {
                EvalTask::Expression(expression, expected, visited) => {
                    (expression, expected, visited)
                }
                other => {
                    self.evaluate_action(other, &mut pending, &mut values)?;
                    continue;
                }
            };
            self.span(expression.span_id)?;
            self.culprit = Some(expression.span_id);
            match &expression.value {
                // `#todo`: a value of whatever type the context expects, about which nothing is
                // known. Accepted with a note; running it is the consumer's error.
                Expr::Hole => {
                    let ty = expected.ok_or(SourceCheckError::Unsupported)?;
                    let facts = self.fact_snapshot();
                    self.note_with(
                        SourceCheckNoteKind::Hole,
                        expression.span_id,
                        "`#todo` stands for a value that is not written yet".to_owned(),
                        facts,
                    );
                    values.push(self.fresh_value(ty)?);
                }
                // A compile-time query (`#size(x)`, `#target.triple`) is not defined yet: what
                // they would answer depends on layouts and a target the checker is not given.
                Expr::Query { .. } => return Err(SourceCheckError::Unsupported),
                // `#unchecked(value)`: the operation `value` performs is accepted unchecked. The
                // directive must have been used by it, or it accepts nothing.
                Expr::Call { .. } if is_unchecked(expression) => {
                    if visited {
                        let value = unchecked_operand(expression).ok_or(SourceCheckError::Malformed)?;
                        let used = self.release_acceptance(value.span_id)?;
                        if !used {
                            let detail = "`#unchecked(...)` wraps an operation that is checked; remove it".to_owned();
                            return Err(self.explain(SourceCheckError::UnneededAcceptance, detail));
                        }
                    } else {
                        let value = self.unchecked_syntax(expression)?;
                        self.acceptances.push((value.span_id, false));
                        self.charge(2)?;
                        pending.push(EvalTask::Expression(expression, expected, true));
                        pending.push(EvalTask::Expression(value, expected, false));
                    }
                }
                Expr::Call { callee, arguments }
                    if matches!(&callee.value, Expr::Query { name, .. } if matches!(name.value.as_str(), "lower" | "upper" | "size" | "alignment")) =>
                {
                    let value = self.bound_query(expression, callee, arguments)?;
                    // A query that is one number is a constant to whoever records the body. One
                    // over a type parameter (`#lower(r)` in `Range(r)`) is one number only in the
                    // world checked here, not in every instance, so it is not.
                    let generic = arguments.value.iter().any(|argument| {
                        matches!(&argument.value, Argument::Positional(Spanned { value: Expr::Name(name), .. })
                            if self.proof.generics.iter().any(|generic| *generic == name.as_str()))
                    });
                    if value.lower == value.upper && !generic {
                        self.query_values.push((expression.span_id, value.lower));
                    }
                    values.push(value);
                }
                Expr::Call { callee, .. } if matches!(callee.value, Expr::Query { .. }) => {
                    return Err(SourceCheckError::Unsupported);
                }
                // `#target.triple`, `#target.register_bits`: facts about the machine, known now.
                Expr::Member { base, name } if matches!(base.value, Expr::Query { .. }) => {
                    values.push(self.target_fact(expression, base, name)?);
                }
                Expr::Cast {
                    value,
                    as_keyword,
                    target,
                } => {
                    self.cast_syntax(expression, value, *as_keyword, target)?;
                    if visited {
                        let value = values.pop().ok_or(SourceCheckError::Malformed)?;
                        let target = self.cast_target(target)?;
                        values.push(self.reconstruct(expression.span_id, value, target)?);
                    } else if let Some(view) = self.view_cast(expression, value, target)? {
                        values.push(view);
                    } else if let Some(address) = self.address_cast(expression.span_id, value, target)? {
                        values.push(address);
                    } else {
                        self.charge(2)?;
                        pending.push(EvalTask::Expression(expression, expected, true));
                        pending.push(EvalTask::Expression(value, None, false));
                    }
                }
                Expr::Loop(loop_expression) => {
                    self.loop_tasks(
                        expression,
                        loop_expression,
                        expected,
                        visited,
                        &mut pending,
                        &mut values,
                    )?;
                }
                Expr::Sequence(sequence) => {
                    self.sequence_syntax(expression, sequence)?;
                    self.sequence_tasks(&sequence.elements, expected, &mut pending)?;
                }
                Expr::Array(array) => {
                    self.array_tasks(
                        expression,
                        array,
                        expected,
                        visited,
                        &mut pending,
                        &mut values,
                    )?;
                }
                Expr::Call { .. } => {
                    self.evaluate_call(expression, expected, visited, &mut pending, &mut values)?;
                }
                Expr::Binary {
                    left,
                    operator,
                    right,
                } if self.dispatches(left, operator.value)? => {
                    let infix =
                        InfixOperator::from_binary(operator.value).expect("guarded infix operator");
                    self.child(expression.span_id, left.span_id)?;
                    self.child(expression.span_id, operator.span_id)?;
                    self.child(expression.span_id, right.span_id)?;
                    self.token(operator.span_id, infix.symbol())?;
                    let plan = self.operator_plan(left, infix)?;
                    if visited {
                        let right_value = values.pop().ok_or(SourceCheckError::Malformed)?;
                        let left_value = values.pop().ok_or(SourceCheckError::Malformed)?;
                        // An open domain with no operator of its own is the number it holds.
                        let open = (infix.arithmetic().is_some() || infix.is_integer_operation())
                            && matches!(left_value.ty, SourceType::Aggregate { .. })
                            && self.is_open_domain(left_value.ty)
                            && self.find_infix(left_value.ty, infix)?.is_none();
                        let (left_value, right_value) = if open {
                            (self.open_number(left_value)?, self.open_number(right_value)?)
                        } else {
                            (left_value, right_value)
                        };
                        if matches!(left_value.ty, SourceType::Aggregate { .. }) {
                            // A nominal left operand has no built-in arithmetic: its type
                            // must define the operator with `#infix`.
                            let function = self
                                .find_infix(left_value.ty, infix)?
                                .ok_or(SourceCheckError::Type)?;
                            // An operand the method only borrows must have been a place we
                            // could lend; a computed linear one would simply be lost.
                            if let Some((_, passing)) = &plan {
                                for (operand, value, passing) in [
                                    (left, left_value, passing[0]),
                                    (right, right_value, passing[1]),
                                ] {
                                    let linear = value.owner.is_some_and(|owner| {
                                        !self.proof.owners[owner].unrestricted
                                    });
                                    if passing.borrows()
                                        && linear
                                        && !self.is_lent(operand, passing)?
                                    {
                                        return Err(SourceCheckError::Unsupported);
                                    }
                                }
                            }
                            values.push(self.apply_call(function, vec![left_value, right_value])?);
                            if let Some((_, passing)) = &plan {
                                for (operand, passing) in [(left, passing[0]), (right, passing[1])]
                                {
                                    if passing == calls::Passing::Mut && self.is_place(operand)? {
                                        self.havoc_place(operand)?;
                                    }
                                }
                            }
                            continue;
                        }
                        // On integers the checker computes only `+ - *` itself.
                        let exact = match infix.arithmetic() {
                            Some(arithmetic) => {
                                self.arithmetic(arithmetic, left_value, right_value)?
                            }
                            // `/ % & | ^ << >>`: checked for their result domain, not recorded.
                            None => self.integer_operation(infix, left_value, right_value)?,
                        };
                        values.push(exact);
                    } else {
                        self.charge(3)?;
                        let mut lend = [false, false];
                        let mut exclusive = [false, false];
                        if let Some((_, passing)) = &plan {
                            self.check_call_borrows(
                                &[(left, SourceType::Unit), (right, SourceType::Unit)],
                                passing,
                            )?;
                            for (index, operand) in [left, right].into_iter().enumerate() {
                                lend[index] = self.is_lent(operand, passing[index])?;
                                exclusive[index] = passing[index] == calls::Passing::Mut;
                            }
                        }
                        pending.push(EvalTask::Expression(expression, expected, true));
                        pending.push(if lend[1] {
                            EvalTask::Borrow(right, exclusive[1])
                        } else {
                            EvalTask::Expression(right, None, false)
                        });
                        pending.push(if lend[0] {
                            EvalTask::Borrow(left, exclusive[0])
                        } else {
                            EvalTask::Expression(left, None, false)
                        });
                    }
                }
                Expr::Name(name)
                    if matches!(expected, Some(SourceType::Integer { .. }))
                        && self.is_borrowed_domain_read(name)? =>
                {
                    // Inside a domain's method, a borrowed receiver read as an integer is a
                    // copy of its hidden number, so nothing is moved.
                    values.push(self.borrow_place(expression, false)?);
                }
                Expr::Name(name) if self.number_value(name).is_some() => {
                    // A number type parameter read as a value, inside a method of its type.
                    self.token(expression.span_id, name)?;
                    let number = self.number_value(name).ok_or(SourceCheckError::Malformed)?;
                    values.push(self.scalar_facts(number)?);
                }
                Expr::Name(name) if self.constant_in_scope(name)? => {
                    // A module constant read as a value: the number it evaluated to.
                    self.token(expression.span_id, name)?;
                    let number = self.local(name, name)?;
                    values.push(number);
                }
                Expr::Name(name) if !self.proof.names.iter().any(|(other, _)| *other == name) => {
                    values.push(self.bare_value(expression, expected)?);
                }
                Expr::Chain(_) => {
                    self.chain_tasks(expression, 0, expected, &mut pending)?;
                }
                Expr::Conditional(case) => {
                    self.conditional_tasks(
                        expression,
                        case,
                        expected,
                        visited,
                        &mut pending,
                        &mut values,
                    )?;
                }
                Expr::Match(case) => {
                    if visited {
                        let subject = values.pop().ok_or(SourceCheckError::Malformed)?;
                        self.match_tasks(case, subject, expected, &mut pending)?;
                    } else {
                        self.child(expression.span_id, case.when_keyword)?;
                        self.child(expression.span_id, case.is_keyword)?;
                        self.child(expression.span_id, case.subject.span_id)?;
                        self.child(expression.span_id, case.arms.span_id)?;
                        self.charge(2)?;
                        pending.push(EvalTask::Expression(expression, expected, true));
                        pending.push(EvalTask::Expression(&case.subject, None, false));
                    }
                }
                Expr::Block(block) => {
                    self.block_tasks(expression, block, expected, &mut pending)?;
                }
                Expr::Group(child) => {
                    self.group_syntax(expression, child)?;
                    self.charge(1)?;
                    pending.push(EvalTask::Expression(child, expected, false));
                }
                Expr::Bind(binding) => {
                    let declared = self.local_type(binding, expression.span_id)?;
                    self.charge(1)?;
                    let value = &binding
                        .value
                        .as_ref()
                        .ok_or(SourceCheckError::MissingBody)?
                        .expression;
                    pending.push(EvalTask::BindView(binding, declared));
                    pending.push(EvalTask::Expression(value, declared, false));
                }
                // `level.value` on a union whose variants have values: derived, see `sums.rs`.
                Expr::Member { base, name } if name.value == "value" && self.discriminant_union(base).is_some() => {
                    values.push(self.discriminant_value(expression, base, name)?);
                }
                Expr::Member { .. } if self.bound_target(expression)?.is_some() => {
                    // `value.release` on a value of a bounded type parameter.
                    let (function, types, receiver) = self
                        .bound_target(expression)?
                        .expect("guarded bound target");
                    self.proof.forced_types = Some(types);
                    self.call_tasks(
                        expression,
                        function,
                        receiver,
                        None,
                        visited,
                        &mut pending,
                        &mut values,
                    )?;
                }
                // `rest.origin` with `origin(self ref) T: return self.base`: reading the accessor
                // is reading the field, so the number keeps its identity and what is known of it.
                Expr::Member { base, name } if !visited && self.plain_accessor_read(expression, base, name)?.is_some() => {
                    let value = self
                        .plain_accessor_read(expression, base, name)?
                        .ok_or(SourceCheckError::Malformed)?;
                    values.push(value);
                }
                Expr::Member { .. } if self.method_target(expression)?.is_some() => {
                    // `p.sum` without parentheses calls a method that takes only `self`.
                    let (function, receiver) = self
                        .method_target(expression)?
                        .expect("guarded method target");
                    self.call_tasks(
                        expression,
                        function,
                        receiver,
                        None,
                        visited,
                        &mut pending,
                        &mut values,
                    )?;
                }
                Expr::Member { base, .. } if self.names_union(base)? => {
                    values.push(self.bare_value(expression, expected)?);
                }
                Expr::Member { base, name } if !self.is_place(expression)? => {
                    // The base is computed, not a place: its field cannot be named safely
                    // yet, but a field nobody may name is reported as private first.
                    self.child(expression.span_id, base.span_id)?;
                    self.child(expression.span_id, name.span_id)?;
                    self.name(name)?;
                    if visited {
                        let base = values.pop().ok_or(SourceCheckError::Malformed)?;
                        values.push(self.computed_projection(base, name)?);
                        continue;
                    }
                    self.charge(2)?;
                    pending.push(EvalTask::Expression(expression, expected, true));
                    pending.push(EvalTask::Expression(base, None, false));
                }
                Expr::Member { .. } => {
                    // A field read copies; a linear field can only be lent (as an argument
                    // or receiver) or taken out by destructuring its record.
                    let field = self.project_place(expression)?;
                    if field
                        .owner
                        .is_some_and(|owner| !self.proof.owners[owner].unrestricted)
                    {
                        return Err(SourceCheckError::MoveOutOfBorrow);
                    }
                    values.push(field);
                }
                // `deref value`: the value a borrow refers to, copied out.
                Expr::Unary { operator, operand } if operator.value == UnaryOperator::Deref => {
                    if visited {
                        let borrowed = values.pop().ok_or(SourceCheckError::Malformed)?;
                        values.push(self.copy_out(borrowed)?);
                    } else if self.proof.in_copy && self.is_place(operand)? {
                        // In a type's own `copy` the receiver is read in place, not moved.
                        self.keyword(operator.span_id, "deref")?;
                        self.child(expression.span_id, operator.span_id)?;
                        self.child(expression.span_id, operand.span_id)?;
                        let borrowed = self.project_place(operand)?;
                        values.push(self.copy_out(borrowed)?);
                    } else {
                        self.keyword(operator.span_id, "deref")?;
                        self.child(expression.span_id, operator.span_id)?;
                        self.child(expression.span_id, operand.span_id)?;
                        self.charge(2)?;
                        pending.push(EvalTask::Expression(expression, expected, true));
                        pending.push(EvalTask::Expression(operand, None, false));
                    }
                }
                // `ref place` and `mut place` in a value position create a borrow of the place.
                Expr::PrefixedType(prefixed)
                    if matches!(
                        prefixed.keyword.value,
                        TypePrefixKind::Ref | TypePrefixKind::Mut
                    ) && prefixed.group.is_none() =>
                {
                    let exclusive = prefixed.keyword.value == TypePrefixKind::Mut;
                    self.keyword(prefixed.keyword.span_id, if exclusive { "mut" } else { "ref" })?;
                    self.child(expression.span_id, prefixed.keyword.span_id)?;
                    self.child(expression.span_id, prefixed.target.span_id)?;
                    self.charge(2)?;
                    values.push(self.borrow_handle(&prefixed.target, exclusive)?);
                }
                // `@place`: the unchecked address of a place.
                Expr::PrefixedType(prefixed)
                    if prefixed.keyword.value == TypePrefixKind::Pointer
                        && prefixed.group.is_none() =>
                {
                    self.pointer_syntax(expression, prefixed)?;
                    values.push(self.address_expression(expression.span_id, &prefixed.target)?);
                }
                _ => values.push(self.expression(expression)?),
            }
        }
        if values.len() != 1 {
            return Err(SourceCheckError::Malformed);
        }
        if let Some(expected) = expected {
            values[0] = self.view_as(values[0], expected)?;
        }
        // A handle is a borrow of an owner someone else holds: it owes nothing itself.
        if let Some(owner) = values[0].owner
            && values[0].borrow.is_none()
        {
            self.obligate(owner);
        }
        Ok(values[0])
    }

    fn evaluate_call(
        &mut self,
        expression: &'a Expression,
        expected: Option<SourceType>,
        visited: bool,
        pending: &mut Vec<EvalTask<'a>>,
        values: &mut Vec<Value>,
    ) -> Result<()> {
        let Expr::Call { callee, arguments } = &expression.value else {
            unreachable!("evaluate_call is scheduled only for calls");
        };
        self.child(expression.span_id, callee.span_id)?;
        self.child(expression.span_id, arguments.span_id)?;
        self.proof.call_expected = expected;
        if let Some(function) = self.call_target(callee)? {
            return self.call_tasks(
                expression,
                function,
                None,
                Some(arguments),
                visited,
                pending,
                values,
            );
        }
        if let Some((function, types, receiver)) = self.bound_target(callee)? {
            self.proof.forced_types = Some(types);
            return self.call_tasks(
                expression,
                function,
                receiver,
                Some(arguments),
                visited,
                pending,
                values,
            );
        }
        if let Some((function, types)) = self.applied_target(callee)? {
            self.proof.forced_types = Some(types);
            return self.call_tasks(
                expression,
                function,
                None,
                Some(arguments),
                visited,
                pending,
                values,
            );
        }
        if let Some((function, receiver)) = self.method_target(callee)? {
            return self.call_tasks(
                expression,
                function,
                receiver,
                Some(arguments),
                visited,
                pending,
                values,
            );
        }
        self.proof.call_expected = None;
        let (schema, context) = self.constructor_type(callee, expected)?;
        // `Union.Variant` with no fields is a value, not something to call.
        if matches!(callee.value, Expr::Member { .. })
            && self.module.layouts[schema].schema.fields.is_empty()
        {
            return Err(SourceCheckError::Type);
        }
        self.schema_access(schema)?;
        if visited {
            let count = arguments.value.len();
            if values.len() < count {
                return Err(SourceCheckError::Malformed);
            }
            let actual = values.split_off(values.len() - count);
            let packed = self.pack(schema, context, arguments, actual)?;
            self.note_construct(expression, packed.ty);
            values.push(packed);
        } else {
            self.constructor_tasks(expression, schema, context, arguments, pending)?;
        }
        Ok(())
    }

    /// `element.release(value)` where `element` is a type parameter bound to a trait that
    /// declares `release`: the call's function, and the types it is generic over (`Self` first).
    fn bound_target(
        &mut self,
        callee: &'a Expression,
    ) -> Result<Option<(usize, Vec<SourceType>, Option<&'a Expression>)>> {
        let Expr::Member { base, name } = &callee.value else {
            return Ok(None);
        };
        let Expr::Name(head) = &base.value else {
            return Ok(None);
        };
        if self.proof.bounds.is_empty() {
            return Ok(None);
        }
        // `element.release(value)` names the type; `value.release()` is a value of that type.
        let (index, receiver) = match self.local_position(head)? {
            Some(position) => {
                let SourceType::Generic { index } = self.proof.names[position].1.ty else {
                    return Ok(None);
                };
                (index as usize, Some(&**base))
            }
            None => {
                let Some(index) = self.proof.generics.iter().position(|name| *name == head.as_str())
                else {
                    return Ok(None);
                };
                (index, None)
            }
        };
        self.charge(self.proof.bounds.len())?;
        for (generic, bound) in self.proof.bounds.clone() {
            if generic as usize != index {
                continue;
            }
            let (trait_name, arguments): (&str, &[Spanned<Argument>]) = match &bound.value {
                Expr::Name(trait_name) => (trait_name.as_str(), &[]),
                Expr::Call { callee, arguments } => match &callee.value {
                    Expr::Name(trait_name) => (trait_name.as_str(), arguments.value.as_slice()),
                    _ => continue,
                },
                _ => continue,
            };
            let Some(function) = self.lookup_method(trait_name, &name.value)? else {
                continue;
            };
            self.child(callee.span_id, base.span_id)?;
            self.child(callee.span_id, name.span_id)?;
            self.token(base.span_id, head)?;
            self.name(name)?;
            let scope: Vec<_> = self
                .proof
                .generics
                .iter()
                .enumerate()
                .map(|(position, name)| {
                    (*name, SourceType::Generic { index: position as u32 })
                })
                .collect();
            let mut types = vec![SourceType::Generic { index: index as u32 }];
            for argument in arguments {
                let Argument::Positional(written) = &argument.value else {
                    return Err(SourceCheckError::Type);
                };
                types.push(self.resolve_type(written, &scope)?);
            }
            return Ok(Some((function, types, receiver)));
        }
        Ok(None)
    }

    pub(super) fn constructor_type(
        &mut self,
        callee: &'a Expression,
        expected: Option<SourceType>,
    ) -> Result<(usize, Option<SourceType>)> {
        let Expr::Name(name) = &callee.value else {
            self.charge(self.proof.generics.len())?;
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
            let ty = self.resolve_type(callee, &scope)?;
            let SourceType::Aggregate { id } = ty else {
                return Err(SourceCheckError::Type);
            };
            let schema = self.module.instances[id as usize].schema();
            // `Slot.Full(x)` on a generic union: the variant of the expected `Slot(Scalar)`.
            if !self.module.layouts[schema].schema.parameters.is_empty() {
                let found = match expected {
                    Some(SourceType::Sum { id: sum }) => self.sum_variants(sum)?.into_iter().find(|variant| {
                        matches!(variant, SourceType::Aggregate { id } if self.module.instances[*id as usize].schema() == schema)
                    }),
                    // The second visit is given the variant itself.
                    Some(variant @ SourceType::Aggregate { id })
                        if self.module.instances[id as usize].schema() == schema =>
                    {
                        Some(variant)
                    }
                    _ => None,
                };
                let Some(variant) = found else {
                    let written = self.module.layouts[schema].schema.name.clone();
                    let detail = format!(
                        "`{written}` is a variant of a generic union and nothing says which instance is meant; write the expected type (a typed binding or a result type such as `{}(...)`)",
                        written.split('.').next().unwrap_or(&written)
                    );
                    return Err(self.explain(SourceCheckError::Type, detail));
                };
                return Ok((schema, Some(variant)));
            }
            return Ok((schema, Some(ty)));
        };
        self.token(callee.span_id, name)?;
        self.check_constructor_scope(name)?;
        // Inside a type's methods, `Self(...)` builds the receiver type itself.
        if name == "Self"
            && let Some(receiver @ SourceType::Aggregate { id }) = self.proof.receiver
        {
            return Ok((self.module.instances[id as usize].schema(), Some(receiver)));
        }
        // Where a sum is expected, a bare name is that sum's variant of the name.
        let alternatives = match expected {
            Some(SourceType::Sum { id }) => Some(self.sum_variants(id)?),
            _ => None,
        };
        // The second visit of a call is given the variant itself as the expected type.
        let named_by_expected = match expected {
            Some(SourceType::Aggregate { id }) => {
                let schema = self.module.instances[id as usize].schema();
                let written = self.module.layouts[schema].schema.name.as_str();
                (written.contains('.') && written.rsplit('.').next() == Some(name.as_str()))
                    .then_some(schema)
            }
            _ => None,
        };
        let schema = match (&alternatives, named_by_expected) {
            (_, Some(schema)) => schema,
            (Some(variants), None) => self.resolve_variant(variants, name)?,
            (None, None) => match self.schema(name) {
                Err(SourceCheckError::UnknownName)
                    if self.module.union_variants.iter().any(|(_, variant, _)| variant == name) =>
                {
                    let unions: Vec<String> = self
                        .module
                        .union_variants
                        .iter()
                        .filter(|(_, variant, _)| variant == name)
                        .map(|(owner, _, _)| self.module.schemas[*owner].name.value.clone())
                        .collect();
                    let detail = format!(
                        "`{name}` is a variant of `{}`, not a name of the module, and nothing says which union is expected here; write the expected type (a typed binding, a parameter or a result type) or `{}.{name}`",
                        unions.join("` or `"),
                        unions[0]
                    );
                    return Err(self.explain(SourceCheckError::UnknownName, detail));
                }
                other => other?,
            },
        };
        if !self.module.layouts[schema].schema.variants.is_empty() {
            return Err(SourceCheckError::Type);
        }
        let expected = match alternatives {
            Some(variants) => variants.into_iter().find(|ty| {
                matches!(ty, SourceType::Aggregate { id } if self.module.instances[*id as usize].schema() == schema)
            }),
            None => expected,
        };
        let context = match expected {
            Some(SourceType::Aggregate { id })
                if self.module.instances[id as usize].schema() == schema =>
            {
                expected
            }
            _ => None,
        };
        Ok((schema, context))
    }

    fn constructor_tasks(
        &mut self,
        expression: &'a Expression,
        schema: usize,
        context: Option<SourceType>,
        arguments: &'a Spanned<Vec<Spanned<Argument>>>,
        pending: &mut Vec<EvalTask<'a>>,
    ) -> Result<()> {
        let resolved = self.constructor_arguments(schema, arguments)?;
        // A record without type parameters has one instance, so its field domains are
        // known where no type is expected: a wrapping field needs to know it is one.
        let context = match context {
            None if self.module.layouts[schema].schema.parameters.is_empty() => {
                Some(self.instance(schema, Vec::new())?)
            }
            // A generic record with no expected type: its arguments may be read from the payload
            // of a variant given to one of its fields (`Buffer(target: Full(n), rest: Empty)`).
            None if !self.is_declared_variant(schema) => self.infer_from_variant_payloads(schema, &resolved)?,
            other => other,
        };
        let types = match context {
            Some(SourceType::Aggregate { id }) => Some(self.instance_fields(id)?),
            _ => None,
        };
        self.charge(resolved.len() + 1)?;
        pending.push(EvalTask::Expression(expression, context, true));
        // Resolve fields by name, but evaluate values in their written order.
        for (index, value) in resolved.into_iter().rev() {
            let expected = types.as_ref().map(|types| types[index]);
            // A `ref`/`mut` field is given a handle, never a value.
            if let Some(kind) = self.module.layouts[schema].schema.fields[index].borrow {
                pending.push(EvalTask::Handle(value, kind == BorrowKind::Exclusive));
                continue;
            }
            pending.push(EvalTask::Expression(value, expected, false));
        }
        Ok(())
    }

    /// The instance of the generic record `schema` that the variants given to its fields imply:
    /// a field `target Slot(element)` given `Full(n)` binds `element` to the type of the local `n`.
    /// `None` unless every parameter is bound this way.
    fn infer_from_variant_payloads(
        &mut self,
        schema: usize,
        resolved: &[(usize, &'a Expression)],
    ) -> Result<Option<SourceType>> {
        let count = self.module.layouts[schema].schema.parameters.len();
        let mut bound: Vec<Option<SourceType>> = vec![None; count];
        for (index, value) in resolved {
            let Expr::Call { callee, arguments } = &value.value else { continue };
            let (Expr::Name(variant), [first]) = (&callee.value, arguments.value.as_slice()) else { continue };
            let Argument::Positional(payload) = &first.value else { continue };
            let Expr::Name(local) = &payload.value else { continue };
            let SourceType::Sum { id } = self.module.layouts[schema].schema.fields[*index].ty else { continue };
            let union = self.module.instances[id as usize].schema();
            let union_arguments = self.module.instances[id as usize].arguments.clone();
            let Some(layout) = self.module.union_variants.iter().find(|(owner, name, _)| {
                self.module.layouts[*owner].schema.name == self.module.layouts[union].schema.name && name == variant
            }).map(|(_, _, layout)| *layout) else { continue };
            let Some(field) = self.module.layouts[layout].schema.fields.first() else { continue };
            let SourceType::Generic { index: parameter } = field.ty else { continue };
            let Some(SourceType::Generic { index: slot }) = union_arguments.get(parameter as usize).copied() else { continue };
            let Some(position) = self.local_position(local)? else { continue };
            let ty = self.proof.names[position].1.ty;
            if let Some(entry) = bound.get_mut(slot as usize) {
                entry.get_or_insert(ty);
            }
        }
        let Some(arguments) = bound.into_iter().collect::<Option<Vec<_>>>() else {
            return Ok(None);
        };
        Ok(Some(self.instance(schema, arguments)?))
    }

    fn constructor_arguments(
        &mut self,
        schema: usize,
        arguments: &'a Spanned<Vec<Spanned<Argument>>>,
    ) -> Result<Vec<(usize, &'a Expression)>> {
        let count = self.module.layouts[schema].schema.fields.len();
        if count != arguments.value.len() {
            return Err(SourceCheckError::Type);
        }
        self.charge_product(count, 2)?;
        let mut seen = vec![false; count];
        let mut resolved = Vec::new();
        for (position, argument) in arguments.value.iter().enumerate() {
            self.child(arguments.span_id, argument.span_id)?;
            let (index, value) = self.constructor_argument(schema, argument, position)?;
            if seen[index] {
                return Err(SourceCheckError::Type);
            }
            seen[index] = true;
            resolved.push((index, value));
        }
        Ok(resolved)
    }

    fn constructor_argument(
        &mut self,
        schema: usize,
        argument: &'a Spanned<Argument>,
        position: usize,
    ) -> Result<(usize, &'a Expression)> {
        let (name, value) = match &argument.value {
            Argument::Named { name, value } => {
                self.child(argument.span_id, name.span_id)?;
                self.child(argument.span_id, value.span_id)?;
                (self.name(name)?, value)
            }
            // `name Type` declares a field; it is not a value.
            Argument::Typed { .. } => return Err(SourceCheckError::Type),
            Argument::Positional(value) => {
                self.child(argument.span_id, value.span_id)?;
                // The one payload of a variant a union declares needs no label.
                let fields = &self.module.layouts[schema].schema.fields;
                if fields.len() == 1 && fields[0].name.is_empty() {
                    return Ok((0, value));
                }
                // A variant's first payload is positional even when it has a name.
                if position == 0 && !fields.is_empty() && self.is_declared_variant(schema) {
                    return Ok((0, value));
                }
                let Expr::Name(name) = &value.value else {
                    return Err(SourceCheckError::Type);
                };
                self.token(value.span_id, name)?;
                (name.as_str(), value)
            }
        };
        self.charge_product(
            self.module.layouts[schema].schema.fields.len(),
            name.len() + 1,
        )?;
        let index = self.module.layouts[schema]
            .schema
            .fields
            .iter()
            .position(|field| field.name == name)
            .ok_or(SourceCheckError::Type)?;
        Ok((index, value))
    }

    fn pack(
        &mut self,
        schema: usize,
        context: Option<SourceType>,
        arguments: &'a Spanned<Vec<Spanned<Argument>>>,
        actual: Vec<Value>,
    ) -> Result<Value> {
        let resolved = self.constructor_arguments(schema, arguments)?;
        let indices = resolved.into_iter().map(|(index, _)| index).collect();
        self.pack_indexed(schema, context, indices, actual)
    }

    /// Build the record `schema` from `actual`, the value of field `indices[i]` being `actual[i]`.
    pub(super) fn pack_indexed(
        &mut self,
        schema: usize,
        context: Option<SourceType>,
        indices: Vec<usize>,
        actual: Vec<Value>,
    ) -> Result<Value> {
        let count = self.module.layouts[schema].schema.fields.len();
        if actual.len() != indices.len() {
            return Err(SourceCheckError::Malformed);
        }
        self.charge(count)?;
        let mut ordered = vec![None; count];
        for (index, value) in indices.into_iter().zip(actual) {
            if ordered[index].replace(value).is_some() {
                return Err(SourceCheckError::Type);
            }
        }
        self.charge(ordered.len())?;
        let mut children = ordered
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or(SourceCheckError::Type)?;
        let ty = self.infer_instance(schema, context, &children)?;
        let SourceType::Aggregate { id } = ty else {
            return Err(SourceCheckError::Type);
        };
        let typed = self.instance_fields(id)?;
        for (expected, value) in typed.iter().zip(&mut children) {
            *value = self.view_as(*value, *expected)?;
        }
        self.lengths_agree(schema, &children)?;
        self.field_requirements(id, &mut children, false)?;
        for value in &children {
            // A handle moves into the view; the place it borrows is not consumed with it.
            if value.borrow.is_none() {
                self.release(*value)?;
            }
        }
        let value = self.fresh_value(ty)?;
        let owner = value.owner.ok_or(SourceCheckError::Type)?;
        self.proof.owners[owner].children = children;
        self.proof.owners[owner].materialized = true;
        self.obligate(owner);
        Ok(value)
    }

    fn infer_instance(
        &mut self,
        schema: usize,
        context: Option<SourceType>,
        children: &[Value],
    ) -> Result<SourceType> {
        if let Some(ty) = context {
            return Ok(ty);
        }
        let count = self.module.layouts[schema].schema.parameters.len();
        self.charge(count + children.len())?;
        let mut arguments = vec![None; count];
        for (index, value) in children.iter().enumerate() {
            let template = self.module.layouts[schema].schema.fields[index].ty;
            // Closed fields need no inference. Packing validates their active
            // values afterward, just as it does with an explicit type context.
            if self.needs_inference(template)? {
                let actual = value
                    .sum
                    .map(|id| SourceType::Sum { id })
                    .or(value.structural)
                    .unwrap_or(value.ty);
                self.infer_argument(template, actual, &mut arguments)?;
            }
        }
        self.charge(arguments.len())?;
        let arguments = arguments
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or(SourceCheckError::Type)?;
        self.instance(schema, arguments)
    }

    fn needs_inference(&mut self, root: SourceType) -> Result<bool> {
        self.charge(1)?;
        let mut pending = vec![root];
        while let Some(ty) = pending.pop() {
            self.charge(1)?;
            match ty {
                SourceType::Generic { .. } => return Ok(true),
                SourceType::Aggregate { id }
                | SourceType::Sum { id }
                | SourceType::Array { id }
                | SourceType::Sequence { id }
                | SourceType::Pointer { id } => {
                    self.charge(self.module.instances[id as usize].arguments.len())?;
                    pending.extend_from_slice(&self.module.instances[id as usize].arguments);
                }
                SourceType::Integer { .. } | SourceType::Unit | SourceType::Text => {}
            }
        }
        Ok(false)
    }

    pub(super) fn infer_argument(
        &mut self,
        template: SourceType,
        actual: SourceType,
        arguments: &mut [Option<SourceType>],
    ) -> Result<()> {
        self.charge(1)?;
        let mut pending = vec![(template, actual)];
        while let Some((template, actual)) = pending.pop() {
            self.charge(1)?;
            match (template, actual) {
                (SourceType::Generic { index }, actual) => {
                    let slot = arguments
                        .get_mut(index as usize)
                        .ok_or(SourceCheckError::Type)?;
                    if let Some(previous) = slot {
                        if *previous != actual {
                            return Err(SourceCheckError::Type);
                        }
                    } else {
                        *slot = Some(actual);
                    }
                }
                // `[element; capacity]` against a constant-extent array: the number is its length.
                (SourceType::Array { id: left }, SourceType::Array { id: right })
                    if self.module.instances[left as usize].head
                        == CheckedSourceInstanceHead::Sized
                        && matches!(
                            self.module.instances[right as usize].head,
                            CheckedSourceInstanceHead::Array { .. }
                        ) =>
                {
                    let CheckedSourceInstanceHead::Array { extent } =
                        self.module.instances[right as usize].head
                    else {
                        unreachable!("guarded constant-extent head");
                    };
                    let number = SourceType::Integer {
                        lower: I256::from(extent),
                        upper: I256::from(extent),
                    };
                    let template_arguments = self.module.instances[left as usize].arguments.clone();
                    let element = self.module.instances[right as usize].arguments[0];
                    pending.push((template_arguments[1], number));
                    pending.push((template_arguments[0], element));
                }
                (SourceType::Aggregate { id: left }, SourceType::Aggregate { id: right })
                | (SourceType::Sum { id: left }, SourceType::Sum { id: right })
                | (SourceType::Array { id: left }, SourceType::Array { id: right })
                | (SourceType::Sequence { id: left }, SourceType::Sequence { id: right })
                | (SourceType::Pointer { id: left }, SourceType::Pointer { id: right }) => {
                    let left = left as usize;
                    let right = right as usize;
                    if self.module.instances[left].head != self.module.instances[right].head {
                        if matches!(template, SourceType::Sum { .. })
                            && matches!(actual, SourceType::Sum { .. })
                        {
                            for variant in self.sum_variants(right as u32)? {
                                pending.push((template, variant));
                            }
                            continue;
                        }
                        return Err(SourceCheckError::Type);
                    }
                    let count = self.module.instances[left].arguments.len();
                    if count != self.module.instances[right].arguments.len() {
                        return Err(SourceCheckError::Type);
                    }
                    self.charge(count)?;
                    pending.extend(
                        self.module.instances[left]
                            .arguments
                            .iter()
                            .copied()
                            .zip(self.module.instances[right].arguments.iter().copied()),
                    );
                }
                (SourceType::Sum { id }, SourceType::Aggregate { id: actual_id }) => {
                    let head = self.module.instances[actual_id as usize].schema();
                    let variant = self.sum_variants(id)?.into_iter().find(|ty| {
                        matches!(ty, SourceType::Aggregate { id } if self.module.instances[*id as usize].schema() == head)
                    }).ok_or(SourceCheckError::Type)?;
                    pending.push((variant, actual));
                }
                _ => self.compatible(template, actual)?,
            }
        }
        Ok(())
    }
}

impl<'a> Checker<'a> {
    pub(super) fn schema_access(&mut self, schema: usize) -> Result<()> {
        let fields = &self.module.layouts[schema].schema.fields;
        self.charge(fields.len())?;
        if schema >= self.module.schemas.len()
            && self.module.layouts[schema]
                .schema
                .fields
                .iter()
                .any(|field| field.private)
        {
            return Err(SourceCheckError::Privacy);
        }
        Ok(())
    }

    /// Whether `base` is the name of a union rather than a value in scope, so that
    /// `base.Variant` is a variant and not a field.
    pub(super) fn names_union(&mut self, base: &'a Expression) -> Result<bool> {
        let Expr::Name(name) = &base.value else {
            return Ok(false);
        };
        if self.local_position(name)?.is_some() || self.proof.generics.contains(&name.as_str()) {
            return Ok(false);
        }
        let Some(schema) = self.lookup_symbol(name, false)? else {
            return Ok(false);
        };
        let layout = &self.module.layouts[schema].schema;
        Ok(!layout.variants.is_empty()
            || self
                .module
                .union_variants
                .iter()
                .any(|(u, _, _)| *u == schema))
    }

    pub(super) fn check_constructor_scope(&mut self, name: &str) -> Result<()> {
        self.charge(self.proof.generics.len())?;
        if self.local_position(name)?.is_some() || self.proof.generics.contains(&name) {
            return Err(SourceCheckError::Type);
        }
        Ok(())
    }

    /// A method of a generic type that takes `self` is generic over the type's parameters:
    /// they come first, ahead of the written ones, and are read from the receiver at a call.
    pub(super) fn implicit_type_parameters(&mut self, binding: &'a Binding) -> Result<()> {
        // A trait's bodiless method is generic over `Self` and the trait's own parameters.
        if let Some(declaration) = self.proof.abstract_trait {
            self.proof.generics.push("Self");
            self.proof.parameter_roles.push(calls::Parameter::Type);
            self.proof.parameter_passing.push(calls::Passing::Owner);
            if let Some(parameters) = declaration.parameters.as_ref() {
                self.charge(parameters.value.len())?;
                for parameter in &parameters.value {
                    self.proof.generics.push(parameter.value.name.value.as_str());
                    self.proof.parameter_roles.push(calls::Parameter::Type);
                    self.proof.parameter_passing.push(calls::Passing::Owner);
                }
            }
            return Ok(());
        }
        let Some(schema) = self.proof.receiver_schema else {
            return Ok(());
        };
        if !modules::takes_self(binding) && numbers::writes_type_parameters(binding) {
            return Ok(());
        }
        // Method bodies use the names the `has` block of the type writes.
        let declaration = self.module.schema_domain_has[schema].unwrap_or(self.module.schemas[schema]);
        let Some(parameters) = declaration.parameters.as_ref() else {
            return Ok(());
        };
        self.charge(parameters.value.len())?;
        let first = self.proof.generics.len();
        for parameter in &parameters.value {
            let index = self.proof.generics.len() as u32;
            // `Copied(element has Release)`: calls on `element` need the trait it names.
            if let Some(Spanned {
                value: Annotation::Bound { bound, .. },
                ..
            }) = &parameter.value.annotation
            {
                self.proof.bounds.push((index, bound));
            }
            self.proof.generics.push(parameter.value.name.value.as_str());
            self.proof.parameter_roles.push(calls::Parameter::Type);
            self.proof.parameter_passing.push(calls::Passing::Owner);
        }
        self.declare_numbers(schema, first)?;
        Ok(())
    }

    pub(super) fn checked_type_parameters(&mut self) -> Result<Vec<String>> {
        self.charge(self.proof.generics.len())?;
        let mut output = Vec::new();
        for index in 0..self.proof.generics.len() {
            self.charge(self.proof.generics[index].len())?;
            output.push(self.proof.generics[index].to_owned());
        }
        Ok(output)
    }

    pub(super) fn narrow_scalar(&mut self, value: Value) -> Result<()> {
        self.charge(self.proof.scalars.len())?;
        if let Some(identity) = value.identity {
            let scalar = self
                .proof
                .scalars
                .iter_mut()
                .find(|(id, _, _)| *id == identity)
                .ok_or(SourceCheckError::Type)?;
            scalar.1 = scalar.1.max(value.lower);
            scalar.2 = scalar.2.min(value.upper);
        }
        self.consistent_facts()
    }

    fn fact_edges(&mut self) -> Result<Vec<(usize, usize, bool)>> {
        // One pass over the scalars builds the index (only when there are facts to look up);
        // each fact then costs two lookups and its edges.
        if !self.proof.facts.is_empty() {
            self.charge(self.proof.scalars.len())?;
        }
        self.charge_product(self.proof.facts.len(), 2)?;
        edges_indexed(&self.proof.facts, &self.proof.scalars)
    }

    fn consistent_facts(&mut self) -> Result<()> {
        let edges = self.fact_edges()?;
        self.charge(self.proof.scalars.len())?;
        let mut bounds = self.proof.scalars.clone();
        for _ in 0..=bounds.len() {
            self.charge(edges.len() + bounds.len())?;
            let mut changed = false;
            for (left, right, strict) in &edges {
                let lower = if *strict {
                    bounds[*left]
                        .1
                        .checked_add(I256::ONE)
                        .ok_or(SourceCheckError::Type)?
                } else {
                    bounds[*left].1
                };
                let upper = if *strict {
                    bounds[*right]
                        .2
                        .checked_sub(I256::ONE)
                        .ok_or(SourceCheckError::Type)?
                } else {
                    bounds[*right].2
                };
                if lower > bounds[*right].1 {
                    bounds[*right].1 = lower;
                    changed = true;
                }
                if upper < bounds[*left].2 {
                    bounds[*left].2 = upper;
                    changed = true;
                }
            }
            if bounds.iter().any(|(_, lower, upper)| lower > upper) {
                return Err(SourceCheckError::Type);
            }
            if !changed {
                self.proof.scalars = bounds;
                return Ok(());
            }
        }
        Err(SourceCheckError::Type)
    }

    pub(super) fn entails(
        &mut self,
        operator: BinaryOperator,
        left: u32,
        right: u32,
    ) -> Result<bool> {
        self.charge_product(self.proof.scalars.len(), 2)?;
        let left = self
            .proof
            .scalars
            .iter()
            .position(|(id, _, _)| *id == left)
            .ok_or(SourceCheckError::Type)?;
        let right = self
            .proof
            .scalars
            .iter()
            .position(|(id, _, _)| *id == right)
            .ok_or(SourceCheckError::Type)?;
        let edges = self.fact_edges()?;
        match operator {
            BinaryOperator::Equal => Ok(self.reaches(&edges, left, right, false)?
                && self.reaches(&edges, right, left, false)?),
            BinaryOperator::Less => self.reaches(&edges, left, right, true),
            BinaryOperator::LessOrEqual => self.reaches(&edges, left, right, false),
            BinaryOperator::Greater => self.reaches(&edges, right, left, true),
            BinaryOperator::GreaterOrEqual => self.reaches(&edges, right, left, false),
            _ => Err(SourceCheckError::Unsupported),
        }
    }

    fn reaches(
        &mut self,
        edges: &[(usize, usize, bool)],
        left: usize,
        right: usize,
        strict: bool,
    ) -> Result<bool> {
        self.charge(self.proof.scalars.len() + 1)?;
        let mut seen = vec![[false; 2]; self.proof.scalars.len()];
        let mut pending = vec![(left, false)];
        while let Some((node, path_strict)) = pending.pop() {
            self.charge(1)?;
            if node == right && (!strict || path_strict) {
                return Ok(true);
            }
            if seen[node][usize::from(path_strict)] {
                continue;
            }
            seen[node][usize::from(path_strict)] = true;
            self.charge(edges.len())?;
            for (from, to, edge_strict) in edges {
                if *from == node {
                    pending.push((*to, path_strict || *edge_strict));
                }
            }
        }
        Ok(false)
    }
}

/// The trait a bound names: `Release`, or `Add` in `Add(Result)`.
pub(super) fn bound_trait_name(bound: &Expression) -> Option<&str> {
    match &bound.value {
        Expr::Name(name) => Some(name.as_str()),
        Expr::Call { callee, .. } => match &callee.value {
            Expr::Name(name) => Some(name.as_str()),
            _ => None,
        },
        _ => None,
    }
}

impl<'a> Checker<'a> {
    /// The field a plain accessor of a local type returns, read from the place `base`, when
    /// `base.name` is such an accessor and the field is a number or unrestricted. `None` leaves
    /// the expression to the ordinary call.
    pub(super) fn plain_accessor_read(
        &mut self,
        expression: &'a Expression,
        base: &'a Expression,
        name: &'a Name,
    ) -> Result<Option<Value>> {
        if self.plain_accessors.is_empty() || !self.is_place(base)? {
            return Ok(None);
        }
        let Some((function, _)) = self.method_target(expression)? else {
            return Ok(None);
        };
        self.charge(self.module.function_names.len() + self.plain_accessors.len())?;
        let Some((qualified, _)) = self
            .module
            .function_names
            .iter()
            .find(|(_, index)| *index == function)
            .cloned()
        else {
            return Ok(None);
        };
        let Some((owner, _)) = qualified.rsplit_once('.') else {
            return Ok(None);
        };
        if !self
            .plain_accessors
            .iter()
            .any(|(type_name, method)| type_name == owner && *method == name.value)
        {
            return Ok(None);
        }
        let parent = self.project_place(base)?;
        // By the accessor's name: the projection reads the field it returns, private or not.
        // Anything that does not resolve that way stays an ordinary call.
        let child = match self.projection_value(parent, name) {
            Ok(child) => child,
            Err(SourceCheckError::Budget) => return Err(SourceCheckError::Budget),
            Err(_) => return Ok(None),
        };
        let copyable = matches!(child.ty, SourceType::Integer { .. })
            || child.owner.is_some_and(|owner| self.proof.owners[owner].unrestricted);
        if !copyable {
            return Ok(None);
        }
        Ok(Some(self.scalar_facts(child)?))
    }
}

type FactEdges = Vec<(usize, usize, bool)>;

/// The edges of the facts over the scalars, finding each scalar through an index built once.
/// The first scalar with an id wins, as `position` does.
fn edges_indexed(
    facts: &[(u32, BinaryOperator, u32)],
    scalars: &[(u32, I256, I256)],
) -> Result<FactEdges> {
    let mut index = std::collections::BTreeMap::new();
    for (position, (id, _, _)) in scalars.iter().enumerate() {
        index.entry(*id).or_insert(position);
    }
    let mut edges = Vec::new();
    for (left, operator, right) in facts {
        let left = *index.get(left).ok_or(SourceCheckError::Type)?;
        let right = *index.get(right).ok_or(SourceCheckError::Type)?;
        push_edges(&mut edges, left, *operator, right)?;
    }
    Ok(edges)
}

fn push_edges(
    edges: &mut FactEdges,
    left: usize,
    operator: BinaryOperator,
    right: usize,
) -> Result<()> {
    match operator {
        BinaryOperator::Equal => {
            edges.push((left, right, false));
            edges.push((right, left, false));
        }
        BinaryOperator::Less => edges.push((left, right, true)),
        BinaryOperator::LessOrEqual => edges.push((left, right, false)),
        BinaryOperator::Greater => edges.push((right, left, true)),
        BinaryOperator::GreaterOrEqual => edges.push((right, left, false)),
        _ => return Err(SourceCheckError::Unsupported),
    }
    Ok(())
}

#[cfg(test)]
mod fact_edge_tests {
    use super::*;

    /// The earlier implementation: two linear scans of the scalars per fact.
    fn edges_scanned(
        facts: &[(u32, BinaryOperator, u32)],
        scalars: &[(u32, I256, I256)],
    ) -> Result<FactEdges> {
        let mut edges = Vec::new();
        for (left, operator, right) in facts {
            let left = scalars
                .iter()
                .position(|(id, _, _)| id == left)
                .ok_or(SourceCheckError::Type)?;
            let right = scalars
                .iter()
                .position(|(id, _, _)| id == right)
                .ok_or(SourceCheckError::Type)?;
            push_edges(&mut edges, left, *operator, right)?;
        }
        Ok(edges)
    }

    #[test]
    fn indexed_edges_match_the_scanned_edges() {
        use BinaryOperator::*;
        let operators = [Equal, Less, LessOrEqual, Greater, GreaterOrEqual, Add];
        // Ids repeat (the first wins) and some are absent, to cover every outcome.
        let scalars: Vec<(u32, I256, I256)> = [7u32, 3, 9, 3, 1, 12]
            .iter()
            .map(|id| (*id, I256::ZERO, I256::from(10)))
            .collect();
        let ids = [7u32, 3, 9, 1, 12, 99];
        let mut seed = 12345u32;
        let mut next = move || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 16) as usize
        };
        for length in 0..12 {
            for _ in 0..200 {
                let facts: Vec<_> = (0..length)
                    .map(|_| {
                        (
                            ids[next() % ids.len()],
                            operators[next() % operators.len()],
                            ids[next() % ids.len()],
                        )
                    })
                    .collect();
                assert_eq!(
                    edges_indexed(&facts, &scalars),
                    edges_scanned(&facts, &scalars),
                    "{facts:?}"
                );
            }
        }
        assert_eq!(edges_indexed(&[], &scalars), Ok(Vec::new()));
    }
}
