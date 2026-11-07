//! Source syntax for Gin, without resolution, evaluation, or staging state.
//!
//! Its compiler dependency is `span`; the parser constructs these source occurrences
//! and semantic consumers interpret them without adding inferred facts to the tree.
//!
//! # Representation rules
//!
//! - Lists retain source order and duplicate occurrences.
//! - [`Spanned<T>`] locates syntax in this file's [`SpanTable`]. A [`SpanId`] is
//!   source provenance, not node identity or a key for semantic facts.
//! - Names, explicit calls, and named arguments have different representations.
//! - Annotations retain written header roles, never resolved types.
//! - Literal values are not decoded or evaluated; their spelling is in the source.
//! - Grouping and explicit returns survive; there is no desugaring here.
//! - Recovery is explicit: missing expressions have zero-width spans; rejected
//!   source uses error items/statements, never a semantic placeholder.
//!
//! The structs are deliberately ordinary owned Rust data. Producers (including
//! the `parser` crate) are responsible for valid, UTF-8-aligned
//! spans, enclosing child spans, and source order. Construction does not
//! currently validate those invariants.

mod annotation;
mod collection;
mod domain;
mod expr;
mod import;
mod item;
mod query;
mod type_prefix;

use std::sync::Arc;

pub use annotation::Annotation;
pub use collection::{
    ArrayExpression, ArrayLength, LoopExpression, LoopInvariant, NumericSelectExpression, RepeatExpression,
    SelectExpression, SequenceDelimiters, SequenceExpression,
};
pub use domain::{DomainBase, Refinement, RefinementPredicate, TypeDomain};
pub use expr::{
    Argument, AssignStatement, BinaryOperator, Block, ChainArm, ChainExpression, ClauseItem, ConditionalExpression, Contract,
    DestructureStatement,
    ElseBranch, Expr, Expression, Guard, IsTest, LiteralKind, MatchArm, MatchExpression, MatchPattern, PatternFields,
    ReturnStatement, Statement, Transition, UnaryOperator, VariantPattern,
    WhenIsStatement, WhenStatement,
};
pub use import::{Import, ImportEntry, ImportMember, ImportSource};
pub use item::{
    Binding, BindingConvention, BindingOperator, BindingValue, Convention, FieldAttribute, Item,
    ExtraResult, Parameter, ResultAlternative, TypeBody, TypeDeclaration, TypeMember,
};
pub use query::{
    AnnotationKeyword, CollectionPunctuation, Declaration, DeclarationKind, DomainKeyword,
    GroupDelimiter, LoopKeyword, NodeAtOffset, SyntaxNode, SyntaxNodeKind,
};
pub use span::{Span, SpanId, SpanTable, Spanned};
pub use type_prefix::{GroupPath, ParameterConvention, PrefixedType, TypePrefixKind};

/// An identifier's spelling and its own source location, without a resolved target.
pub type Name = Spanned<String>;

/// A source snapshot and the syntax occurrences represented in it.
///
/// `items` is not a symbol table. Repeated declarations, overloads, and type
/// extensions remain separate occurrences. `private` remains a source marker.
///
/// The original text retains all whitespace and comments, but this is not a
/// lossless CST: there is no token tree, printer, or edit operation yet. Syntax
/// spans and the comment inventory refer to this exact, unmodified snapshot.
///
/// Cloning shares the text, but clones the tree and span table. There is no
/// copy-on-write tree or stable identity across snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    pub text: Arc<str>,
    pub span_table: SpanTable,
    pub items: Vec<Spanned<Item>>,
    /// All comments, including docs, in source order; no attachment heuristics.
    pub comments: Vec<Spanned<CommentKind>>,
}

/// Comment spelling comes from its span, including the comment introducer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentKind {
    Ordinary,
    /// `---`
    Documentation,
    /// `--|`
    ModuleDocumentation,
}
