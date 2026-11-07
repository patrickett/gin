//! `where place :: value`: what a call does to the places it is lent `mut`.
//!
//! A clause is checked where the function returns: the value `expr` had at entry, which is
//! worked out before the body runs, must be exactly the value the place holds by then. A
//! number is known exactly through its entry value plus a constant (`Value::affine`), so
//! `x :: x + 1` is provable and `x :: x * 2` is refused. Callers use the published form
//! instead of forgetting the place, and a place the body never writes is unchanged.
use super::*;

/// A place a contract speaks about: a `mut` parameter, and the fields below it by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlacePath {
    pub(super) parameter: u32,
    pub(super) fields: Vec<String>,
}

/// What a clause computes, over the values the parameters had at entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ClauseExpr {
    Constant(I256),
    Place(PlacePath),
    Binary(CheckedArithmetic, Box<ClauseExpr>, Box<ClauseExpr>),
    /// `when left <comparison> right then A else B`, taken in the worlds the comparison
    /// allows, as a conditional is.
    Select {
        comparison: BinaryOperator,
        left: Box<ClauseExpr>,
        right: Box<ClauseExpr>,
        then_branch: Box<ClauseExpr>,
        else_branch: Box<ClauseExpr>,
    },
}

/// What a call does to one place of an argument it was lent `mut`. A place with no effect
/// is unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Effect {
    /// The place holds this afterwards.
    Set(PlacePath, ClauseExpr),
    /// The body may change the place and nothing says to what.
    Forgotten(PlacePath),
}

/// One checked clause of the function being verified.
#[derive(Clone)]
pub(super) struct Clause<'a> {
    place: &'a Expression,
    path: PlacePath,
    /// The value at entry, which the place must hold when the function returns.
    expected: Value,
    expr: ClauseExpr,
}

impl<'a> Checker<'a> {
    /// How the `k`-th value parameter is passed. Passing is kept per parameter role, type
    /// parameters included, while names and arguments count values only.
    fn value_passing(&self, k: usize) -> Option<calls::Passing> {
        let at = self
            .proof
            .parameter_roles
            .iter()
            .enumerate()
            .filter(|(_, role)| matches!(role, calls::Parameter::Value(_)))
            .nth(k)?
            .0;
        self.proof.parameter_passing.get(at).copied()
    }

    fn all_values(&self) -> bool {
        self.proof
            .parameter_roles
            .iter()
            // Type parameters are inferred and take no argument, so value arguments keep their
            // positions; only a compile-time literal has none to speak of.
            .all(|role| !matches!(role, calls::Parameter::Literal))
    }

    /// The `mut` parameter and fields a place expression names.
    pub(super) fn clause_place(&mut self, place: &Expression, parameter_count: usize) -> Result<PlacePath> {
        let (root, fields) = assign::split_place(place).ok_or(SourceCheckError::Malformed)?;
        let parameter = self.proof.names[..parameter_count]
            .iter()
            .rposition(|(name, _)| *name == root)
            .ok_or(SourceCheckError::UnknownName)?;
        if self.value_passing(parameter) != Some(calls::Passing::Mut) {
            return Err(SourceCheckError::Immutable);
        }
        let fields = self.unfold_accessors(parameter, &fields)?;
        Ok(PlacePath {
            parameter: parameter as u32,
            fields,
        })
    }

