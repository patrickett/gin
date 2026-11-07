use crate::{Expression, Refinement, RefinementPredicate, SpanId, Spanned, TypeDomain};

/// A written value, return, or parameter annotation, never a semantic type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Annotation {
    Expression(Expression),
    Domain {
        /// Absent in canonical `in` headers; retained for recovered `is in` source.
        is_keyword: Option<SpanId>,
        domain: Spanned<TypeDomain>,
    },
    /// Producers retain at least one written refinement.
    Refined {
        /// Absent for bare `and` preconditions; no nominal base is synthesized.
        base: Option<Expression>,
        refinements: Vec<Spanned<Refinement>>,
    },
    /// `has Trait`: a parameter whose type is inferred and must provide the trait.
    Bound {
        has_keyword: SpanId,
        bound: Expression,
    },
    Predicate {
        is_keyword: SpanId,
        predicate: Spanned<RefinementPredicate>,
        refinements: Vec<Spanned<Refinement>>,
    },
}
