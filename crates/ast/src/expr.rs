use crate::{Binding, BindingOperator, Name, PrefixedType, RefinementPredicate, SpanId, Spanned};

/// An expression and the span of the complete source expression.
pub type Expression = Spanned<Expr>;

/// Source expression forms, before name resolution, staging, or desugaring.
///
/// A capitalized name is still a name. Whether a dotted access denotes a
/// module member, record field, or method is determined outside this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Name(String),
    /// Written type-position syntax, not a value borrow or a convention.
    PrefixedType(PrefixedType),
    /// The enclosing expression span retains the original spelling.
    Literal(LiteralKind),
    Member {
        base: Box<Expression>,
        name: Name,
    },
    /// Only explicit application: `f()` is different from the name `f`.
    Call {
        callee: Box<Expression>,
        /// The list span includes existing parentheses; recovery may lack a closing one.
        arguments: Spanned<Vec<Spanned<Argument>>>,
    },
    Unary {
        operator: Spanned<UnaryOperator>,
        operand: Box<Expression>,
    },
    Binary {
        left: Box<Expression>,
        operator: Spanned<BinaryOperator>,
        right: Box<Expression>,
    },
    /// `#name`, a compile-time query: `#size(x)` is a call of it and `#target.triple` a member
    /// of it. Which queries exist is the checker's to say, including `#unchecked(value)`, the
    /// writer's acceptance that the operation `value` performs stops being proved.
    Query {
        hash: SpanId,
        name: Name,
    },
    /// `subject is Pattern` in a condition: whether the subject is that member, or that number.
    /// Binders of the pattern are not in scope outside a one-armed case.
    IsTest(Box<IsTest>),
    /// `name in Domain: value` inside parentheses: the value, checked in the domain. The
    /// parentheses are the enclosing `Group`.
    Bind(Box<Binding>),
    /// An explicit written cast, without conversion validity or construction authority.
    Cast {
        value: Box<Expression>,
        as_keyword: SpanId,
        target: Box<Expression>,
    },
    /// Parentheses remain present rather than being discarded during parsing.
    Group(Box<Expression>),
    Block(Block),
    Array(Box<crate::ArrayExpression>),
    Sequence(Box<crate::SequenceExpression>),
    Repeat(Box<crate::RepeatExpression>),
    Select(Box<crate::SelectExpression>),
    NumericSelect(Box<crate::NumericSelectExpression>),
    /// Inclusive written range, without constant evaluation.
    Range {
        start: Box<Expression>,
        to_keyword: SpanId,
        end: Box<Expression>,
    },
    Loop(Box<crate::LoopExpression>),
    /// Ordinary consuming case syntax; resolution and exhaustiveness belong later.
    Match(Box<MatchExpression>),
    /// `when condition then value else other`: a branch chosen by a condition, with no
    /// patterns. What the condition may be is decided later.
    Conditional(Box<ConditionalExpression>),
    /// `when` alone on its line, then indented `condition then value` arms and a final
    /// `else value`: guarded arms tried in order.
    Chain(Box<ChainExpression>),
    /// The explicit source hole `#todo`, not a parser-recovery placeholder.
    Hole,
    /// A required expression absent from incomplete source; its span is zero-width.
    Missing,
    /// A written `base.` whose member name has not been supplied yet.
    IncompleteMember {
        base: Box<Expression>,
    },
}

/// The test form of `is`: the pattern is a variant (with binders) or an integer literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsTest {
    pub subject: Expression,
    pub is_keyword: SpanId,
    pub pattern: Spanned<MatchPattern>,
}

/// A call argument is not a declaration or a local binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Argument {
    Positional(Expression),
    Named { name: Name, value: Expression },
    /// `name Type`, a payload field declared in place: `Reserved(storage Vacant(x))` in an `or`
    /// list or an inline result header. Not a value; the checker reads it in type position only.
    Typed { name: Name, annotation: Expression },
}

