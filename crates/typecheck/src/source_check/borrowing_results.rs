//! Results that borrow: `f(...) ref T` and `f(...) mut T`.
//!
//! The written prefix says the result is a borrow of a `T` place (not a `T`). Which arguments
//! it borrows from is inferred from the verified body (`Summary`-style, see `result_loans`
//! below) and replayed at each call, so the sources stay borrowed while the result is live.
use super::loans::LoanRef;
use super::*;

/// One argument a borrowing result keeps borrowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ResultSource {
    /// Position among the value parameters.
    pub(super) argument: usize,
    pub(super) exclusive: bool,
}

impl<'a> Checker<'a> {
    /// A function's written result: `ref T` / `mut T` records its borrow kind in the proof
    /// state and yields `T`.
    pub(super) fn result_annotation(
        &mut self,
        annotation: &'a Spanned<Annotation>,
    ) -> Result<SourceType> {
        if let Annotation::Expression(expression) = &annotation.value
            && let Expr::PrefixedType(prefix) = &expression.value
            && Self::is_borrow_prefix(prefix)
        {
            self.span(annotation.span_id)?;
            self.child(annotation.span_id, expression.span_id)?;
            self.borrow_prefix_syntax(expression, prefix)?;
            self.proof.result_borrow = Some(if prefix.keyword.value == TypePrefixKind::Mut {
                BorrowKind::Exclusive
            } else {
                BorrowKind::Shared
            });
            self.charge(self.proof.generics.len())?;
            let scope: Vec<_> = self
                .proof
                .generics
                .iter()
                .enumerate()
                .map(|(index, name)| (*name, SourceType::Generic { index: index as u32 }))
                .collect();
            return self
                .resolve_type(&prefix.target, &scope)
                .map_err(|error| match error {
                    SourceCheckError::UnknownName => SourceCheckError::Unsupported,
                    other => other,
                });
        }
        self.annotation_type(annotation)
    }

    /// `ref base.[index]` / `mut base.[index]`: a borrow of one element of a borrowed or
    /// sliced array, accepted when `0 <= index < extent` is proved. The loan is on the root
    /// place of `base`; the element itself is a fresh value that owns nothing.
    pub(super) fn element_handle(
        &mut self,
        expression: &'a Expression,
        select: &'a SelectExpression,
        exclusive: bool,
    ) -> Result<Value> {
        self.child(expression.span_id, select.base.span_id)?;
        self.child(expression.span_id, select.index.span_id)?;
        for (span, token) in [(select.dot, "."), (select.open, "["), (select.close, "]")] {
            self.child(expression.span_id, span)?;
            self.token(span, token)?;
        }
        let index = self.expression(&select.index)?;
        let index = self.domain_number(index)?;
        let index = self.scalar_facts(index)?;
        self.scalar(index)?;
        let array = self.project_place(&select.base)?;
        let SourceType::Array { id } = array.ty else {
            return Err(SourceCheckError::Unsupported);
        };
        let dynamic = matches!(
            self.module.instances[id as usize].head,
            CheckedSourceInstanceHead::Slice | CheckedSourceInstanceHead::Sized
        ) || array
            .owner
            .is_some_and(|owner| self.proof.owners[owner].passing.borrows());
        if !dynamic {
            return Err(SourceCheckError::Unsupported);
        }
        let extent = self.extent_of(array)?;
        if index.lower < I256::ZERO || !self.proves_below(index, extent)? {
            let detail = "cannot prove the index is within the array".to_owned();
            return Err(self.explain(SourceCheckError::Unproved, detail));
        }
        let (element, _) = self.array_layout(array.ty)?;
        let source = self.borrow_handle(&select.base, exclusive)?;
        let Some(handle) = source.borrow else {
            return Err(SourceCheckError::Unsupported);
        };
        let mut value = self.fresh_value(element)?;
        if let Some(owner) = value.owner {
            // The element lives in the lent array, not here: nothing for the body to end,
            // and callers get a fresh stand-in of its type.
            self.proof.owners[owner].drained = true;
            self.proof.owners[owner].passing = if exclusive {
                calls::Passing::Mut
            } else {
                calls::Passing::Ref
            };
        }
        value.borrow = Some(handle);
        Ok(value)
    }
}

impl<'a> Checker<'a> {
    /// A returned handle holds `handle`, a loan on a borrowed parameter: remember which
    /// parameter, so a call keeps that argument borrowed while its result is live.
    pub(super) fn note_result_source(&mut self, handle: LoanRef, exclusive: bool) -> Result<()> {
        let parameters = self.proof.parameter_passing.len().min(self.proof.names.len());
        self.charge(parameters)?;
        let argument = self.proof.names[..parameters]
            .iter()
            .position(|(_, value)| value.owner == Some(handle.owner));
        let Some(argument) = argument else {
            return Ok(());
        };
        match self
            .proof
            .result_sources
            .iter_mut()
            .find(|source| source.argument == argument)
        {
            Some(source) => source.exclusive |= exclusive,
            None => self.proof.result_sources.push(ResultSource {
                argument,
                exclusive,
            }),
        }
        Ok(())
    }

