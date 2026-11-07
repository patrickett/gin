//! Written collection syntax, without element types, bounds checks, or storage semantics.

use crate::{Expression, Name, SpanId};

/// Sequence syntax, distinct from a group, call, repeat, or array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceExpression {
    pub delimiters: SequenceDelimiters,
    pub elements: Vec<Expression>,
    /// Written commas, including the comma required by a singleton sequence.
    pub separators: Vec<SpanId>,
}

/// Only return lists may omit parentheses. Neither form supplies semantic privilege.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceDelimiters {
    Parenthesized {
        open: SpanId,
        /// A zero-width span denotes a missing required closing delimiter.
        close: SpanId,
    },
    Bare,
}

/// `base.0`, retaining the written dot and integer spelling without brackets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumericSelectExpression {
    pub base: Expression,
    pub dot: SpanId,
    pub index: Expression,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrayExpression {
    pub open: SpanId,
    /// `[T]` and `[value]` remain contextual syntax, not resolved type/value categories.
    pub elements: Vec<Expression>,
    /// Beside each element, the label written before its colon (`[lhs: rax]`); same length
    /// as `elements`.
    pub labels: Vec<Option<Name>>,
    /// Written commas, including a trailing comma; newlines remain in the source snapshot.
    pub separators: Vec<SpanId>,
    pub length: Option<ArrayLength>,
    /// A zero-width span denotes a missing required closing delimiter.
    pub close: SpanId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrayLength {
    pub separator: SpanId,
    pub value: Expression,
}

/// `(value; length)`, distinct from grouping and bracket syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepeatExpression {
    pub open: SpanId,
    pub value: Expression,
    pub separator: SpanId,
    pub length: Expression,
    pub close: SpanId,
}

/// `base.[index]` or `base.[start; length]`. Slice lengths are not end bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectExpression {
    pub base: Expression,
    pub dot: SpanId,
    pub open: SpanId,
    pub index: Expression,
    /// Absent for indexing; present even when the written `;` lacks a value.
    pub length: Option<ArrayLength>,
    pub close: SpanId,
}

/// Single-binder for loops and expression while conditions, both requiring `do`.
/// `where P` on a loop head: the loop's invariant, written, never inferred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopInvariant {
    pub where_keyword: SpanId,
    pub condition: Expression,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopExpression {
    For {
        for_keyword: SpanId,
        binder: Name,
        in_keyword: SpanId,
        iterable: Expression,
        /// `where P` after the range: what holds at the head of every pass. None inferred.
        invariant: Option<LoopInvariant>,
        /// Required `do`; a diagnosed zero-width span retains a missing keyword.
        do_keyword: SpanId,
        body: Expression,
    },
    While {
        while_keyword: SpanId,
        condition: Expression,
        /// `where P` after the condition: what holds at the head of every pass.
        invariant: Option<LoopInvariant>,
        do_keyword: SpanId,
        body: Expression,
    },
}
