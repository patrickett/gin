//! Reassignment: `place :: value` stores into a mutable local, or a field path of one, and
//! a `mut` call makes the caller forget what the callee may have changed.
//!
//! An unrestricted record is shared by every copy of it, so writing into one is
//! copy-on-write: the owner on the path is cloned first and only the writer's name sees the
//! change. A linear record has exactly one holder and is changed in place. A linear value is
//! never overwritten, since that would drop it; only unrestricted places can be assigned.
use super::*;

/// A place `a.b.c` taken apart: its root local and the fields below it.
pub(super) fn split_place(place: &Expression) -> Option<(&str, Vec<&Name>)> {
    match &place.value {
        Expr::Name(name) => Some((name.as_str(), Vec::new())),
        Expr::Member { base, name } => {
            let (root, mut path) = split_place(base)?;
            path.push(name);
            Some((root, path))
        }
        Expr::Group(inner) => split_place(inner),
        _ => None,
    }
}

impl<'a> Checker<'a> {
    /// The type of what a place holds, which a value stored there must fit: a local's
    /// declared range or type, or a field's declared type.
    /// A place can only be reassigned through a `mut` parameter or a `:` local, whatever
    /// the value it is given; this is the first thing to say about an assignment.
    pub(super) fn require_assignable(&mut self, place: &'a Expression) -> Result<()> {
        // An element of an array or of memory behind a pointer is a place, but writing one
        // needs the memory model (what a pointer is, and what aliases it), which is not built.
        if Self::place_is_indexed(place) {
            return Err(SourceCheckError::Unsupported);
        }
        let (root, _) = split_place(place).ok_or(SourceCheckError::Malformed)?;
        self.local_position(root)?
            .ok_or(SourceCheckError::UnknownName)?;
        if self.proof.assignable.contains(&root) {
            Ok(())
        } else {
            Err(SourceCheckError::Immutable)
        }
    }

    /// Whether a place steps through `.[index]` anywhere.
    pub(super) fn place_is_indexed(place: &Expression) -> bool {
        let mut next = place;
        loop {
            match &next.value {
                Expr::Select(_) => return true,
                Expr::Member { base, .. } => next = base,
                Expr::Group(inner) => next = inner,
                _ => return false,
            }
        }
    }

    pub(super) fn place_declared_type(&mut self, place: &'a Expression) -> Result<SourceType> {
        match &place.value {
            Expr::Name(name) => {
                self.span(place.span_id)?;
                self.token(place.span_id, name)?;
                let index = self
                    .local_position(name)?
                    .ok_or(SourceCheckError::UnknownName)?;
                let value = self.proof.names[index].1;
                Ok(match (value.ty, value.integer_domain) {
                    (SourceType::Integer { .. }, Some((lower, upper))) => {
                        SourceType::Integer { lower, upper }
                    }
                    _ => value.ty,
                })
            }
            Expr::Member { base, name } => {
                self.span(place.span_id)?;
                self.child(place.span_id, base.span_id)?;
                self.child(place.span_id, name.span_id)?;
                self.name(name)?;
                // Only the place below the base is read, so a loan on another field is no obstacle.
                let parent = self.project_place_at(base, &sums::place_fields(place))?;
                let parent = self.owner_view(parent)?;
                let SourceType::Aggregate { id } = parent.ty else {
                    return Err(SourceCheckError::Type);
                };
                // Reading validates the name, privacy, and the hidden number of a domain.
                self.projection_value(parent, name)?;
                let index = self.field_index(id, &name.value)?;
                Ok(self.instance_fields(id)?[index])
            }
            Expr::Group(inner) => {
                self.group_syntax(place, inner)?;
                self.place_declared_type(inner)
            }
            _ => Err(SourceCheckError::Unsupported),
        }
    }

    pub(super) fn field_index(&mut self, id: u32, name: &str) -> Result<usize> {
        let schema = self.module.instances[id as usize].schema();
        self.charge_product(
            self.module.layouts[schema].schema.fields.len(),
            name.len() + 1,
        )?;
        self.module.layouts[schema]
            .schema
            .fields
            .iter()
            .position(|field| field.name == name)
            .ok_or(SourceCheckError::UnknownName)
    }

    pub(super) fn holds_linear(&self, value: Value) -> bool {
        value
            .owner
            .is_some_and(|owner| !self.proof.owners[owner].unrestricted)
    }