    /// Rewrite each segment that names an inline accessor (`self.initialized_count`) as the field
    /// it returns (`self.initialized`), following the types from the parameter. A segment that is
    /// a field, or whose type is not a record, is kept as written.
    fn unfold_accessors(&mut self, parameter: usize, fields: &[&Name]) -> Result<Vec<String>> {
        let mut ty = Some(self.proof.names[parameter].1.ty);
        let mut queue: std::collections::VecDeque<String> =
            fields.iter().map(|field| field.value.clone()).collect();
        let mut output = Vec::new();
        let mut steps = 0;
        while let Some(mut written) = queue.pop_front() {
            steps += 1;
            if steps > 32 {
                return Err(SourceCheckError::Unsupported);
            }
            self.charge(self.accessors.len() + 1)?;
            if let Some(SourceType::Aggregate { id }) = ty {
                let schema = self.module.instances[id as usize].schema();
                let owner = self.module.layouts[schema].schema.name.clone();
                let has_field = self.module.layouts[schema]
                    .schema
                    .fields
                    .iter()
                    .any(|candidate| candidate.name == written);
                if !has_field
                    && let Some((_, _, returned)) = self
                        .accessors
                        .iter()
                        .find(|(type_name, method, _)| *type_name == owner && *method == written)
                {
                    // An accessor reading deeper (`self.storage.length`) is that whole path.
                    let mut parts: Vec<String> = returned.split('.').map(str::to_owned).collect();
                    written = parts.remove(0);
                    for part in parts.into_iter().rev() {
                        queue.push_front(part);
                    }
                }
                let position = self.module.layouts[schema]
                    .schema
                    .fields
                    .iter()
                    .position(|candidate| candidate.name == written);
                ty = match position {
                    Some(at) => self.instance_fields(id)?.get(at).copied(),
                    None => None,
                };
            } else {
                ty = None;
            }
            output.push(written);
        }
        Ok(output)
    }

    /// The portable form of a clause's value: constants, places of parameters, `+ - *`.
    fn compile_clause(
        &mut self,
        expression: &'a Expression,
        parameter_count: usize,
    ) -> Result<ClauseExpr> {
        self.charge(1)?;
        match &expression.value {
            Expr::Group(inner) => self.compile_clause(inner, parameter_count),
            Expr::Literal(LiteralKind::Integer) => {
                let text = self
                    .literal_text(expression.span_id)
                    .ok_or(SourceCheckError::Malformed)?;
                let value = decode_integer_accumulator(text)?
                    .checked_neg()
                    .ok_or(SourceCheckError::Overflow)?;
                Ok(ClauseExpr::Constant(value))
            }
            Expr::Name(_) | Expr::Member { .. } => {
                let path = self.clause_read(expression, parameter_count)?;
                Ok(ClauseExpr::Place(path))
            }
            Expr::Binary {
                left,
                operator,
                right,
            } => {
                let arithmetic = InfixOperator::from_binary(operator.value)
                    .and_then(InfixOperator::arithmetic)
                    .ok_or(SourceCheckError::Unsupported)?;
                let left = self.compile_clause(left, parameter_count)?;
                let right = self.compile_clause(right, parameter_count)?;
                Ok(ClauseExpr::Binary(arithmetic, Box::new(left), Box::new(right)))
            }
            Expr::Conditional(case) => {
                let Expr::Binary {
                    left,
                    operator,
                    right,
                } = &case.condition.value
                else {
                    return Err(SourceCheckError::Unsupported);
                };
                if !matches!(
                    operator.value,
                    BinaryOperator::Equal
                        | BinaryOperator::Less
                        | BinaryOperator::LessOrEqual
                        | BinaryOperator::Greater
                        | BinaryOperator::GreaterOrEqual
                ) {
                    return Err(SourceCheckError::Unsupported);
                }
                Ok(ClauseExpr::Select {
                    comparison: operator.value,
                    left: Box::new(self.compile_clause(left, parameter_count)?),
                    right: Box::new(self.compile_clause(right, parameter_count)?),
                    then_branch: Box::new(self.compile_clause(&case.then_branch, parameter_count)?),
                    else_branch: Box::new(self.compile_clause(&case.else_branch, parameter_count)?),
                })
            }
            _ => Err(SourceCheckError::Unsupported),
        }
    }

    /// A read of any parameter, not only a `mut` one.
    fn clause_read(&mut self, place: &Expression, parameter_count: usize) -> Result<PlacePath> {
        let (root, fields) = assign::split_place(place).ok_or(SourceCheckError::Unsupported)?;
        let parameter = self.proof.names[..parameter_count]
            .iter()
            .rposition(|(name, _)| *name == root)
            .ok_or(SourceCheckError::UnknownName)?;
        let fields = self.unfold_accessors(parameter, &fields)?;
        Ok(PlacePath {
            parameter: parameter as u32,
            fields,
        })
    }

