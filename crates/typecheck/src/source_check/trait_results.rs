//! The result a provided trait gives a method.
//!
//! `Add(result) has add(self, rhs Self) result` leaves the result to whoever provides it:
//! `SignedBigInt has Add(IntegerResult)` says the result of its `add` is `IntegerResult`. A
//! method that leaves its result off takes that type; one that writes a result must write
//! exactly it, so a caller reading the trait sees what the method returns.
use super::*;

impl<'a> Checker<'a> {
    /// The argument written for the trait parameter that `candidate`'s result is, where
    /// `candidate` is the trait's declaration of the method and `arguments` those the type
    /// applied the trait to.
    pub(super) fn trait_result_argument(
        &mut self,
        provided: &'a TypeDeclaration,
        candidate: &'a Binding,
        arguments: &'a [Spanned<Argument>],
    ) -> Result<Option<&'a Expression>> {
        // Written as variants, the result is the trait's sum, which the provider's header must
        // repeat (`provider_result` in `sums.rs`), not a type to resolve.
        if !candidate.alternatives.is_empty() {
            return Ok(None);
        }
        let Some(Spanned {
            value: Annotation::Expression(written),
            ..
        }) = &candidate.annotation
        else {
            return Ok(None);
        };
        // A result the trait writes outright (`Self`, `Bool`) is the same for every provider.
        let (Some(parameters), Expr::Name(result)) = (&provided.parameters, &written.value) else {
            return Ok(Some(written));
        };
        self.charge(parameters.value.len())?;
        let Some(position) = parameters
            .value
            .iter()
            .position(|parameter| parameter.value.name.value == *result)
        else {
            return Ok(Some(written));
        };
        Ok(match arguments.get(position).map(|argument| &argument.value) {
            Some(Argument::Positional(argument)) => Some(argument),
            _ => None,
        })
    }

    /// The method's result once its trait has spoken: the written result must be the trait's,
    /// and an omitted one is the trait's.
    pub(super) fn trait_supplied_result(
        &mut self,
        written: Option<SourceType>,
    ) -> Result<Option<SourceType>> {
        let Some(required) = self.proof.trait_result else {
            return Ok(written);
        };
        let scope = self.generic_scope();
        let required = self.resolve_type(required, &scope)?;
        match written {
            Some(written) if written != required => {
                let detail = format!(
                    "the result must be `{}`, which the provided trait names",
                    self.type_label(required)
                );
                Err(self.explain(SourceCheckError::Type, detail))
            }
            _ => Ok(Some(required)),
        }
    }
}