    pub(super) fn clone_owner(&mut self, owner: usize) -> Result<usize> {
        self.charge_flow()?;
        let mut copy = self.proof.owners[owner].clone();
        // Loans are taken on the original; a copy is a different value.
        copy.lent.clear();
        self.proof.owners.push(copy);
        Ok(self.proof.owners.len() - 1)
    }

    /// `left op right` for the compound assignment `place op: right`, by the same rules as
    /// the binary expression: a nominal left operand uses its type's `#infix` method, an
    /// integer is computed exactly (`+ - *`).
    pub(super) fn compound_value(
        &mut self,
        operator: BinaryOperator,
        left: Value,
        right: Value,
    ) -> Result<Value> {
        let infix = InfixOperator::from_binary(operator).ok_or(SourceCheckError::Malformed)?;
        if matches!(left.ty, SourceType::Aggregate { .. }) {
            let function = self
                .find_infix(left.ty, infix)?
                .ok_or(SourceCheckError::Type)?;
            return self.apply_call(function, vec![left, right]);
        }
        let arithmetic = infix.arithmetic().ok_or(SourceCheckError::Unsupported)?;
        self.arithmetic(arithmetic, left, right)
    }

    /// Store the evaluated `value` at the assignment's place.
    pub(super) fn assign(&mut self, assign: &'a AssignStatement, value: Value) -> Result<()> {
        let (root, path) = split_place(&assign.place).ok_or(SourceCheckError::Malformed)?;
        let index = self
            .local_position(root)?
            .ok_or(SourceCheckError::UnknownName)?;
        if !self.proof.assignable.contains(&root) {
            return Err(SourceCheckError::Immutable);
        }
        let current = self.proof.names[index].1;
        if current.owner.is_some() && !current.available {
            return Err(SourceCheckError::UseAfterMove);
        }
        self.record_write(root, path.iter().map(|field| field.value.clone()).collect())?;
        let Some((last, parents)) = path.split_last() else {
            // The whole local is replaced.
            if self.holds_linear(current) {
                return Err(SourceCheckError::OverwriteLinear);
            }
            let mut stored = value;
            if matches!(current.ty, SourceType::Integer { .. }) {
                stored.integer_domain = current.integer_domain;
            }
            stored.available = true;
            self.proof.names[index].1 = stored;
            return Ok(());
        };
        let mut holder = self.writable_root(index)?;
        for field in parents {
            holder = self.writable_child(holder, field)?;
        }
        self.store_field(holder, last, value)
    }

    /// The root local as a record whose owner may be changed without anyone else seeing it.
    fn writable_root(&mut self, index: usize) -> Result<Value> {
        let value = self.proof.names[index].1;
        let SourceType::Aggregate { .. } = value.ty else {
            return Err(SourceCheckError::Type);
        };
        self.materialize_children(value)?;
        let owner = value.owner.ok_or(SourceCheckError::Type)?;
        if self.proof.owners[owner].unrestricted {
            let fresh = self.clone_owner(owner)?;
            let rebound = Value {
                owner: Some(fresh),
                ..value
            };
            self.proof.names[index].1 = rebound;
            return Ok(rebound);
        }
        Ok(value)
    }

    /// Step into a record field that is itself a record, ready to be written.
    fn writable_child(&mut self, holder: Value, field: &Name) -> Result<Value> {
        // Reading validates the name, privacy, and the hidden number of a domain.
        self.projection_value(holder, field)?;
        self.writable_child_by(holder, &field.value)
    }

    fn writable_child_by(&mut self, holder: Value, field: &str) -> Result<Value> {
        let SourceType::Aggregate { id } = holder.ty else {
            return Err(SourceCheckError::Type);
        };
        let index = self.field_index(id, field)?;
        let owner = holder.owner.ok_or(SourceCheckError::Type)?;
        let child = self.proof.owners[owner].children[index];
        let SourceType::Aggregate { .. } = child.ty else {
            return Err(SourceCheckError::Type);
        };
        self.materialize_children(child)?;
        let child_owner = child.owner.ok_or(SourceCheckError::Type)?;
        if self.proof.owners[child_owner].unrestricted {
            let fresh = self.clone_owner(child_owner)?;
            let rebound = Value {
                owner: Some(fresh),
                ..child
            };
            self.proof.owners[owner].children[index] = rebound;
            return Ok(rebound);
        }
        Ok(child)
    }