    /// Work out each clause's value at entry, before the body can change anything.
    // TODO(contract-models): before allowing general library computations or derived entry-state
    // ghosts in clauses, specify purity, termination and erasure. A ghost read of a linear owner
    // must never turn into an implicit runtime copy.
    pub(super) fn contract_entry(
        &mut self,
        binding: &'a Binding,
        parameter_count: usize,
    ) -> Result<()> {
        self.proof.clauses.clear();
        self.proof.writes.clear();
        for alternative in &binding.alternatives {
            self.span(alternative.span_id)?;
            self.keyword(alternative.value.or_keyword, "or")?;
            self.child(alternative.span_id, alternative.value.or_keyword)?;
            self.child(alternative.span_id, alternative.value.annotation.span_id)?;
            if let Some(contract) = &alternative.value.contract {
                self.child(alternative.span_id, contract.span_id)?;
                self.contract_syntax(contract)?;
            }
        }
        self.proof.variant_clauses.clear();
        if let Some(contract) = &binding.contract {
            if !self.all_values() {
                return Err(SourceCheckError::Unsupported);
            }
            self.span(contract.span_id)?;
            self.keyword(contract.value.where_keyword, "where")?;
            self.child(contract.span_id, contract.value.where_keyword)?;
            for (position, item) in contract.value.items.iter().enumerate() {
                self.span(item.span_id)?;
                self.child(contract.span_id, item.span_id)?;
                if position > 0 {
                    self.child(contract.span_id, contract.value.joiners[position - 1])?;
                    self.joiner(contract.value.joiners[position - 1])?;
                }
            }
        }
        // A result written as variants: each variant's transitions hold where it is returned
        // (`variant_clauses`, by variant), and a caller learns them where that variant is matched.
        if !binding.alternatives.is_empty() {
            let contracts: Vec<Option<&'a Spanned<Contract>>> = std::iter::once(binding.contract.as_ref())
                .chain(binding.alternatives.iter().map(|alt| alt.value.contract.as_ref()))
                .collect();
            for contract in contracts {
                let mut clauses = Vec::new();
                if let Some(contract) = contract {
                    if !self.all_values() && self.contract_has_transition(contract) {
                        return Err(SourceCheckError::Unsupported);
                    }
                    self.transition_clauses(contract, parameter_count, &mut clauses)?;
                }
                self.proof.variant_clauses.push(clauses);
            }
            return Ok(());
        }
        let Some(contract) = &binding.contract else {
            return Ok(());
        };
        let mut clauses = Vec::new();
        self.transition_clauses(contract, parameter_count, &mut clauses)?;
        self.proof.clauses = clauses;
        Ok(())
    }

    /// The `::` transitions of one `where`, with each place's value at entry.
    fn transition_clauses(
        &mut self,
        contract: &'a Spanned<Contract>,
        parameter_count: usize,
        clauses: &mut Vec<Clause<'a>>,
    ) -> Result<()> {
        for item in &contract.value.items {
            let ClauseItem::Transition(clause) = &item.value else {
                continue;
            };
            let transition = item;
            self.child(transition.span_id, clause.place.span_id)?;
            self.child(transition.span_id, clause.operator.clone())?;
            self.child(transition.span_id, clause.value.span_id)?;
            self.token(clause.operator, "::")?;
            let path = self.clause_place(&clause.place, parameter_count)?;
            if clauses.iter().any(|known: &Clause<'a>| known.path == path) {
                return Err(SourceCheckError::Malformed);
            }
            let expr = self.compile_clause(&clause.value, parameter_count)?;
            // The value at entry is read from the parameters by path, not by evaluating the
            // clause as an expression: an accessor in it (`self.length`) would be a call, and a
            // call on a `mut` parameter forgets what is known of it.
            let entry: Vec<Value> = self.proof.names[..parameter_count].iter().map(|(_, value)| *value).collect();
            let expected = self.clause_value(&expr, &entry)?;
            if !matches!(expected.ty, SourceType::Integer { .. }) {
                return Err(SourceCheckError::Unsupported);
            }
            clauses.push(Clause {
                place: &clause.place,
                path,
                expected,
                expr,
            });
        }
        Ok(())
    }

