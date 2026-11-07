//! Borrowed, read-only views of source syntax. No symbol resolution or stable IDs.

mod lookup;
mod outline;

use std::fmt;

use crate::{Annotation, DomainBase, Refinement, RefinementPredicate, TypeDomain, UnaryOperator};
use crate::{BindingConvention, GroupPath, ParameterConvention, TypePrefixKind};

use crate::{
    Argument, BinaryOperator, Binding, BindingOperator, Contract, Convention, Expression, Import,
    ImportEntry, ImportMember, ImportSource, Item, Name, Parameter, ResultAlternative, Span, Spanned, ClauseItem,
    Statement, TypeBody, TypeDeclaration, TypeMember,
};

pub use outline::{Declaration, DeclarationKind};

/// Query tags for domain keywords; AST fields retain their own source spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainKeyword {
    In,
    To,
    And,
}

/// The annotation introducer, distinct from domain membership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationKeyword {
    Is,
    Has,
}

/// Written braces of a group path, including an optionally absent close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupDelimiter {
    Open,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionPunctuation {
    Open,
    Close,
    Comma,
    Semicolon,
    Dot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopKeyword {
    For,
    In,
    While,
    Where,
    Do,
}

/// A borrowed syntax view and its resolved range in the queried source snapshot.
#[derive(Debug, Clone, Copy)]
pub struct SyntaxNode<'syntax> {
    pub kind: SyntaxNodeKind<'syntax>,
    pub span: Span,
}

