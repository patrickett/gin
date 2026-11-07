use crate::{Convention, Expression, Name, SpanId, Spanned};

/// A written type-prefix keyword or punctuation, without effective ownership facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypePrefixKind {
    Ref,
    Mut,
    /// Storage ownership, not the existing-value parameter convention [`Convention::Own`].
    ///
    /// TODO(storage-own-replacement): retain the narrow stored-field capability
    /// while investigating a general replacement. It must account for empty and
    /// partially initialized storage, preserve field obligations through moves,
    /// extraction/replacement and cleanup, and retain construction/access controls.
    /// Check real library bodies and separate clients, including failure paths;
    /// a raw pointer or a renamed builtin is not sufficient evidence.
    /// Strict `end` is non-transferable and is not a mechanical replacement.
    /// This lossless AST variant does not itself restrict source roles or implement ownership.
    Own,
    /// The experimental raw pointer type spelling `@T`, not a value operation.
    Pointer,
}

/// A written type prefix, without inferred groups, authority, or initialization facts.
///
/// Recovery may retain a group even on `own` or `@`; diagnostics belong to the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixedType {
    pub keyword: Spanned<TypePrefixKind>,
    pub group: Option<Spanned<GroupPath>>,
    pub target: Box<Expression>,
}

/// A written group path. Its enclosing span retains braces, dots, and spacing.
///
/// Recovery may retain no components or omit the closing brace. Names are never
/// fabricated or resolved into semantic group identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupPath {
    pub open: SpanId,
    pub components: Vec<Name>,
    pub close: Option<SpanId>,
}

/// A parameter keyword and its attached written group, if any.
///
/// The enclosing span covers both; the keyword span covers only `ref`, `mut`,
/// `own`, or `end`. This does not synthesize a borrowed type or an implicit group.
/// The parser permits groups on `ref`/`mut`, not on `own`/`end`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterConvention {
    pub keyword: Spanned<Convention>,
    pub group: Option<Spanned<GroupPath>>,
}