    /// The transitions of each variant of `binding`'s result, as a caller applies them: the
    /// place and the expression over the values at the call, by member.
    pub(super) fn variant_transitions(
        &mut self,
        contract: Option<&'a Spanned<Contract>>,
        parameter_count: usize,
    ) -> Result<Vec<(PlacePath, ClauseExpr)>> {
        let mut output = Vec::new();
        let Some(contract) = contract else { return Ok(output) };
        for item in &contract.value.items {
            if let ClauseItem::Transition(clause) = &item.value {
                let path = self.clause_place(&clause.place, parameter_count)?;
                let expr = self.compile_clause(&clause.value, parameter_count)?;
                output.push((path, expr));
            }
        }
        Ok(output)
    }

    /// Each claim must hold where the function returns, with the result bound to the name the
    /// header gives it. An unnamed result is only compared, in the leading-operator form.
    pub(super) fn claims_exit(
        &mut self,
        binding: &'a Binding,
        returned: Option<Value>,
    ) -> Result<()> {
        // A caller assumes each member's claims in that member's world (proved below).
        if !self.proof.variants.is_empty() && self.proof.variant_postconditions.is_empty() {
            self.compile_variant_claims(binding)?;
        }
        // The claims of the variant that was returned; the others describe other outcomes.
        let contract = if self.proof.variants.is_empty() {
            binding.contract.as_ref()
        } else {
            let mut at = returned.and_then(|value| {
                self.proof.variants.iter().position(|member| *member == value.ty)
            });
            // A member of a sum alternative (`AllocError.OutOfMemory` for `or AllocError`) is
            // that alternative.
            if at.is_none()
                && let Some(value) = returned
            {
                for (index, member) in self.proof.variants.clone().into_iter().enumerate() {
                    if let SourceType::Sum { id } = member
                        && self.sum_variants(id)?.contains(&value.ty)
                    {
                        at = Some(index);
                        break;
                    }
                }
            }
            self.proof.exit_variant = at;
            match at {
                Some(0) => binding.contract.as_ref(),
                Some(at) => binding.alternatives[at - 1].value.contract.as_ref(),
                // A value already typed as the sum has no variant to take claims from.
                None => {
                    let claims = binding.contract.iter().chain(
                        binding.alternatives.iter().filter_map(|alt| alt.value.contract.as_ref()),
                    );
                    if claims.count() > 0 {
                        return Err(SourceCheckError::Unsupported);
                    }
                    None
                }
            }
        };
        let Some(contract) = contract else {
            return Ok(());
        };
        let names = self.proof.names.len();
        if let (Some(name), Some(value)) = (&binding.result_binder, returned) {
            self.name(name)?;
            self.proof.names.push((name.value.as_str(), value));
        }
        // The named payload fields of the variant returned are in scope in its claims.
        if let Some(value) = returned {
            let seen = self.owner_view(value)?.ty;
            let fields = self.named_payload_fields(seen)?;
            if !fields.is_empty() {
                let view = self.owner_view(value)?;
                self.materialize_children(view)?;
                let owner = view.owner.ok_or(SourceCheckError::Type)?;
                let SourceType::Aggregate { id } = view.ty else {
                    return Err(SourceCheckError::Type);
                };
                let schema = self.module.instances[id as usize].schema();
                let written = self
                    .module
                    .variant_field_names
                    .iter()
                    .find(|(layout, _)| *layout == schema)
                    .map(|(_, names)| names.clone())
                    .unwrap_or_default();
                for (index, name) in written.into_iter().enumerate() {
                    if name.is_empty() {
                        continue;
                    }
                    if self.proof.names[..names].iter().any(|(other, _)| *other == name) {
                        let detail = format!("the payload field `{name}` has the name of a parameter; rename one");
                        return Err(self.explain(SourceCheckError::Duplicate, detail));
                    }
                    let child = self.proof.owners[owner].children[index];
                    self.proof.names.push((name, child));
                }
            }
        }
        // Claims a caller can assume: over parameters and the named result, one variant only.
        if self.proof.postconditions.is_empty()
            && binding.alternatives.is_empty()
            && binding.result_binder.is_some()
        {
            let names: Vec<&'a str> = self.proof.names.iter().map(|(name, _)| *name).collect();
            let mut post = Vec::new();
            for item in &contract.value.items {
                if let ClauseItem::Claim(RefinementPredicate::Expression(relation)) = &item.value {
                    // Every claim here is proved below; one the caller cannot read is left out.
                    let before = post.len();
                    match self.relation_constraint(relation, &names, &mut post) {
                        Err(SourceCheckError::Budget) => return Err(SourceCheckError::Budget),
                        Ok(()) => {}
                        Err(_) => post.truncate(before),
                    }
                }
            }
            self.proof.postconditions = post;
        }
        for item in &contract.value.items {
            let ClauseItem::Claim(claim) = &item.value else {
                continue;
            };
            match claim {
                RefinementPredicate::Expression(proposition) => {
                    self.child(item.span_id, proposition.span_id)?;
                    self.prove(proposition)?;
                }
                RefinementPredicate::ImplicitComparison { operator, right } => {
                    self.child(item.span_id, operator.span_id)?;
                    self.child(item.span_id, right.span_id)?;
                    self.comparison(operator)?;
                    let result = returned.ok_or(SourceCheckError::Unsupported)?;
                    let right = self.expression(right)?;
                    self.compare(operator.value, result, right)?;
                }
                RefinementPredicate::ImplicitDomain(_) => {
                    return Err(SourceCheckError::Unsupported);
                }
            }
        }
        self.proof.names.truncate(names);
        Ok(())
    }