    /// Replace one field, then prove the record's own constraints still hold.
    fn store_field(&mut self, holder: Value, field: &Name, value: Value) -> Result<()> {
        self.projection_value(holder, field)?;
        self.store_field_by(holder, &field.value, value)
    }

    fn store_field_by(&mut self, holder: Value, field: &str, value: Value) -> Result<()> {
        let SourceType::Aggregate { id } = holder.ty else {
            return Err(SourceCheckError::Type);
        };
        let index = self.field_index(id, field)?;
        let owner = holder.owner.ok_or(SourceCheckError::Type)?;
        if self.holds_linear(self.proof.owners[owner].children[index]) {
            return Err(SourceCheckError::OverwriteLinear);
        }
        let declared = self.instance_fields(id)?[index];
        let mut stored = value;
        if let SourceType::Integer { lower, upper } = declared {
            stored.integer_domain = Some((lower, upper));
        }
        stored.available = true;
        self.proof.owners[owner].children[index] = stored;
        let mut children = self.proof.owners[owner].children.clone();
        // A callee's effects leave the record's invariants holding (the callee checked its own
        // writes), so they are assumed of the new state rather than proved field by field.
        // Applying a callee's effects, the claims are assumed once every effect is stored.
        if !self.proof.trusting_effects {
            self.field_requirements(id, &mut children, false)?;
        }
        Ok(())
    }

    /// After a `mut` call the callee may have changed anything of the place that its type
    /// allows, so what the caller knew of it is dropped. Nothing says what changed yet.
    pub(super) fn havoc_place(&mut self, place: &'a Expression) -> Result<()> {
        let Some((root, path)) = split_place(place) else {
            return Ok(());
        };
        let Some(index) = self.local_position(root)? else {
            return Ok(());
        };
        self.record_write(root, path.iter().map(|field| field.value.clone()).collect())?;
        match path.split_last() {
            None => self.havoc_local(index),
            Some((last, parents)) => {
                let mut holder = self.writable_root(index)?;
                for field in parents {
                    holder = self.writable_child(holder, field)?;
                }
                self.havoc_field(holder, last)
            }
        }
    }

    pub(super) fn havoc_local(&mut self, index: usize) -> Result<()> {
        let value = self.proof.names[index].1;
        match value.ty {
            SourceType::Integer { lower, upper } => {
                let (lower, upper) = value.integer_domain.unwrap_or((lower, upper));
                let mut fresh = self.fresh_value(SourceType::Integer { lower, upper })?;
                fresh.integer_domain = Some((lower, upper));
                self.proof.names[index].1 = fresh;
            }
            SourceType::Aggregate { .. } => {
                let owner = value.owner.ok_or(SourceCheckError::Type)?;
                if self.proof.owners[owner].unrestricted {
                    self.proof.names[index].1 = self.input_value(value.ty)?;
                } else if self.holds_generic(owner, 0) {
                    self.forget_keeping_values(value, 0)?;
                } else {
                    self.forget_contents(owner);
                }
            }
            SourceType::Unit => {}
            // A value of a type parameter is opaque: after a `mut` call nothing more is known of
            // it than before, apart from what its owner held, which is forgotten.
            SourceType::Generic { .. } => {
                if let Some(owner) = value.owner {
                    self.forget_contents(owner);
                }
            }
            _ => return Err(SourceCheckError::Unsupported),
        }
        Ok(())
    }

    fn havoc_field(&mut self, holder: Value, field: &Name) -> Result<()> {
        self.projection_value(holder, field)?;
        self.havoc_field_by(holder, &field.value)
    }

