//! Loans: what `v: ref x` / `v: mut x` mean while the handle `v` is reachable.
//!
//! A handle is a `Value` whose `borrow` names a `Loan` stored on the owner it borrows
//! (`Owner::lent`, so every snapshot of owners carries its loans). A loan has no end event: it
//! is live exactly while some available name, or a value inside one (a view's fields), still
//! holds its handle. The first rule is lexical: names go out of scope with their block, which
//! releases the loans they held and nothing else needs doing.
//!
//! Loans are whole-root: they cover the owner of the local a place starts at, whatever field or
//! range of it was named. A shared loan allows reads and further shared loans; an exclusive
//! loan allows nothing else. Moving, ending, writing or lending `mut` needs no loan at all.
//!
//! A call whose result borrows from an argument gives the result the loans of that argument
//! (inferred from the verified body: `borrowing_results.rs`).
//! TODO(slice 5): non-lexical last-use liveness would ignore names past their last use.
use super::*;

/// What a loan was taken on, which decides whether a returned handle may leave the function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LoanOrigin {
    /// A local, or a parameter taken by value: dies with the function.
    Local,
    /// A `ref`/`mut` parameter: the caller's place, which outlives the call.
    Param,
}

#[derive(Debug, Clone)]
pub(super) struct Loan {
    pub(super) exclusive: bool,
    pub(super) origin: LoanOrigin,
    /// Position of the borrowed local in `proof.names` when the loan was taken. A value that
    /// holds the loan may be stored only in a local declared after it.
    pub(super) depth: usize,
    /// The name of the borrowed local, for messages.
    pub(super) source: String,
    /// The fields below the local that were borrowed (`x.a.b` is `["a", "b"]`; empty for all
    /// of `x`). Two places overlap when one path is a prefix of the other, so a loan on `x.a`
    /// does not block `x.b`.
    pub(super) path: Vec<String>,
    /// Loans that stay borrowed exactly while this one is: a result that borrows from several
    /// arguments holds one handle and ties the others to it.
    pub(super) tied: Vec<LoanRef>,
}

/// Whether two field paths below one local name overlapping places.
fn overlaps(left: &[String], right: &[String]) -> bool {
    left.iter().zip(right).all(|(a, b)| a == b)
}

/// The loan a handle holds: the owner it is stored on and its position there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LoanRef {
    pub(super) owner: usize,
    pub(super) index: u32,
}

/// The live loans on one owner, by kind.
#[derive(Default)]
struct Held {
    shared: Option<String>,
    exclusive: Option<String>,
}

impl<'a> Checker<'a> {
    pub(super) fn loan_of(&self, loan: LoanRef) -> Option<&Loan> {
        self.proof
            .owners
            .get(loan.owner)?
            .lent
            .get(loan.index as usize)
    }

    /// Whether some available name holds a handle for `target`, directly or inside a value.
    fn loan_is_live(&mut self, target: LoanRef) -> Result<bool> {
        self.charge(self.proof.names.len() + 1)?;
        let mut pending: Vec<Value> = self
            .proof
            .names
            .iter()
            .filter(|(_, value)| value.available)
            .map(|(_, value)| *value)
            .collect();
        let mut seen: Vec<usize> = Vec::new();
        let mut held: Vec<LoanRef> = Vec::new();
        while let Some(value) = pending.pop() {
            self.charge(1)?;
            if value.borrow == Some(target) {
                return Ok(true);
            }
            if let Some(handle) = value.borrow
                && !held.contains(&handle)
            {
                held.push(handle);
            }
            // A handle's owner is the source's own, which holds no handles of its own kind.
            if value.borrow.is_some() {
                continue;
            }
            let Some(owner) = value.owner else {
                continue;
            };
            if seen.contains(&owner) {
                continue;
            }
            seen.push(owner);
            self.charge(self.proof.owners[owner].children.len())?;
            pending.extend(self.proof.owners[owner].children.iter().copied());
        }
        // A loan tied to a held one is held with it.
        while let Some(handle) = held.pop() {
            self.charge(1)?;
            let tied = self.loan_of(handle).map(|loan| loan.tied.clone());
            for next in tied.unwrap_or_default() {
                if next == target {
                    return Ok(true);
                }
                if !held.contains(&next) {
                    held.push(next);
                }
            }
        }
        Ok(false)
    }