    fn contract_has_transition(&self, contract: &Spanned<Contract>) -> bool {
        contract
            .value
            .items
            .iter()
            .any(|item| matches!(item.value, ClauseItem::Transition(_)))
    }

    /// A joiner between items is a `,`, or `and` inside parentheses.
    // TODO(header-layout): validate retained newline separators here when wrapped headers land;
    // arbitrary whitespace must not silently become a clause joiner.
    fn joiner(&mut self, span: SpanId) -> Result<()> {
        match self.span(span)? {
            "," | "and" => Ok(()),
            _ => Err(SourceCheckError::Malformed),
        }
    }

    /// The written shape of a variant's `where`: its keyword, parentheses and items.
    fn contract_syntax(&mut self, contract: &'a Spanned<Contract>) -> Result<()> {
        self.span(contract.span_id)?;
        self.keyword(contract.value.where_keyword, "where")?;
        self.child(contract.span_id, contract.value.where_keyword)?;
        for (position, item) in contract.value.items.iter().enumerate() {
            self.span(item.span_id)?;
            self.child(contract.span_id, item.span_id)?;
            if position > 0 {
                self.child(contract.span_id, contract.value.joiners[position - 1])?;
                self.joiner(contract.value.joiners[position - 1])?;
            }
        }
        Ok(())
    }