/// Concrete syntax structures available without inferring types or resolving names.
///
/// Equal ranges need not identify equal nodes: expression wrappers and their name
/// leaves can cover the same bytes. These views are not identities across edits.
#[derive(Clone, Copy)]
pub enum SyntaxNodeKind<'syntax> {
    File,
    Item(&'syntax Spanned<Item>),
    Import(&'syntax Import),
    ImportEntry(&'syntax Spanned<ImportEntry>),
    ImportSource(&'syntax Spanned<ImportSource>),
    ImportMember(&'syntax Spanned<ImportMember>),
    Binding(&'syntax Binding),
    FieldAttribute(&'syntax Spanned<crate::FieldAttribute>),
    AttributeHash,
    PrivateKeyword,
    InfixKeyword,
    InlineKeyword,
    InfixOperator,
    Destructure(&'syntax crate::DestructureStatement),
    MatchPattern(&'syntax Spanned<crate::MatchPattern>),
    /// The containing `SyntaxNode` supplies the span for this borrowed source variant.
    VariantPattern(&'syntax crate::VariantPattern),
    PatternBinderList(&'syntax crate::PatternFields),
    PatternBinder(&'syntax Name),
    /// The field a pattern binder names: the name after the colon of `binder: field`.
    PatternSelector(&'syntax Name),
    PatternDelimiter(GroupDelimiter),
    Match(&'syntax crate::MatchExpression),
    Assign(&'syntax crate::AssignStatement),
    ReassignOperator,
    Conditional(&'syntax crate::ConditionalExpression),
    Chain(&'syntax crate::ChainExpression),
    ElseKeyword,
    MatchArmList(&'syntax Spanned<Vec<Spanned<crate::MatchArm>>>),
    MatchArm(&'syntax Spanned<crate::MatchArm>),
    WhenKeyword,
    IsKeyword,
    ThenKeyword,
    AndKeyword,
    BindingConvention(&'syntax Spanned<BindingConvention>),
    ResultBinder(&'syntax Name),
    TypeDeclaration(&'syntax TypeDeclaration),
    ParameterList(&'syntax Spanned<Vec<Spanned<Parameter>>>),
    Parameter(&'syntax Spanned<Parameter>),
    ParameterConvention(&'syntax Spanned<ParameterConvention>),
    Convention(&'syntax Spanned<Convention>),
    TypePrefixKeyword(&'syntax Spanned<TypePrefixKind>),
    GroupPath(&'syntax Spanned<GroupPath>),
    GroupDelimiter(GroupDelimiter),
    Annotation(&'syntax Spanned<Annotation>),
    AnnotationKeyword(AnnotationKeyword),
    Expression(&'syntax Expression),
    Array(&'syntax crate::ArrayExpression),
    Sequence(&'syntax crate::SequenceExpression),
    Repeat(&'syntax crate::RepeatExpression),
    Select(&'syntax crate::SelectExpression),
    NumericSelect(&'syntax crate::NumericSelectExpression),
    CollectionPunctuation(CollectionPunctuation),
    RangeKeyword,
    AsKeyword,
    Loop(&'syntax crate::LoopExpression),
    LoopBinder(&'syntax Name),
    LoopKeyword(LoopKeyword),
    ArgumentList(&'syntax Spanned<Vec<Spanned<Argument>>>),
    Argument(&'syntax Spanned<Argument>),
    Statement(&'syntax Spanned<Statement>),
    Contract(&'syntax Spanned<Contract>),
    ResultAlternative(&'syntax Spanned<ResultAlternative>),
    ExtraResult(&'syntax Spanned<crate::ExtraResult>),
    OrKeyword,
    Transition(&'syntax Spanned<ClauseItem>),
    WhereKeyword,
    TypeBody(&'syntax Spanned<TypeBody>),
    TypeDomain(&'syntax Spanned<TypeDomain>),
    DomainBase(&'syntax Spanned<DomainBase>),
    Refinement(&'syntax Spanned<Refinement>),
    RefinementPredicate(&'syntax Spanned<RefinementPredicate>),
    UnaryOperator(&'syntax Spanned<UnaryOperator>),
    DomainKeyword(DomainKeyword),
    TypeMember(&'syntax Spanned<TypeMember>),
    Name(&'syntax str),
    BinaryOperator(&'syntax Spanned<BinaryOperator>),
    BindingOperator(&'syntax Spanned<BindingOperator>),
}

// Logging a hit must not recursively dump every declaration body in its context.
impl fmt::Debug for SyntaxNodeKind<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::File => "File",
            Self::Item(_) => "Item",
            Self::Import(_) => "Import",
            Self::ImportEntry(_) => "ImportEntry",
            Self::ImportSource(_) => "ImportSource",
            Self::ImportMember(_) => "ImportMember",
            Self::Binding(_) => "Binding",
            Self::FieldAttribute(_) => "FieldAttribute",
            Self::AttributeHash => "AttributeHash",
            Self::PrivateKeyword => "PrivateKeyword",
            Self::InfixKeyword => "InfixKeyword",
            Self::InlineKeyword => "InlineKeyword",
            Self::InfixOperator => "InfixOperator",
            Self::Destructure(_) => "Destructure",
            Self::MatchPattern(_) => "MatchPattern",
            Self::VariantPattern(_) => "VariantPattern",
            Self::PatternBinderList(_) => "PatternBinderList",
            Self::PatternBinder(_) => "PatternBinder",
            Self::PatternSelector(_) => "PatternSelector",
            Self::PatternDelimiter(_) => "PatternDelimiter",
            Self::Match(_) => "Match",
            Self::Assign(_) => "Assign",
            Self::ReassignOperator => "ReassignOperator",
            Self::Conditional(_) => "Conditional",
            Self::Chain(_) => "Chain",
            Self::ElseKeyword => "ElseKeyword",
            Self::MatchArmList(_) => "MatchArmList",
            Self::MatchArm(_) => "MatchArm",
            Self::WhenKeyword => "WhenKeyword",
            Self::IsKeyword => "IsKeyword",
            Self::ThenKeyword => "ThenKeyword",
            Self::AndKeyword => "AndKeyword",
            Self::BindingConvention(_) => "BindingConvention",
            Self::ResultBinder(_) => "ResultBinder",
            Self::TypeDeclaration(_) => "TypeDeclaration",
            Self::ParameterList(_) => "ParameterList",
            Self::Parameter(_) => "Parameter",
            Self::ParameterConvention(_) => "ParameterConvention",
            Self::Convention(_) => "Convention",
            Self::TypePrefixKeyword(_) => "TypePrefixKeyword",
            Self::GroupPath(_) => "GroupPath",
            Self::GroupDelimiter(_) => "GroupDelimiter",
            Self::Annotation(_) => "Annotation",
            Self::AnnotationKeyword(_) => "AnnotationKeyword",
            Self::Expression(_) => "Expression",
            Self::Array(_) => "Array",
            Self::Sequence(_) => "Sequence",
            Self::Repeat(_) => "Repeat",
            Self::Select(_) => "Select",
            Self::NumericSelect(_) => "NumericSelect",
            Self::CollectionPunctuation(_) => "CollectionPunctuation",
            Self::RangeKeyword => "RangeKeyword",
            Self::AsKeyword => "AsKeyword",
            Self::Loop(_) => "Loop",
            Self::LoopBinder(_) => "LoopBinder",
            Self::LoopKeyword(_) => "LoopKeyword",
            Self::ArgumentList(_) => "ArgumentList",
            Self::Argument(_) => "Argument",
            Self::Statement(_) => "Statement",
            Self::Contract(_) => "Contract",
            Self::ResultAlternative(_) => "ResultAlternative",
            Self::ExtraResult(_) => "ExtraResult",
            Self::OrKeyword => "OrKeyword",
            Self::Transition(_) => "Transition",
            Self::WhereKeyword => "WhereKeyword",
            Self::TypeBody(_) => "TypeBody",
            Self::TypeDomain(_) => "TypeDomain",
            Self::DomainBase(_) => "DomainBase",
            Self::Refinement(_) => "Refinement",
            Self::RefinementPredicate(_) => "RefinementPredicate",
            Self::UnaryOperator(_) => "UnaryOperator",
            Self::DomainKeyword(_) => "DomainKeyword",
            Self::TypeMember(_) => "TypeMember",
            Self::Name(name) => return formatter.debug_tuple("Name").field(name).finish(),
            Self::BinaryOperator(_) => "BinaryOperator",
            Self::BindingOperator(_) => "BindingOperator",
        };
        formatter.write_str(label)
    }
}

/// The selected node plus its source context, all borrowed from one snapshot.
#[derive(Debug)]
pub struct NodeAtOffset<'syntax> {
    pub node: SyntaxNode<'syntax>,
    /// Structural ancestors, from the file root to the immediate parent.
    pub ancestors: Vec<SyntaxNode<'syntax>>,
    /// Binding/type owners and selected pattern binders, outermost first.
    ///
    /// Local bindings are included. Parameters are syntax nodes, not entries in
    /// this list. The first declaration therefore identifies the top-level owner;
    /// the last identifies the nearest binding/type or selected positional binder,
    /// not a resolved symbol. Pattern binders never enter the top-level outline.
    pub declarations: Vec<Declaration<'syntax>>,
}