    /// The sources a function publishes: what its body returned loans on; for a function with
    /// no body (an `#instruction`, a trait method) every borrowed argument, as long as the
    /// result is a borrow or a view.
    pub(super) fn published_result_sources(
        &mut self,
        bodiless: bool,
        result: SourceType,
        positions: &[calls::Parameter],
        passing: &[calls::Passing],
    ) -> Vec<ResultSource> {
        let found = std::mem::take(&mut self.proof.result_sources);
        if !bodiless {
            return found;
        }
        if self.proof.result_borrow.is_none() && !self.is_view(result) {
            return Vec::new();
        }
        positions
            .iter()
            .zip(passing)
            .filter(|(position, _)| matches!(position, calls::Parameter::Value(_)))
            .enumerate()
            .filter(|(_, (_, passing))| passing.borrows())
            .map(|(argument, (_, passing))| ResultSource {
                argument,
                exclusive: *passing == calls::Passing::Mut,
            })
            .collect()
    }

    /// After a call: the result keeps its sources borrowed. A source that is itself a view (or
    /// holds handles) passes on copies of those handles; a place is borrowed afresh; a
    /// temporary has nothing to keep borrowed.
    pub(super) fn attach_result_loans(
        &mut self,
        interface: &calls::Function,
        expressions: &[(&'a Expression, SourceType)],
        actual: &[Value],
        result: Value,
    ) -> Result<Value> {
        // A borrow owes nothing: a fresh stand-in for the borrowed place is not ended here.
        if let (Some(kind), Some(owner)) = (interface.result_borrow, result.owner)
            && !actual.iter().any(|argument| argument.owner == Some(owner))
        {
            self.proof.owners[owner].live = false;
            self.proof.owners[owner].passing = match kind {
                BorrowKind::Shared => calls::Passing::Ref,
                BorrowKind::Exclusive => calls::Passing::Mut,
            };
        }
        if interface.result_sources.is_empty() {
            return Ok(result);
        }
        let mut loans: Vec<LoanRef> = Vec::new();
        for source in &interface.result_sources {
            self.charge(2)?;
            let (Some((expression, _)), Some(argument)) =
                (expressions.get(source.argument), actual.get(source.argument))
            else {
                return Err(SourceCheckError::Malformed);
            };
            let inner = self.handles_in(*argument)?;
            let take = |loan: LoanRef, loans: &mut Vec<LoanRef>| {
                if !loans.contains(&loan) {
                    loans.push(loan);
                }
            };
            let exclusive_inner = inner.iter().any(|handle| {
                self.loan_of(*handle).is_some_and(|loan| loan.exclusive)
            });
            for handle in &inner {
                take(*handle, &mut loans);
            }
            // A shared view needs no second hold on its own slot; an exclusive one does.
            if (inner.is_empty() || exclusive_inner) && self.is_place(expression)? {
                let lent = self.borrow_handle(expression, source.exclusive)?;
                if let Some(handle) = lent.borrow {
                    take(handle, &mut loans);
                }
            }
        }
        if loans.is_empty() {
            return Ok(result);
        }
        self.proof.loaned = true;
        Ok(self.give_loans(result, &loans, interface.result_borrow.is_some()))
    }

    /// Hand `loans` to `result`: a borrowing result holds the first and keeps the rest tied to
    /// it; a view holds them in its borrow fields.
    fn give_loans(&mut self, mut result: Value, loans: &[LoanRef], borrowed: bool) -> Value {
        let head = loans[0];
        for tied in &loans[1..] {
            if let Some(loan) = self
                .proof
                .owners
                .get_mut(head.owner)
                .and_then(|owner| owner.lent.get_mut(head.index as usize))
                && !loan.tied.contains(tied)
            {
                loan.tied.push(*tied);
            }
        }
        if borrowed {
            result.borrow = Some(head);
            return result;
        }
        if let SourceType::Aggregate { id } = result.ty
            && let Some(owner) = result.owner
        {
            let schema = self.module.instances[id as usize].schema();
            let fields: Vec<Option<BorrowKind>> = self.module.layouts[schema]
                .schema
                .fields
                .iter()
                .map(|field| field.borrow)
                .collect();
            for (index, kind) in fields.into_iter().enumerate() {
                if kind.is_some()
                    && let Some(child) = self.proof.owners[owner].children.get_mut(index)
                {
                    child.borrow = Some(head);
                }
            }
        }
        result
    }
}