    /// Each clause must hold where the function returns.
    pub(super) fn contract_exit(&mut self) -> Result<()> {
        let mut clauses = self.proof.clauses.clone();
        // The transitions of the variant returned; a value of the sum itself names none, which
        // `claims_exit` refused already when any variant writes a `where`.
        if self.proof.variant_clauses.iter().any(|clauses| !clauses.is_empty()) {
            let at = self.proof.exit_variant.ok_or(SourceCheckError::Unsupported)?;
            clauses.extend(self.proof.variant_clauses.get(at).cloned().unwrap_or_default());
        }
        for clause in clauses {
            // The place is read by its path, so an accessor written in the clause reads the
            // field it returns.
            let mut base = clause.place;
            loop {
                match &base.value {
                    Expr::Member { base: inner, .. } | Expr::Group(inner) => base = inner,
                    _ => break,
                }
            }
            let root = self.local_place_at(base, &clause.path.fields)?;
            // The path was resolved through accessors already: it is read by position, as the
            // callee's own fields, whatever their visibility here.
            let actual = self.read_at(root, &clause.path.fields)?;
            let actual = self.scalar_facts(actual)?;
            // A place of an open domain holds the number it is.
            let actual = self.open_number(actual)?;
            let actual = self.scalar_facts(actual)?;
            let expected = self.scalar_facts(clause.expected)?;
            let exact = |value: Value| value.lower == value.upper;
            let mut equal = (exact(actual) && exact(expected) && actual.lower == expected.lower)
                || (actual.affine.is_some() && actual.affine == expected.affine);
            // `first + n` written and `first + n` promised: equal by their definitions.
            if !equal
                && let (Some(left), Some(right)) = (actual.identity, expected.identity)
            {
                equal = self.linear_entails(BinaryOperator::Equal, left, right)?;
            }
            if !equal {
                return Err(SourceCheckError::Unproved);
            }
        }
        Ok(())
    }

    /// Remember that the body may change a `mut` parameter's place.
    pub(super) fn record_write(&mut self, root: &str, fields: Vec<String>) -> Result<()> {
        let Some(index) = self.local_position(root)? else {
            return Ok(());
        };
        self.write_guard(root, index, &fields)?;
        if self.value_passing(index) != Some(calls::Passing::Mut) {
            return Ok(());
        }
        let path = PlacePath {
            parameter: index as u32,
            fields,
        };
        if !self.proof.writes.contains(&path) {
            self.charge(1)?;
            self.proof.writes.push(path);
        }
        Ok(())
    }

    /// What this body does to its `mut` parameters, for callers. `None` when it is not
    /// described, and callers forget everything they lent.
    pub(super) fn function_effects(&self) -> Option<Vec<Effect>> {
        if !self.all_values() {
            return None;
        }
        let mut effects: Vec<Effect> = self
            .proof
            .clauses
            .iter()
            .map(|clause| Effect::Set(clause.path.clone(), clause.expr.clone()))
            .collect();
        for write in &self.proof.writes {
            if !self.proof.clauses.iter().any(|clause| clause.path == *write) {
                effects.push(Effect::Forgotten(write.clone()));
            }
        }
        // A place a variant transitions is forgotten at the call and learned where that
        // variant is matched (`variant_effects`).
        for clause in self.proof.variant_clauses.iter().flatten() {
            let forgotten = Effect::Forgotten(clause.path.clone());
            if !effects.contains(&forgotten) {
                effects.push(forgotten);
            }
        }
        Some(effects)
    }

    /// Callers apply the effects of every world the callee was verified in.
    pub(super) fn merge_effects(
        into: &mut Option<Vec<Effect>>,
        other: Option<Vec<Effect>>,
    ) {
        match (into.as_mut(), other) {
            (Some(known), Some(other)) => {
                for effect in other {
                    if !known.contains(&effect) {
                        known.push(effect);
                    }
                }
            }
            _ => *into = None,
        }
    }

