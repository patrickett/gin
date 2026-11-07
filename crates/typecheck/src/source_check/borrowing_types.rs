//! Borrowing types: a record with a `ref`/`mut` field is a view.
//!
//! `data ref T` / `data mut T` stay type *prefixes* on a field; the field's `ty` is `T` and
//! `CheckedSourceField::borrow` says it holds a borrow of a `T` place instead of a `T`. A view
//! owns nothing it borrows, so it carries no obligation (`Owner::view`) and may be dropped.
//! It may be copied only when `#unrestricted`, which every field must then permit: a `ref`
//! field does, a `mut` field never does (copying would duplicate the exclusive handle).
use super::*;

impl<'a> Checker<'a> {
    // TODO(group-contract): define signature-local group binding, supporting-access dependencies,
    // coordinated mutable aliases and retargeting before accepting grouped borrowed types.
    // Escaping result groups need an unambiguous origin rule; a group name alone grants no authority.
    pub(super) fn is_borrow_prefix(prefix: &PrefixedType) -> bool {
        matches!(
            prefix.keyword.value,
            TypePrefixKind::Ref | TypePrefixKind::Mut
        ) && prefix.group.is_none()
    }

    /// The keyword and operand of a `ref T` / `mut T` field type, with their spans in place.
    pub(super) fn borrow_prefix_syntax(
        &mut self,
        expression: &'a Expression,
        prefix: &'a PrefixedType,
    ) -> Result<()> {
        self.child(expression.span_id, prefix.keyword.span_id)?;
        self.child(expression.span_id, prefix.target.span_id)?;
        let word = if prefix.keyword.value == TypePrefixKind::Mut {
            "mut"
        } else {
            "ref"
        };
        self.keyword(prefix.keyword.span_id, word)?;
        let range = self.range_of(expression.span_id);
        let keyword = self.range_of(prefix.keyword.span_id);
        let target = self.range_of(prefix.target.span_id);
        if range.start != keyword.start || range.end != target.end {
            return Err(SourceCheckError::Malformed);
        }
        self.syntax_trivia(target.file, keyword.end, target.start)
    }

    /// A field's declared type, the borrow it holds when written `ref T` / `mut T`, and the
    /// sibling field that is its length when written `[element; len]`.
    pub(super) fn resolve_field_annotation(
        &mut self,
        annotation: &'a Spanned<Annotation>,
        scope: &[(&'a str, SourceType)],
        siblings: &[&str],
    ) -> Result<(SourceType, Option<BorrowKind>, Option<u32>)> {
        let mut borrow = None;
        let mut written = None;
        if let Annotation::Expression(expression) = &annotation.value {
            written = Some(expression);
            if let Expr::PrefixedType(prefix) = &expression.value
                && Self::is_borrow_prefix(prefix)
            {
                self.span(annotation.span_id)?;
                self.child(annotation.span_id, expression.span_id)?;
                self.borrow_prefix_syntax(expression, prefix)?;
                self.charge(1)?;
                borrow = Some(if prefix.keyword.value == TypePrefixKind::Mut {
                    BorrowKind::Exclusive
                } else {
                    BorrowKind::Shared
                });
                written = Some(&prefix.target);
            }
        }
        if let Some(target) = written {
            // Only an array whose length names a sibling needs its own resolution.
            if Self::names_sibling_length(target, scope, siblings) {
                if borrow.is_none() {
                    self.span(annotation.span_id)?;
                    self.child(annotation.span_id, target.span_id)?;
                }
                if let Some((ty, length)) = self.dependent_array(target, scope, siblings)? {
                    // An array whose length is a field's value has no size of its own, so it
                    // can only be reached through a borrow of storage somewhere else.
                    if borrow.is_none() {
                        let detail = "an array with a dynamic length cannot be owned: write `ref` or `mut` before it".to_owned();
                        return Err(self.explain(SourceCheckError::Type, detail));
                    }
                    return Ok((ty, borrow, Some(length)));
                }
            }
            if borrow.is_some() {
                let ty = self
                    .resolve_type(target, scope)
                    .map_err(|error| match error {
                        SourceCheckError::UnknownName => SourceCheckError::Unsupported,
                        other => other,
                    })?;
                return Ok((ty, borrow, None));
            }
        }
        Ok((self.resolve_annotation(annotation, scope)?, None, None))
    }

    pub(super) fn borrowing_of(fields: &[CheckedSourceField]) -> Borrowing {
        let shared = fields.iter().any(|f| f.borrow == Some(BorrowKind::Shared));
        let exclusive = fields
            .iter()
            .any(|f| f.borrow == Some(BorrowKind::Exclusive));
        match (shared, exclusive) {
            (false, false) => Borrowing::None,
            (true, false) => Borrowing::Shared,
            (false, true) => Borrowing::Exclusive,
            (true, true) => Borrowing::Mixed,
        }
    }

    /// Whether values of `ty` are views: a record that borrows (itself, or through a field that
    /// is a view) and whose other fields are all unrestricted, so it owns nothing that must be
    /// ended.
    pub(super) fn is_view(&self, ty: SourceType) -> bool {
        self.view_within(ty, 8) == Some(true)
    }

    /// `Some(borrows)` for a record whose fields all own nothing, else `None`.
    fn view_within(&self, ty: SourceType, depth: u32) -> Option<bool> {
        let SourceType::Aggregate { id } = ty else {
            return None;
        };
        let layout = self
            .module
            .instances
            .get(id as usize)
            .and_then(|instance| self.module.layouts.get(instance.schema()))?;
        if depth == 0 {
            return None;
        }
        let mut borrows = layout.schema.borrowing != Borrowing::None;
        for field in &layout.schema.fields {
            if field.borrow.is_some() {
                continue;
            }
            match self.view_within(field.ty, depth - 1) {
                // A field that is itself a view owns nothing either.
                Some(true) => borrows = true,
                _ if self.capability_of(field.ty) == Capability::Unrestricted => {}
                _ => return None,
            }
        }
        Some(borrows)
    }

    /// What a view's borrow fields hold is borrowed with it: never owned by the view.
    pub(super) fn mark_borrowed_children(&mut self, id: u32, children: &[Value]) {
        let schema = self.module.instances[id as usize].schema();
        if self.module.layouts[schema].schema.borrowing == Borrowing::None {
            return;
        }
        for (index, child) in children.iter().enumerate() {
            let kind = self.module.layouts[schema].schema.fields[index].borrow;
            if let (Some(kind), Some(owner)) = (kind, child.owner) {
                self.proof.owners[owner].passing = match kind {
                    BorrowKind::Shared => calls::Passing::Ref,
                    BorrowKind::Exclusive => calls::Passing::Mut,
                };
            }
        }
    }

    /// An `#unrestricted` record may not hold an exclusive borrow: copying it would duplicate
    /// the handle.
    pub(super) fn exclusive_field_offender(&mut self, schema: usize) -> Option<SourceCheckError> {
        let layout = &self.module.layouts[schema].schema;
        let field = layout
            .fields
            .iter()
            .find(|field| field.borrow == Some(BorrowKind::Exclusive))?;
        let detail = format!(
            "`{}` is `#unrestricted` but `{}` holds an exclusive borrow, which cannot be copied",
            layout.name, field.name
        );
        Some(self.explain(SourceCheckError::UnrestrictedField, detail))
    }
}
