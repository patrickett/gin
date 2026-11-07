//! The number an `#unrestricted` number-like domain holds is readable everywhere.
//!
//! A linear domain, or a record with a hidden representation, keeps its number to its own
//! methods (`representation_privacy.rs`). A domain marked `#unrestricted` is a plain number
//! with a name: `buffer.length + 1`, `x in Off: field` and the values of `where` clauses read
//! it as the number it is, with whatever claims have narrowed it.
use super::*;

impl<'a> Checker<'a> {
    /// Whether `ty` is a domain whose number anyone may read.
    pub(super) fn is_open_domain(&self, ty: SourceType) -> bool {
        self.domain_bounds_of(ty).is_some() && self.capability_of(ty) == Capability::Unrestricted
    }

    /// `value` as its number when it is an open domain; any other value unchanged.
    pub(super) fn open_number(&mut self, value: Value) -> Result<Value> {
        if !self.is_open_domain(value.ty) {
            return Ok(value);
        }
        let (lower, upper) = self.domain_bounds_of(value.ty).ok_or(SourceCheckError::Type)?;
        let mut inner = self.field_child(value, 0)?;
        inner = self.scalar_facts(inner)?;
        let (narrow_lower, narrow_upper) = (inner.lower.max(lower), inner.upper.min(upper));
        if narrow_lower <= narrow_upper {
            inner.lower = narrow_lower;
            inner.upper = narrow_upper;
        }
        if !self.claims_of(value.ty).is_empty() {
            inner = self.ensure_identity(inner)?;
            self.seed_claims(inner, value.ty)?;
        }
        Ok(inner)
    }
}