    /// The live loans on `owner` that overlap the place at `path` below it.
    fn held_at(&mut self, owner: usize, path: &[String]) -> Result<Held> {
        let count = self.proof.owners[owner].lent.len();
        let mut held = Held::default();
        for index in 0..count {
            let target = LoanRef {
                owner,
                index: index as u32,
            };
            let loan = self.proof.owners[owner].lent[index].clone();
            if !overlaps(&loan.path, path) {
                continue;
            }
            let slot = if loan.exclusive {
                &held.exclusive
            } else {
                &held.shared
            };
            if slot.is_some() || !self.loan_is_live(target)? {
                continue;
            }
            if loan.exclusive {
                held.exclusive = Some(loan.source);
            } else {
                held.shared = Some(loan.source);
            }
        }
        Ok(held)
    }

    /// Reading `value` (a local, by name): refused while an exclusive loan on it is live,
    /// unless `value` is that very handle.
    pub(super) fn read_guard(&mut self, value: Value) -> Result<()> {
        self.read_guard_at(value, &[])
    }

    /// [`Self::read_guard`] for the place at `path` below the local.
    pub(super) fn read_guard_at(&mut self, value: Value, path: &[String]) -> Result<()> {
        let Some(owner) = value.owner else {
            return Ok(());
        };
        if value.borrow.is_some() || self.proof.owners[owner].lent.is_empty() {
            return Ok(());
        }
        let held = self.held_at(owner, path)?;
        if let Some(source) = held.exclusive {
            let detail = format!("`{source}` is read while a `mut` borrow of it is still held");
            return Err(self.explain(SourceCheckError::BorrowConflict, detail));
        }
        Ok(())
    }

    /// Moving or ending the local `name`: refused while any loan on it is live.
    pub(super) fn move_guard(&mut self, value: Value, name: &str) -> Result<()> {
        let Some(owner) = value.owner else {
            return Ok(());
        };
        if value.borrow.is_some() || self.proof.owners[owner].lent.is_empty() {
            return Ok(());
        }
        let held = self.held_at(owner, &[])?;
        if held.exclusive.is_some() || held.shared.is_some() {
            let detail = format!("`{name}` cannot be moved or ended while it is borrowed");
            return Err(self.explain(SourceCheckError::BorrowConflict, detail));
        }
        Ok(())
    }

    /// Lending the local `name` (whole or in part), shared or exclusive.
    pub(super) fn lend_guard(
        &mut self,
        value: Value,
        name: &str,
        exclusive: bool,
        path: &[String],
    ) -> Result<()> {
        if let Some(handle) = value.borrow {
            // A reborrow through a handle: a shared handle cannot be lent out exclusively.
            if exclusive && self.loan_of(handle).is_some_and(|loan| !loan.exclusive) {
                let detail = format!("`{name}` is a shared borrow and cannot be lent `mut`");
                return Err(self.explain(SourceCheckError::BorrowConflict, detail));
            }
            return Ok(());
        }
        let Some(owner) = value.owner else {
            return Ok(());
        };
        if self.proof.owners[owner].lent.is_empty() {
            return Ok(());
        }
        let held = self.held_at(owner, path)?;
        if held.exclusive.is_some() {
            let detail =
                format!("`{name}` is already borrowed `mut`; a second borrow would overlap");
            return Err(self.explain(SourceCheckError::BorrowConflict, detail));
        }
        if exclusive && held.shared.is_some() {
            let detail =
                format!("`{name}` is borrowed by a live `ref`; a `mut` borrow would overlap");
            return Err(self.explain(SourceCheckError::BorrowConflict, detail));
        }
        Ok(())
    }

