use crate::{
    Annotation, Contract, Expression, Import, Name, ParameterConvention, SpanId,
    Spanned,
    TypeDomain,
};

/// One top-level source occurrence. No item is merged or indexed by name here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Import(Box<Import>),
    Binding(Box<Binding>),
    Type(Box<TypeDeclaration>),
    Expression(Expression),
    /// The written section marker, not a derived visibility flag on each item.
    Private,
    /// The written `#entry` line. It marks the binding that follows; that pairing is
    /// checked later, so the marker carries no target of its own.
    Entry,
    /// The written `#wrap` line. It marks the named integer domain declaration that
    /// follows: arithmetic whose destination is that domain is reduced into it. Like
    /// `#entry`, the pairing is checked later, so the marker carries no target.
    Wrap,
    /// The written `#unrestricted` line. It marks the nominal type declaration that
    /// follows as freely copyable and discardable. Like `#wrap`, the pairing and whether
    /// the type may be unrestricted are checked later, so the marker carries no target.
    Unrestricted,
    /// The written `#instruction('bl')` line. It marks the body-less function that follows
    /// as that machine instruction. `text` is the span of the string literal.
    Instruction { text: SpanId },
    /// The written `#load` line. It marks the body-less function that follows as a typed read
    /// of memory: `load(element Type, pointer @element, index N) element`.
    Load,
    /// The written `#store` line, marking `store(element Type, pointer @element, index N,
    /// value element)` as a typed write of memory.
    Store,
    /// Rejected or unsupported source retained by its span, with parser diagnostics.
    Error,
}

/// The common source shape of a binding, function, or simple `has` member.
///
/// `None` parameters means no written parameter list; an empty list means `()`.
/// `annotation` is the written value/return annotation, never an inferred type.
/// `None` value means a declaration without an initializer or body.
///
/// This initial slice does not model receiver-qualified names, general attributes,
/// effective ownership semantics, or alternative results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// Ordered written field attributes, not effective visibility.
    pub attributes: Vec<Spanned<FieldAttribute>>,
    pub name: Name,
    pub parameters: Option<Spanned<Vec<Spanned<Parameter>>>>,
    /// Written header convention/restriction, not an inferred function classification.
    /// `None` means unwritten; simple `has` fields keep `own` in their annotation.
    pub convention: Option<Spanned<BindingConvention>>,
    /// One written result name, not a body-local binding or resolved symbol.
    /// `None` leaves the result anonymous; visibility is not checked here.
    pub result_binder: Option<Name>,
    pub annotation: Option<Spanned<Annotation>>,
    /// The written `where` clause: claims about the result and transitions of the places
    /// lent `mut`.
    pub contract: Option<Spanned<Contract>>,
    /// The other variants of a result that can be one of several, each after the previous
    /// variant's `or`: `Reserved where (..) or AllocError where (..)`. The first variant is
    /// `annotation` and `contract`.
    pub alternatives: Vec<Spanned<ResultAlternative>>,
    /// The further named results of `f(..) first T, second U`, after the first result: the
    /// function returns a record with those fields. The first is `result_binder` and `annotation`.
    pub extra_results: Vec<Spanned<ExtraResult>>,
    pub value: Option<BindingValue>,
}

/// One further named result of a function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraResult {
    pub comma: SpanId,
    pub name: Name,
    pub annotation: Spanned<Annotation>,
}

/// One further variant of a result: its type and the claims it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultAlternative {
    pub or_keyword: SpanId,
    pub annotation: Spanned<Annotation>,
    pub contract: Option<Spanned<Contract>>,
}

/// A written field attribute, without visibility or applicability semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldAttribute {
    Private {
        hash: SpanId,
        private_keyword: SpanId,
    },
    /// `#infix('+')` on a method: the written operator string and its punctuation. Which
    /// operators exist, and whether the method fits, is checked later.
    Infix {
        hash: SpanId,
        keyword: SpanId,
        open: SpanId,
        operator: SpanId,
        close: SpanId,
    },
    /// `#inline` on a method: a hint that its body may be placed at its calls. The compiler
    /// is free to ignore it; it never changes what a program means.
    Inline { hash: SpanId, keyword: SpanId },
    /// `#register('x0')` on a field: the machine register this field occupies at an
    /// instruction boundary. Which names exist is the target's, not the compiler's.
    Register {
        hash: SpanId,
        keyword: SpanId,
        open: SpanId,
        name: SpanId,
        close: SpanId,
    },
}

/// A written header convention/restriction, not semantic permission or proof.
///
/// Syntax alone neither initializes nor checks ownership and supplies no default
/// terminal or ownership mode. Later legacy `own` keywords remain storage type prefixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingConvention {
    /// Legacy experimental syntax, retained without assigning new semantics.
    Own,
    /// Non-escape/scope-ending restriction on an owner, not a transfer marker.
    End,
}

/// Parameters remain a list: duplicate names survive for later diagnostics.
///
/// A bare `self` has neither a synthesized annotation nor a convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    pub name: Name,
    pub convention: Option<Spanned<ParameterConvention>>,
    pub annotation: Option<Spanned<Annotation>>,
    pub default: Option<Expression>,
}

/// Written parameter keywords, without effective passing/storage semantics or defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Convention {
    Ref,
    Mut,
    /// Legacy ownership-transfer convention on an existing value.
    /// Permits forwarding ownership and has distinct checks from the default terminal convention.
    /// Distinct from storage [`crate::TypePrefixKind::Own`]; it adds no storage layer.
    /// Retaining this source form does not assign it the revised strict End semantics.
    Own,
    /// Non-escape/scope-ending restriction on an owner, not a transfer marker.
    End,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingValue {
    pub operator: Spanned<BindingOperator>,
    pub expression: Expression,
}

/// Source punctuation, without assigning analysis-time mutability or staging.
///
/// Reassignment (`::`) and compound assignment are not declarations and will
/// need their own statement forms; they are intentionally absent from this slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingOperator {
    /// `:`
    Colon,
    /// `:=`
    ColonEquals,
}

/// One written type declaration, not an assembled type definition.
///
/// A later `Thing has ...` remains another item rather than being attached to
/// an earlier `Thing is ...`. No methods or result unions are synthesized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDeclaration {
    pub name: Name,
    pub parameters: Option<Spanned<Vec<Spanned<Parameter>>>>,
    /// The span includes `is` or `has` and the complete following body.
    pub body: Spanned<TypeBody>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeBody {
    /// A source expression, including written `or`, not evaluated variants.
    Is(Expression),
    /// Written membership, range bounds, and ordered refinement clauses.
    Domain(Spanned<TypeDomain>),
    /// Simple members only; qualified members and trait provisions come later.
    Has(Vec<Spanned<TypeMember>>),
}

/// A written member or rejected source inside a `has` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeMember {
    Binding(Box<Binding>),
    /// A trait the type provides, written `Type has Trait and Other` on the header.
    /// Whether the name is a trait, and what it adds, is decided later.
    Provision(Name),
    /// A provided trait that takes arguments, `Add(IntegerResult)`, as the call it is written as.
    ProvisionApplied(crate::Expression),
    /// Retains a rejected member's source range without inventing a declaration.
    Error,
}