/// Literal token categories, not runtime types or decoded constant values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiteralKind {
    Integer,
    Float,
    String,
}

/// Written prefix signs, without constant folding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOperator {
    Negate,
    /// `not condition`: it covers the comparison or test that follows it, up to `and`/`or`.
    Not,
    /// `deref value`: the value a `ref` or `mut` borrow refers to, copied out.
    Deref,
    /// `domain of T`: the declared range of a type. Its span covers both words.
    DomainOf,
}

/// Written binary operators. Dispatch and type-dependent meaning belong later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Equal,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
    And,
    Or,
}

/// A source-ordered body, without a synthesized result or separate return slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub statements: Vec<Spanned<Statement>>,
}

/// `where item, item ...` after a result: what a call guarantees. A claim relates values in
/// the state the call returns in; a transition says what a place lent `mut` holds then, in
/// terms of entry. Each transition `value` reads the state at entry, so those are
/// simultaneous. This retains source syntax only; which claims and places are allowed is
/// decided later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contract {
    pub where_keyword: SpanId,
    /// The `(` and `)` around the items, as a result variant's claims are written.
    pub open: Option<SpanId>,
    pub items: Vec<Spanned<ClauseItem>>,
    /// The `and` or `,` written between items: one fewer than the items.
    pub joiners: Vec<SpanId>,
    pub close: Option<SpanId>,
}

/// One item of a [`Contract`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClauseItem {
    /// `place :: value`
    Transition(Transition),
    /// A proposition over the result, the parameters and their entry values, without Boolean
    /// or proof requirements here. An unnamed result is compared with the leading-operator
    /// form (`>= 10`), as in a refinement.
    Claim(RefinementPredicate),
}

/// One `place :: value` clause of a [`Contract`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    /// A parameter or a field path of one, as in an assignment.
    pub place: Expression,
    /// The `::` token.
    pub operator: SpanId,
    pub value: Expression,
}

/// A written return, retaining an absent clause versus explicit recovery syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnStatement {
    /// One written value; comma-separated returns use the ordinary sequence expression.
    pub value: Option<Expression>,
}

/// The inline conditional form of `when`, written on one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionalExpression {
    pub when_keyword: SpanId,
    pub condition: Expression,
    pub then_keyword: SpanId,
    pub then_branch: Expression,
    pub else_keyword: SpanId,
    pub else_branch: Expression,
}

/// A subjectless `when` with indented guarded arms, ending in a required `else`.
///
/// A condition is any condition the inline conditional accepts, including an `x is Pattern`
/// test (an [`Expr::IsTest`]). Its `then` may follow on the next line. When `else` is absent
/// from the source, a diagnostic is reported, `else_keyword` is zero-width and `otherwise` is
/// [`Expr::Missing`]. Arms are tried in order; which condition holds is decided later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainExpression {
    pub when_keyword: SpanId,
    pub arms: Vec<Spanned<ChainArm>>,
    pub else_keyword: SpanId,
    pub otherwise: Expression,
}

/// `condition then value`: one guarded arm of a [`ChainExpression`]. A missing `then` is
/// reported, with a zero-width keyword and an [`Expr::Missing`] value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainArm {
    pub condition: Expression,
    pub then_keyword: SpanId,
    pub value: Expression,
}

/// `when condition then <block> [else <block>]` written as a statement.
///
/// The statement form differs from [`ConditionalExpression`]: its branches are blocks, `else`
/// is optional, and a branch may `return`. Whether the statement is the body's value is decided
/// later, by its position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhenStatement {
    pub when_keyword: SpanId,
    pub condition: Expression,
    pub then_keyword: SpanId,
    pub then_block: Spanned<Block>,
    pub else_branch: Option<ElseBranch>,
}

