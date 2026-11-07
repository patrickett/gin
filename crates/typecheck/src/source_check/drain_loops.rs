//! `for item in items do ...` over an owned array: every element is ended by the body.
//!
//! An array is a language type with no methods, so a record that holds elements which must be
//! ended writes the loop itself (`release(self)` of a library record around the array). The
//! length may be a number not yet known (`[element; count]` inside a generic type), so the
//! body is checked once, for an element about which only its type is known, and must end it
//! and leave every outer owner as it found it; the array is then wholly consumed.
use super::*;

impl<'a> Checker<'a> {
    pub(super) fn drain_loop(
        &mut self,
        binder: &'a str,
        body: &'a Expression,
        array: Value,
    ) -> Result<()> {
        self.charge(8)?;
        let (element, _) = self.array_layout(array.ty)?;
        self.array_complete(array)?;
        let owner = array.owner.ok_or(SourceCheckError::Type)?;
        // Only the owner can end the elements: a borrowed array cannot be drained.
        if self.proof.owners[owner].passing.borrows() {
            return Err(SourceCheckError::MoveOutOfBorrow);
        }
        self.obligate(owner);
        self.forget_loop_state()?;
        let names = self.proof.names.clone();
        let facts = self.proof.facts.clone();
        let scalars = self.proof.scalars.clone();
        let guards = self.proof.integer_guards.clone();
        let assignable = self.proof.assignable.clone();
        let owners_before = self.proof.owners.len();
        let entry: Vec<(bool, usize, bool)> = self
            .proof
            .owners
            .iter()
            .map(|owner| (owner.live, owner.moved.len(), owner.drained))
            .collect();
        self.charge(names.len() + entry.len())?;

        let item = self.fresh_value(element)?;
        if let Some(item_owner) = item.owner {
            self.obligate(item_owner);
        }
        self.insert(binder, item)?;
        let result = self.loop_body(body)?;
        if self.proof.returned {
            self.proof.returned_value = Some(result);
            return Ok(());
        }
        if let Some(owner) = result.owner {
            self.obligate(owner);
        }
        self.loop_pass_obligations(&names, &entry, owners_before)?;

        self.proof.names = names;
        self.proof.facts = facts;
        self.proof.scalars = scalars;
        self.proof.integer_guards = guards;
        self.proof.assignable = assignable;
        self.proof.owners[owner].drained = true;
        self.release(array)?;
        self.forget_loop_state()
    }
}
