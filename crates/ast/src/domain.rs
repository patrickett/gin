use crate::{BinaryOperator, Expression, SpanId, Spanned};

/// A written domain, without evaluating bounds or synthesizing predicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDomain {
    /// Producers may omit `in` only for a range base.
    pub in_keyword: Option<SpanId>,
    pub base: Spanned<DomainBase>,
    pub refinements: Vec<Spanned<Refinement>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainBase {
    Expression(Expression),
    Range {
        lower: Expression,
        to_keyword: SpanId,
        upper: Expression,
    },
}

/// One source-ordered `and` clause, retaining its written subject form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refinement {
    pub and_keyword: SpanId,
    pub predicate: Spanned<RefinementPredicate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefinementPredicate {
    Expression(Expression),
    /// Producers use only `=`, `<`, `<=`, `>`, or `>=`; no subject is invented.
    /// Ungrouped right-hand operators bind more tightly than the comparison.
    ImplicitComparison {
        operator: Spanned<BinaryOperator>,
        right: Expression,
    },
    /// `and in 1 to 10`: an implicit membership bound on the annotated value,
    /// written as a domain rather than a comparison against each edge.
    ImplicitDomain(Spanned<TypeDomain>),
}
