//! `array.[index] :: value`: a checked write to one element of an array.
//!
//! The index is proved in bounds exactly as a read of the same element is (the read is made
//! first, and it is what refuses a linear element: a write would have to end the old one, and
//! only a capability that may be dropped allows that, as for a field). The array must be held
//! exclusively: a `:` local, a `mut` parameter, or a handle made with `mut`. The value is viewed
//! as the element type. Which element was written is not tracked, so what was known of the
//! array's elements is forgotten. Raw memory behind a pointer is not this: it stays the
//! library's indexed-memory trait and its instructions.
use super::*;

/// The local an indexed place starts at, and the fields between it and the first index.
pub(super) fn indexed_root(place: &Expression) -> Option<(&str, Vec<String>)> {
    match &place.value {
        Expr::Select(select) => indexed_root(&select.base),
        Expr::Group(inner) => indexed_root(inner),
        Expr::Name(name) => Some((name.as_str(), Vec::new())),
        Expr::Member { base, name } => {
            let (root, mut path) = indexed_root(base)?;
            path.push(name.value.clone());
            Some((root, path))
        }
        _ => None,
    }
}

impl<'a> Checker<'a> {
    pub(super) fn element_write(&mut self, assign: &'a AssignStatement) -> Result<()> {
        if assign.compound.is_some() {
            return Err(SourceCheckError::Unsupported);
        }
        let Expr::Select(select) = &assign.place.value else {
            return Err(SourceCheckError::Unsupported);
        };
        if select.length.is_some() {
            // A slice is a view, not an element.
            return Err(SourceCheckError::Unsupported);
        }
        let (root, fields) = indexed_root(&assign.place).ok_or(SourceCheckError::Malformed)?;
        let position = self
            .local_position(root)?
            .ok_or(SourceCheckError::UnknownName)?;
        let held = self.proof.names[position].1;
        let exclusive = self.proof.assignable.contains(&root)
            || held
                .owner
                .is_some_and(|owner| self.proof.owners[owner].passing == calls::Passing::Mut);
        if !exclusive {
            return Err(SourceCheckError::Immutable);
        }
        // A known index into a named array is one of its exact slots.
        if matches!(select.base.value, Expr::Name(_)) {
            let index = self.evaluate(&select.index, None)?;
            let index = self.open_number(index)?;
            let index = self.scalar_facts(index)?;
            self.scalar(index)?;
            if index.lower == index.upper {
                return self.exact_element_write(assign, select, index.lower, root);
            }
        }
        // The read proves the index and shows what is there.
        let old = self.evaluate(&assign.place, None).map_err(|error| match error {
            SourceCheckError::MoveOutOfBorrow => SourceCheckError::OverwriteLinear,
            other => other,
        })?;
        if self.holds_linear(old) {
            return Err(SourceCheckError::OverwriteLinear);
        }
        let element = arrays::array_element_view(old);
        let value = self.evaluate(&assign.value, Some(element))?;
        let value = self.view_as(value, element)?;
        if let Some(owner) = value.owner {
            self.obligate(owner);
        }
        self.record_write(root, fields)?;
        let array = self.project_place(&select.base)?;
        if let Some(owner) = array.owner {
            self.forget_contents(owner);
        }
        Ok(())
    }

    /// Write the slot `index` of a named array: the slot is refilled with the new value.
    fn exact_element_write(
        &mut self,
        assign: &'a AssignStatement,
        select: &'a SelectExpression,
        index: I256,
        root: &str,
    ) -> Result<()> {
        let array = self.array_local(&select.base)?;
        let extent = self.position_count(array.ty)?;
        if index < I256::ZERO || index >= I256::from(extent) {
            return Err(SourceCheckError::Type);
        }
        let slot = index.as_u32();
        let owner = array.owner.ok_or(SourceCheckError::Type)?;
        let element = self.position_type(array.ty, slot)?;
        let old = self.array_child(array, slot)?;
        let taken = self.proof.owners[owner].moved.binary_search(&slot);
        // A slot still holding a linear element would be overwritten, which drops it.
        if taken.is_err() && self.holds_linear(old) {
            return Err(SourceCheckError::OverwriteLinear);
        }
        let value = self.evaluate(&assign.value, Some(element))?;
        let value = self.view_as(value, element)?;
        if let Some(held) = value.owner {
            self.obligate(held);
        }
        self.record_write(root, Vec::new())?;
        let at = self.proof.owners[owner]
            .slots
            .binary_search(&slot)
            .map_err(|_| SourceCheckError::Malformed)?;
        self.proof.owners[owner].children[at] = value;
        if let Ok(position) = self.proof.owners[owner].moved.binary_search(&slot) {
            self.proof.owners[owner].moved.remove(position);
            self.proof.owners[owner].drained = false;
        }
        Ok(())
    }
}