    /// Writing into the local `root`: only through a `mut` handle, and never while a loan on
    /// it is live.
    pub(super) fn write_guard(&mut self, root: &str, index: usize, path: &[String]) -> Result<()> {
        let value = self.proof.names[index].1;
        self.ref_field_guard(root, value.ty, path)?;
        if let Some(handle) = value.borrow {
            if self.loan_of(handle).is_some_and(|loan| !loan.exclusive) {
                let detail = format!("`{root}` is a `ref` borrow and cannot be written through");
                return Err(self.explain(SourceCheckError::Immutable, detail));
            }
            return Ok(());
        }
        let Some(owner) = value.owner else {
            return Ok(());
        };
        if self.proof.owners[owner].lent.is_empty() {
            return Ok(());
        }
        let held = self.held_at(owner, path)?;
        if held.exclusive.is_some() || held.shared.is_some() {
            let detail = format!("`{root}` is changed while a borrow of it is still held");
            return Err(self.explain(SourceCheckError::BorrowConflict, detail));
        }
        Ok(())
    }

    /// Writing through a `ref` field of a view is refused, at any depth of the path.
    fn ref_field_guard(&mut self, root: &str, mut ty: SourceType, path: &[String]) -> Result<()> {
        for name in path {
            let SourceType::Aggregate { id } = ty else {
                return Ok(());
            };
            let Some(layout) = self
                .module
                .instances
                .get(id as usize)
                .and_then(|instance| self.module.layouts.get(instance.schema()))
            else {
                return Ok(());
            };
            let Some(field) = layout
                .schema
                .fields
                .iter()
                .find(|field| field.name == *name)
            else {
                return Ok(());
            };
            // A field that sets the length of an array is fixed once the record is built.
            let position = layout
                .schema
                .fields
                .iter()
                .position(|other| other.name == *name);
            if position.is_some_and(|position| {
                layout
                    .schema
                    .fields
                    .iter()
                    .any(|other| other.length == Some(position as u32))
            }) {
                let detail =
                    format!("`{root}.{name}` sets the length of an array and cannot be assigned");
                return Err(self.explain(SourceCheckError::Immutable, detail));
            }
            if field.borrow == Some(BorrowKind::Shared) {
                let detail =
                    format!("`{root}.{name}` is a `ref` borrow and cannot be written through");
                return Err(self.explain(SourceCheckError::Immutable, detail));
            }
            ty = field.ty;
        }
        Ok(())
    }

    /// `ref place` / `mut place` in a value position: lend the place and keep the loan in the
    /// handle. Integer roots have no owner to lend and stay call-scoped copies.
    pub(super) fn borrow_handle(
        &mut self,
        expression: &'a Expression,
        exclusive: bool,
    ) -> Result<Value> {
        if let Expr::Select(select) = &expression.value
            && select.length.is_some()
        {
            return self.slice_handle(expression, select, exclusive);
        }
        if let Expr::Select(select) = &expression.value {
            return self.element_handle(expression, select, exclusive);
        }
        let mut root = expression;
        while let Expr::Member { base, .. } = &root.value {
            root = base;
        }
        let Expr::Name(name) = &root.value else {
            return Err(SourceCheckError::Unsupported);
        };
        let name: &str = name;
        let index = self
            .local_position(name)?
            .ok_or(SourceCheckError::UnknownName)?;
        // Conflicts first (`borrow_place` also refuses `mut` of a shared place).
        let lent = self.borrow_place(expression, exclusive)?;
        let before = self.proof.names[index].1;
        if let Some(handle) = before.borrow {
            // Reborrow: the new handle holds the same loan.
            return Ok(Value {
                borrow: Some(handle),
                ..lent
            });
        }
        if before.owner.is_none() {
            return Ok(lent);
        }
        if lent.borrow.is_some() {
            // A field that is itself a handle (a view's own borrow): lend through it.
            return Ok(lent);
        }
        if exclusive {
            // A linear record is shared with its handle, so writes through the handle land in
            // the source. A number, or a record that is copied on write, is not: forget what
            // is known of it, since the handle may change it.
            let place = if std::ptr::eq(root, expression) {
                self.proof.names[index].1
            } else {
                self.project_place(expression)?
            };
            let shared = place
                .owner
                .is_some_and(|owner| !self.proof.owners[owner].unrestricted);
            if !shared {
                self.havoc_place(expression)?;
            }
        }
        let root_value = self.proof.names[index].1;
        let owner = root_value.owner.ok_or(SourceCheckError::Type)?;
        let value = if std::ptr::eq(root, expression) {
            root_value
        } else {
            self.project_place(expression)?
        };
        let origin = if self.proof.owners[owner].passing.borrows() {
            LoanOrigin::Param
        } else {
            LoanOrigin::Local
        };
        self.charge(1)?;
        self.proof.loaned = true;
        let position = self.proof.owners[owner].lent.len();
        self.proof.owners[owner].lent.push(Loan {
            exclusive,
            origin,
            depth: index,
            source: name.to_owned(),
            path: sums::place_fields(expression),
            tied: Vec::new(),
        });
        Ok(Value {
            borrow: Some(LoanRef {
                owner,
                index: position as u32,
            }),
            ..value
        })
    }