    fn havoc_field_by(&mut self, holder: Value, field: &str) -> Result<()> {
        let SourceType::Aggregate { id } = holder.ty else {
            return Err(SourceCheckError::Type);
        };
        let index = self.field_index(id, field)?;
        let owner = holder.owner.ok_or(SourceCheckError::Type)?;
        let declared = self.instance_fields(id)?[index];
        let old = self.proof.owners[owner].children[index];
        let fresh = match declared {
            SourceType::Integer { lower, upper } => {
                let mut fresh = self.fresh_value(declared)?;
                fresh.integer_domain = Some((lower, upper));
                fresh
            }
            SourceType::Aggregate { .. } => {
                let child_owner = old.owner.ok_or(SourceCheckError::Type)?;
                if !self.proof.owners[child_owner].unrestricted {
                    // The same linear value, with its contents no longer known; the record's
                    // claims about them (`used = live.length`) are known to hold again.
                    self.forget_contents(child_owner);
                    // Applying a callee's effects, the claims are assumed once all are stored.
                    if !self.proof.trusting_effects {
                        let mut children = self.proof.owners[owner].children.clone();
                        self.field_requirements(id, &mut children, true)?;
                        self.proof.owners[owner].children = children;
                    }
                    return Ok(());
                }
                self.input_value(declared)?
            }
            // An array the callee wrote: the same array, whose elements are no longer known.
            SourceType::Array { .. } => {
                let child_owner = old.owner.ok_or(SourceCheckError::Type)?;
                self.forget_contents(child_owner);
                return Ok(());
            }
            _ => return Err(SourceCheckError::Unsupported),
        };
        self.proof.owners[owner].children[index] = fresh;
        // A callee keeps the record's own invariants, so they are known to hold again (once
        // every effect of the call is stored, when effects are being applied).
        if !self.proof.trusting_effects {
            let mut children = self.proof.owners[owner].children.clone();
            self.field_requirements(id, &mut children, true)?;
            self.proof.owners[owner].children = children;
        }
        Ok(())
    }

    /// The value at `fields` below `value`.
    pub(super) fn read_at(&mut self, mut value: Value, fields: &[String]) -> Result<Value> {
        for name in fields {
            value = self.owner_view(value)?;
            let SourceType::Aggregate { id } = value.ty else {
                return Err(SourceCheckError::Type);
            };
            let index = self.field_index(id, name)?;
            self.materialize_children(value)?;
            let owner = value.owner.ok_or(SourceCheckError::Type)?;
            value = self.proof.owners[owner].children[index];
        }
        Ok(value)
    }

    /// The value a lent place holds below `below`, given an identity (stored back) so facts
    /// assumed of it later reach the place.
    pub(super) fn lent_value(&mut self, place: &'a Expression, below: &[String]) -> Result<Value> {
        let (_, index, names) = self.lent_place(place, below)?;
        let root = self.proof.names[index].1;
        let value = self.read_at(root, &names)?;
        if value.identity.is_some() || !matches!(value.ty, SourceType::Integer { .. }) {
            return Ok(value);
        }
        let value = self.ensure_identity(value)?;
        self.proof.trusting_effects = true;
        let stored = self.store_at(place, below, value);
        self.proof.trusting_effects = false;
        stored?;
        Ok(value)
    }

    /// A lent place and the fields below it, as the local it starts at and every field name.
    fn lent_place(
        &mut self,
        place: &'a Expression,
        below: &[String],
    ) -> Result<(&'a str, usize, Vec<String>)> {
        let (root, path) = split_place(place).ok_or(SourceCheckError::Malformed)?;
        let index = self
            .local_position(root)?
            .ok_or(SourceCheckError::UnknownName)?;
        let mut names: Vec<String> = path.iter().map(|field| field.value.clone()).collect();
        names.extend(below.iter().cloned());
        Ok((root, index, names))
    }

    fn writable_holder(&mut self, index: usize, parents: &[String]) -> Result<Value> {
        let mut holder = self.writable_root(index)?;
        for field in parents {
            holder = self.writable_child_by(holder, field)?;
        }
        Ok(holder)
    }

    /// Store `value` in a place lent `mut` (and the fields below it): what a call that was
    /// lent it said it left there.
    pub(super) fn store_at(
        &mut self,
        place: &'a Expression,
        below: &[String],
        value: Value,
    ) -> Result<()> {
        let (root, index, names) = self.lent_place(place, below)?;
        self.record_write(root, names.clone())?;
        let Some((last, parents)) = names.split_last() else {
            let current = self.proof.names[index].1;
            let declared = match (current.ty, current.integer_domain) {
                (SourceType::Integer { .. }, Some((lower, upper))) => {
                    SourceType::Integer { lower, upper }
                }
                (ty, _) => ty,
            };
            let mut stored = self.view_as(value, declared)?;
            if matches!(current.ty, SourceType::Integer { .. }) {
                stored.integer_domain = current.integer_domain;
            }
            stored.available = true;
            self.proof.names[index].1 = stored;
            return Ok(());
        };
        let holder = self.writable_holder(index, parents)?;
        let SourceType::Aggregate { id } = holder.ty else {
            return Err(SourceCheckError::Type);
        };
        let field = self.field_index(id, last)?;
        let declared = self.instance_fields(id)?[field];
        let value = self.view_as(value, declared)?;
        self.store_field_by(holder, last, value)
    }

