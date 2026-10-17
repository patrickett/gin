use super::ParseCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum SyntaxCode {
    AttributeLine,
    BindingNameNotSnakeCase,
    CallArgumentConvention,
    DeferredAutoDestroy,
    DetachedAttribute,
    DomainExtraRefinement,
    DuplicateParameter,
    DuplicateResultName,
    DuplicateSequenceName,
    EmptyParens,
    EmptyWhen,
    Hint,
    IndexRequiresDot,
    InvalidImportTarget,
    InvalidPrivateBlock,
    LegacyImportSelection,
    MisplacedElse,
    MissingImportAlias,
    ModuleDocumentationPosition,
    NestedIsDisjunction,
    NestedPrivateBlock,
    ParameterAfterDefault,
    ParenthesizedSequence,
    ProgressStalled,
    PatternTooLarge,
    PrivateBlockOverride,
    RangeParameter,
    ReservedName,
    ResultProofRequiresName,
    ReturnOutsideFunction,
    SequenceRepeatElements,
    TemplateStringsNotSupported,
    TrailingParameterConvention,
    UnclosedWhen,
    UnexpectedToken,
    UnexpectedTokenInFormatString,
    UnknownCompilerQuery,
    UnnecessaryComma,
    UnsupportedImportAlias,
    UnterminatedFormatString,
    UnusedValue,
}

impl Prefix for SyntaxCode {
    fn prefix(&self) -> &'static str {
        "parse"
    }
}

impl Code for SyntaxCode {
    fn name(&self) -> &'static str {
        match self {
            Self::AttributeLine => "attribute-line",
            Self::BindingNameNotSnakeCase => "bind-name-not-snake-case",
            Self::CallArgumentConvention => "call-argument-convention",
            Self::DeferredAutoDestroy => "deferred-auto-destroy",
            Self::DetachedAttribute => "detached-attribute",
            Self::DomainExtraRefinement => "domain-extra-refinement",
            Self::DuplicateParameter => "duplicate-parameter",
            Self::DuplicateResultName => "duplicate-result-name",
            Self::DuplicateSequenceName => "duplicate-sequence-name",
            Self::EmptyParens => "empty-parens",
            Self::EmptyWhen => "empty-when",
            Self::Hint => "hint",
            Self::IndexRequiresDot => "index-requires-dot",
            Self::InvalidImportTarget => "invalid-import-target",
            Self::InvalidPrivateBlock => "invalid-private-block",
            Self::LegacyImportSelection => "legacy-import-selection",
            Self::MisplacedElse => "misplaced-else",
            Self::MissingImportAlias => "missing-import-alias",
            Self::ModuleDocumentationPosition => "module-doc-comment-position",
            Self::NestedIsDisjunction => "nested-is-disjunction",
            Self::NestedPrivateBlock => "nested-private-block",
            Self::ParameterAfterDefault => "parameter-after-default",
            Self::ParenthesizedSequence => "parenthesized-sequence",
            Self::ProgressStalled => "parser-progress",
            Self::PatternTooLarge => "pattern-too-large",
            Self::PrivateBlockOverride => "private-block-override",
            Self::RangeParameter => "range-parameter",
            Self::ReservedName => "reserved-name",
            Self::ResultProofRequiresName => "result-proof-requires-name",
            Self::ReturnOutsideFunction => "return-outside-function",
            Self::SequenceRepeatElements => "sequence-repeat-elements",
            Self::TemplateStringsNotSupported => "template-strings-not-supported",
            Self::TrailingParameterConvention => "trailing-parameter-convention",
            Self::UnclosedWhen => "unclosed-when",
            Self::UnexpectedToken => "unexpected-token",
            Self::UnexpectedTokenInFormatString => "unexpected-token-in-format-string",
            Self::UnknownCompilerQuery => "unknown-compiler-query",
            Self::UnnecessaryComma => "unnecessary-comma",
            Self::UnsupportedImportAlias => "unsupported-import-alias",
            Self::UnterminatedFormatString => "unterminated-format-string",
            Self::UnusedValue => "unused-value",
        }
    }
}

impl SyntaxCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::AttributeLine,
            Self::BindingNameNotSnakeCase,
            Self::CallArgumentConvention,
            Self::DeferredAutoDestroy,
            Self::DetachedAttribute,
            Self::DomainExtraRefinement,
            Self::DuplicateParameter,
            Self::DuplicateResultName,
            Self::DuplicateSequenceName,
            Self::EmptyParens,
            Self::EmptyWhen,
            Self::Hint,
            Self::IndexRequiresDot,
            Self::InvalidImportTarget,
            Self::InvalidPrivateBlock,
            Self::LegacyImportSelection,
            Self::MisplacedElse,
            Self::MissingImportAlias,
            Self::ModuleDocumentationPosition,
            Self::NestedIsDisjunction,
            Self::NestedPrivateBlock,
            Self::ParameterAfterDefault,
            Self::ParenthesizedSequence,
            Self::ProgressStalled,
            Self::PatternTooLarge,
            Self::PrivateBlockOverride,
            Self::RangeParameter,
            Self::ReservedName,
            Self::ResultProofRequiresName,
            Self::ReturnOutsideFunction,
            Self::SequenceRepeatElements,
            Self::TemplateStringsNotSupported,
            Self::TrailingParameterConvention,
            Self::UnclosedWhen,
            Self::UnexpectedToken,
            Self::UnexpectedTokenInFormatString,
            Self::UnknownCompilerQuery,
            Self::UnnecessaryComma,
            Self::UnsupportedImportAlias,
            Self::UnterminatedFormatString,
            Self::UnusedValue,
        ]
        .into_iter()
    }
}

impl From<SyntaxCode> for crate::DiagnosticCode {
    fn from(code: SyntaxCode) -> Self {
        ParseCode::Syntax(code).into()
    }
}