    /// The loans held by `value`, itself or inside it.
    pub(super) fn handles_in(&mut self, value: Value) -> Result<Vec<LoanRef>> {
        let mut found = Vec::new();
        let mut pending = vec![value];
        let mut seen: Vec<usize> = Vec::new();
        while let Some(next) = pending.pop() {
            self.charge(1)?;
            if let Some(handle) = next.borrow {
                found.push(handle);
                continue;
            }
            let Some(owner) = next.owner else {
                continue;
            };
            if seen.contains(&owner) {
                continue;
            }
            seen.push(owner);
            self.charge(self.proof.owners[owner].children.len())?;
            pending.extend(self.proof.owners[owner].children.iter().copied());
        }
        let mut next = 0;
        while next < found.len() {
            self.charge(1)?;
            let tied = self
                .loan_of(found[next])
                .map(|loan| loan.tied.clone())
                .unwrap_or_default();
            for handle in tied {
                if !found.contains(&handle) {
                    found.push(handle);
                }
            }
            next += 1;
        }
        Ok(found)
    }

    /// A value leaves the function: it may not hold a loan on something the function owns.
    pub(super) fn return_guard(&mut self, value: Value) -> Result<()> {
        // A number read through a handle is a copy of the number.
        if !self.proof.loaned || matches!(value.ty, SourceType::Integer { .. }) {
            return Ok(());
        }
        for handle in self.handles_in(value)? {
            if let Some(loan) = self.loan_of(handle).cloned()
                && loan.origin == LoanOrigin::Local
            {
                let detail = format!(
                    "the returned value borrows local `{}`, which does not outlive it",
                    loan.source
                );
                return Err(self.explain(SourceCheckError::BorrowEscape, detail));
            }
            // A loan on a `ref`/`mut` parameter is legal to return: callers keep the argument
            // it came from borrowed while the result is live.
            if let Some(loan) = self.loan_of(handle).cloned() {
                self.note_result_source(handle, loan.exclusive)?;
            }
        }
        Ok(())
    }

    /// `place :: value` stores `value` where it must not outlive what it borrows: only into a
    /// local declared after the borrowed one, never into a parameter's place.
    pub(super) fn store_guard(&mut self, place: &'a Expression, value: Value) -> Result<()> {
        if !self.proof.loaned || matches!(value.ty, SourceType::Integer { .. }) {
            return Ok(());
        }
        let handles = self.handles_in(value)?;
        if handles.is_empty() {
            return Ok(());
        }
        let (root, _) = assign::split_place(place).ok_or(SourceCheckError::Malformed)?;
        let target = self
            .local_position(root)?
            .ok_or(SourceCheckError::UnknownName)?;
        let parameters = self.proof.parameter_passing.len();
        for handle in handles {
            let Some(loan) = self.loan_of(handle).cloned() else {
                continue;
            };
            if target < parameters || target < loan.depth {
                let detail = format!(
                    "storing a borrow of `{}` in `{root}` would keep it borrowed after `{}` ends",
                    loan.source, loan.source
                );
                return Err(self.explain(SourceCheckError::BorrowEscape, detail));
            }
        }
        Ok(())
    }

    /// A local that is a handle, read as a value. A number is copied; a record that may be
    /// copied is copied; anything else cannot be moved out of a borrow.
    pub(super) fn handle_read(&mut self, value: Value) -> Result<Value> {
        let copy = Value {
            borrow: None,
            ..value
        };
        match value.owner {
            Some(owner) if !self.proof.owners[owner].unrestricted => {
                Err(SourceCheckError::MoveOutOfBorrow)
            }
            _ => Ok(copy),
        }
    }