    /// Forget a place lent `mut` (and the fields below it).
    pub(super) fn havoc_at(&mut self, place: &'a Expression, below: &[String]) -> Result<()> {
        let (root, index, names) = self.lent_place(place, below)?;
        self.record_write(root, names.clone())?;
        match names.split_last() {
            None => self.havoc_local(index),
            Some((last, parents)) => {
                let holder = self.writable_holder(index, parents)?;
                self.havoc_field_by(holder, last)
            }
        }
    }

    /// Whether a materialized record holds, at some depth, a value of a type parameter (the
    /// `allocator alloc` of a list).
    fn holds_generic(&self, owner: usize, depth: u32) -> bool {
        const DEPTH_MAX: u32 = 6;
        let record = &self.proof.owners[owner];
        if depth > DEPTH_MAX || !record.materialized || !record.moved.is_empty() || record.active.is_some() {
            return false;
        }
        record.children.iter().any(|child| match child.ty {
            SourceType::Generic { .. } => true,
            SourceType::Aggregate { .. } => child.owner.is_some_and(|inner| self.holds_generic(inner, depth + 1)),
            _ => false,
        })
    }

    /// What a loop may change in a record is forgotten, but the values it holds stay the same
    /// values: a value of a type parameter cannot be made anew, so it must still be the one that
    /// came in (an input the function hands on or ends). Numbers become unknown in their
    /// declared range, linear contents keep their identity with their own contents forgotten,
    /// and the record's claims are assumed again.
    fn forget_keeping_values(&mut self, record: Value, depth: u32) -> Result<()> {
        let owner = record.owner.ok_or(SourceCheckError::Type)?;
        let SourceType::Aggregate { id } = record.ty else {
            return Err(SourceCheckError::Type);
        };
        let declared = self.instance_fields(id)?;
        let mut children = self.proof.owners[owner].children.clone();
        if declared.len() != children.len() {
            self.forget_contents(owner);
            return Ok(());
        }
        self.charge(children.len())?;
        for (child, ty) in children.iter_mut().zip(declared) {
            match child.ty {
                SourceType::Integer { .. } => {
                    let (lower, upper) = match ty {
                        SourceType::Integer { lower, upper } => (lower, upper),
                        _ => return Err(SourceCheckError::Type),
                    };
                    let mut fresh = self.fresh_value(ty)?;
                    fresh.integer_domain = Some((lower, upper));
                    *child = fresh;
                }
                _ => match child.owner {
                    Some(inner) if !self.proof.owners[inner].unrestricted => {
                        if matches!(child.ty, SourceType::Aggregate { .. }) && self.holds_generic(inner, depth + 1) {
                            self.forget_keeping_values(*child, depth + 1)?;
                        } else {
                            self.forget_contents(inner);
                        }
                    }
                    _ => *child = self.input_value(ty)?,
                },
            }
        }
        self.field_requirements(id, &mut children, true)?;
        self.proof.owners[owner].children = children;
        Ok(())
    }

    pub(super) fn forget_contents(&mut self, owner: usize) {
        let owner = &mut self.proof.owners[owner];
        owner.children.clear();
        owner.slots.clear();
        owner.moved.clear();
        owner.materialized = false;
        owner.active = None;
    }
}

impl<'a> Checker<'a> {
    /// After a call's effects are stored: each record the callee was lent satisfies its own
    /// claims again (the callee checked every write it made), so they are assumed.
    pub(super) fn assume_claims_at(&mut self, place: &'a Expression, below: &[String]) -> Result<()> {
        let (_, index, names) = self.lent_place(place, below)?;
        let Some((_, parents)) = names.split_last() else {
            return Ok(());
        };
        // Only the records the callee was lent: the argument and what is inside it. A record
        // that merely encloses the argument (`self` around `self.vacant`) may be mid-change in
        // the caller, so its claims are not assumed here; the caller proves them where it writes.
        let lent = names.len() - below.len();
        for depth in lent..=parents.len() {
            let holder = self.writable_holder(index, &parents[..depth])?;
            let SourceType::Aggregate { id } = holder.ty else {
                continue;
            };
            let owner = holder.owner.ok_or(SourceCheckError::Type)?;
            self.materialize_children(holder)?;
            let mut children = self.proof.owners[owner].children.clone();
            self.field_requirements(id, &mut children, true)?;
            self.proof.owners[owner].children = children;
        }
        Ok(())
    }
}