    /// What a call did to the places it was lent: apply its effects where they are known,
    /// forget the whole place where they are not.
    pub(super) fn apply_effects(
        &mut self,
        effects: Option<&[Effect]>,
        arguments: &[(&'a Expression, SourceType)],
        passing: &[calls::Passing],
        actual: &[Value],
    ) -> Result<()> {
        let Some(effects) = effects else {
            for ((argument, _), passing) in arguments.iter().zip(passing) {
                if *passing == calls::Passing::Mut && self.is_place(argument)? {
                    self.havoc_place(argument)?;
                }
            }
            return Ok(());
        };
        // Every value is worked out from the state before the call, then all are stored.
        let mut sets = Vec::new();
        for effect in effects {
            if let Effect::Set(path, expr) = effect {
                let mut value = self.clause_value(expr, actual)?;
                // A constant stored has an identity, so the claims of its record (`used =
                // live.length`) relate it to the fields they tie it to, as for any computed value.
                if matches!(expr, ClauseExpr::Constant(_)) {
                    value = self.ensure_identity(value)?;
                }
                sets.push((path, value));
            }
        }
        self.proof.trusting_effects = true;
        let applied = (|| -> Result<()> {
            for effect in effects {
                if let Effect::Forgotten(path) = effect {
                    let place = arguments
                        .get(path.parameter as usize)
                        .ok_or(SourceCheckError::Malformed)?
                        .0;
                    self.havoc_at(place, &path.fields)?;
                }
            }
            for (path, value) in &sets {
                let place = arguments
                    .get(path.parameter as usize)
                    .ok_or(SourceCheckError::Malformed)?
                    .0;
                self.store_at(place, &path.fields, *value)?;
            }
            Ok(())
        })();
        self.proof.trusting_effects = false;
        applied?;
        // The records that held the changed places keep their claims: assumed of the new state.
        for effect in effects {
            let path = match effect {
                Effect::Set(path, _) | Effect::Forgotten(path) => path,
            };
            let place = arguments
                .get(path.parameter as usize)
                .ok_or(SourceCheckError::Malformed)?
                .0;
            self.assume_claims_at(place, &path.fields)?;
        }
        Ok(())
    }

    pub(super) fn clause_value(&mut self, expr: &ClauseExpr, actual: &[Value]) -> Result<Value> {
        self.charge(1)?;
        match expr {
            ClauseExpr::Constant(value) => Ok(Value::integer(*value, *value)),
            ClauseExpr::Place(path) => {
                let argument = *actual
                    .get(path.parameter as usize)
                    .ok_or(SourceCheckError::Malformed)?;
                // A place of an open domain holds the number it is.
                let read = self.read_at(argument, &path.fields)?;
                self.open_number(read)
            }
            ClauseExpr::Binary(operator, left, right) => {
                let left = self.clause_value(left, actual)?;
                let right = self.clause_value(right, actual)?;
                self.arithmetic(*operator, left, right)
            }
            ClauseExpr::Select {
                comparison,
                left,
                right,
                then_branch,
                else_branch,
            } => {
                let left = self.clause_value(left, actual)?;
                let right = self.clause_value(right, actual)?;
                let left = self.ensure_identity(left)?;
                let right = self.ensure_identity(right)?;
                let negated = match comparison {
                    BinaryOperator::Equal => None,
                    BinaryOperator::Less => Some(BinaryOperator::GreaterOrEqual),
                    BinaryOperator::LessOrEqual => Some(BinaryOperator::Greater),
                    BinaryOperator::Greater => Some(BinaryOperator::LessOrEqual),
                    BinaryOperator::GreaterOrEqual => Some(BinaryOperator::Less),
                    _ => return Err(SourceCheckError::Unsupported),
                };
                let then_possible = Self::comparison_possible(*comparison, left, right);
                let else_possible = match negated {
                    Some(negated) => Self::comparison_possible(negated, left, right),
                    None => {
                        !(left.lower == left.upper
                            && right.lower == right.upper
                            && left.lower == right.lower)
                    }
                };
                let mut branches = Vec::new();
                if then_possible {
                    branches.push(true);
                }
                if else_possible {
                    branches.push(false);
                }
                let selected = self.choose(branches.len())?;
                if branches[selected] {
                    self.assume(*comparison, left, right)?;
                    self.clause_value(then_branch, actual)
                } else {
                    if let Some(negated) = negated {
                        self.assume(negated, left, right)?;
                    }
                    self.clause_value(else_branch, actual)
                }
            }
        }
    }
}