    /// The handle a `ref` (`exclusive` false) or `mut` field of a borrowing record is given:
    /// `ref place` / `mut place`, a handle already held in a local (moved in; a `mut` handle's
    /// name is gone afterwards), or a place of a borrowed parameter (lent implicitly). Anything
    /// else holds a value, not a borrow.
    pub(super) fn field_handle(
        &mut self,
        expression: &'a Expression,
        exclusive: bool,
    ) -> Result<Value> {
        self.charge(2)?;
        self.span(expression.span_id)?;
        let kind = if exclusive { "`mut`" } else { "`ref`" };
        match &expression.value {
            Expr::Group(inner) => {
                self.group_syntax(expression, inner)?;
                self.field_handle(inner, exclusive)
            }
            Expr::PrefixedType(prefixed) if Self::is_borrow_prefix(prefixed) => {
                let given = prefixed.keyword.value == TypePrefixKind::Mut;
                if exclusive && !given {
                    let detail = format!("a {kind} field needs a `mut` borrow, not a `ref` one");
                    return Err(self.explain(SourceCheckError::Type, detail));
                }
                self.keyword(prefixed.keyword.span_id, if given { "mut" } else { "ref" })?;
                self.child(expression.span_id, prefixed.keyword.span_id)?;
                self.child(expression.span_id, prefixed.target.span_id)?;
                let value = self.borrow_handle(&prefixed.target, given)?;
                let Some(handle) = value.borrow else {
                    // A bare number has no owner to lend.
                    return Err(SourceCheckError::Unsupported);
                };
                // A `mut` borrow given to a `ref` field is used as shared.
                if given && !exclusive {
                    self.downgrade(handle);
                }
                Ok(value)
            }
            Expr::Name(name) => {
                self.token(expression.span_id, name)?;
                let index = self
                    .local_position(name)?
                    .ok_or(SourceCheckError::UnknownName)?;
                let value = self.proof.names[index].1;
                if let Some(handle) = value.borrow {
                    if !value.available {
                        return Err(SourceCheckError::UseAfterMove);
                    }
                    let held = self.loan_of(handle).is_some_and(|loan| loan.exclusive);
                    if exclusive && !held {
                        let detail = format!(
                            "`{name}` is a shared borrow; a {kind} field needs a `mut` one"
                        );
                        return Err(self.explain(SourceCheckError::Type, detail));
                    }
                    if held {
                        // An exclusive handle moves; it is not copied.
                        self.proof.names[index].1.available = false;
                        if !exclusive {
                            self.downgrade(handle);
                        }
                    }
                    return Ok(value);
                }
                self.field_place(expression, name, exclusive)
            }
            Expr::Member { .. } => {
                let mut root = expression;
                while let Expr::Member { base, .. } = &root.value {
                    root = base;
                }
                let Expr::Name(name) = &root.value else {
                    return Err(SourceCheckError::Type);
                };
                self.field_place(expression, name, exclusive)
            }
            _ => {
                let detail = format!("a {kind} field holds a borrow, not a value");
                Err(self.explain(SourceCheckError::Type, detail))
            }
        }
    }

    /// A place of a borrowed parameter (or of a handle) given to a borrow field: lent for it.
    fn field_place(
        &mut self,
        _expression: &'a Expression,
        root: &str,
        exclusive: bool,
    ) -> Result<Value> {
        let index = self
            .local_position(root)?
            .ok_or(SourceCheckError::UnknownName)?;
        let value = self.proof.names[index].1;
        let borrowed = value.borrow.is_some()
            || value
                .owner
                .is_some_and(|owner| self.proof.owners[owner].passing.borrows());
        // A borrow is always written: a place is never lent to a field implicitly, not even a
        // borrowed parameter's.
        let word = if exclusive { "mut" } else { "ref" };
        let detail = if borrowed {
            format!("`{root}` is already borrowed, but the field takes a new borrow: write `{word} {root}`")
        } else {
            format!("`{root}` is a value, but the field holds a borrow: write `{word} {root}`")
        };
        Err(self.explain(SourceCheckError::Type, detail))
    }

    fn downgrade(&mut self, handle: LoanRef) {
        if let Some(loan) = self
            .proof
            .owners
            .get_mut(handle.owner)
            .and_then(|owner| owner.lent.get_mut(handle.index as usize))
        {
            loan.exclusive = false;
        }
    }
}