/// `when subject is Pattern then <block> [else <block>]` written as a statement: one pattern,
/// where a [`MatchExpression`] has an arm for every member. The block runs when the subject is
/// that member; otherwise the statement is passed over. A `_` binder in the pattern discards
/// its field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhenIsStatement {
    pub when_keyword: SpanId,
    pub subject: Expression,
    pub is_keyword: SpanId,
    pub pattern: Spanned<MatchPattern>,
    pub then_keyword: SpanId,
    pub then_block: Spanned<Block>,
    pub else_branch: Option<ElseBranch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElseBranch {
    pub else_keyword: SpanId,
    pub block: Spanned<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchExpression {
    pub when_keyword: SpanId,
    pub subject: Expression,
    pub is_keyword: SpanId,
    pub arms: Spanned<Vec<Spanned<MatchArm>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchArm {
    pub pattern: Spanned<MatchPattern>,
    /// `and condition`, evaluated with the pattern's binders in scope.
    pub guard: Option<Guard>,
    pub then_keyword: SpanId,
    pub body: Expression,
}

/// The guard of a case arm: `pattern and condition then value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guard {
    pub and_keyword: SpanId,
    pub condition: Expression,
}

/// Written case patterns, without resolution, constant decoding, or coverage facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchPattern {
    Variant(VariantPattern),
    /// An integer literal or its written unary negation, retaining exact source spans.
    Integer(Expression),
    /// A text literal: an arm of a case over compile-time text.
    Text(Expression),
    /// `else value`: the arm taken when no other is. The only arm with no `then`.
    Else {
        else_keyword: SpanId,
    },
    /// `in Domain`: whether the subject lies in the domain's declared range. Only a condition
    /// tests it; a case has no arm for it.
    Domain {
        in_keyword: SpanId,
        domain: Expression,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantPattern {
    /// Unresolved constructor spelling, not a call or a resolved type.
    pub constructor: Name,
    /// `None` retains bare nullary syntax, distinct from explicit `Empty()`.
    pub fields: Option<PatternFields>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternFields {
    pub open: SpanId,
    /// Includes parentheses; empty and duplicate binders remain written syntax.
    pub binders: Spanned<Vec<Name>>,
    /// Beside each binder, the field it names when written `binder: field`; `None` for a
    /// bare binder. Same length as `binders`.
    pub selectors: Vec<Option<Name>>,
    /// Beside each binder, the pattern written in its place (`Flaw(OutOfMemory)`: the binder is
    /// the inner constructor's name, kept so spans and text agree). Only a case arm's pattern
    /// has any; same length as `binders`.
    pub nested: Vec<Option<Box<Spanned<VariantPattern>>>>,
    pub close: SpanId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestructureStatement {
    pub pattern: Spanned<VariantPattern>,
    /// Only `:=` is accepted by this source slice.
    pub operator: Spanned<BindingOperator>,
    pub value: Expression,
}

/// `place :: value`: the place holds the value afterwards. The place is a local or a field
/// path `a.b.c`; whether it may be reassigned is decided later.
///
/// `place +: value` is the compound form: the place holds `place + value` afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignStatement {
    pub place: Expression,
    /// The operator before the colon of a compound assignment.
    pub compound: Option<Spanned<BinaryOperator>>,
    /// The `::` token, or the `:` token of a compound assignment.
    pub operator: SpanId,
    pub value: Expression,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    Binding(Box<Binding>),
    /// Reassignment of a place; not a declaration.
    Assign(AssignStatement),
    /// Flat consuming-pattern syntax, without inferred ownership transfers.
    Destructure(DestructureStatement),
    Expression(Expression),
    /// The statement span includes `return`; no value is a bare return.
    Return(ReturnStatement),
    /// A conditional whose branches are statement blocks; `else` is optional.
    When(Box<WhenStatement>),
    /// A one-armed case: `when subject is Pattern then ...`.
    WhenIs(Box<WhenIsStatement>),
    /// Rejected or unsupported statement retained by its span, with diagnostics.
    Error,
}
