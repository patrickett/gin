use super::ParseCode;
use crate::DiagnosticCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum ExpectedSyntaxCode {
    AnnotationSeparator,
    ArgumentSeparator,
    Attribute,
    BindOperator,
    Comparison,
    DeclarationBody,
    DeclarationKeyword,
    DiscriminantInteger,
    ExportSeparator,
    Expression,
    FieldName,
    Identifier,
    ImportExport,
    ImportSource,
    In,
    Indentation,
    IntegerRange,
    InterfaceName,
    ListSeparator,
    MethodName,
    Operator,
    OwnedStorageType,
    ParameterSeparator,
    Predicate,
    PredicateValue,
    ProofClose,
    ProofMember,
    ProofRange,
    ProofRelation,
    ProofTerm,
    QueryName,
    ResultAlternative,
    ResultType,
    Tag,
    Then,
    Token,
    TraitName,
    TypeName,
    TypeOrRange,
    Variant,
}

impl Prefix for ExpectedSyntaxCode {
    fn prefix(&self) -> &'static str {
        "parse-expected"
    }
}

impl Code for ExpectedSyntaxCode {
    fn name(&self) -> &'static str {
        match self {
            Self::AnnotationSeparator => "annotation-separator",
            Self::ArgumentSeparator => "arg-separator",
            Self::Attribute => "attribute",
            Self::BindOperator => "bind-operator",
            Self::Comparison => "comparison",
            Self::DeclarationBody => "declare-body",
            Self::DeclarationKeyword => "declare-keyword",
            Self::DiscriminantInteger => "discriminant-integer",
            Self::ExportSeparator => "export-separator",
            Self::Expression => "expression",
            Self::FieldName => "field-name",
            Self::Identifier => "identifier",
            Self::ImportExport => "import-export",
            Self::ImportSource => "import-source",
            Self::In => "in",
            Self::Indentation => "indent",
            Self::IntegerRange => "integer-range",
            Self::InterfaceName => "interface-name",
            Self::ListSeparator => "list-separator",
            Self::MethodName => "method-name",
            Self::Operator => "operator",
            Self::OwnedStorageType => "owned-storage-type",
            Self::ParameterSeparator => "param-separator",
            Self::Predicate => "predicate",
            Self::PredicateValue => "predicate-value",
            Self::ProofClose => "proof-close",
            Self::ProofMember => "proof-member",
            Self::ProofRange => "proof-range",
            Self::ProofRelation => "proof-relation",
            Self::ProofTerm => "proof-term",
            Self::QueryName => "query-name",
            Self::ResultAlternative => "result-alternative",
            Self::ResultType => "result-type",
            Self::Tag => "tag",
            Self::Then => "then",
            Self::Token => "token",
            Self::TraitName => "trait-name",
            Self::TypeName => "type-name",
            Self::TypeOrRange => "type-or-range",
            Self::Variant => "variant",
        }
    }
}

impl ExpectedSyntaxCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::AnnotationSeparator,
            Self::ArgumentSeparator,
            Self::Attribute,
            Self::BindOperator,
            Self::Comparison,
            Self::DeclarationBody,
            Self::DeclarationKeyword,
            Self::DiscriminantInteger,
            Self::ExportSeparator,
            Self::Expression,
            Self::FieldName,
            Self::Identifier,
            Self::ImportExport,
            Self::ImportSource,
            Self::In,
            Self::Indentation,
            Self::IntegerRange,
            Self::InterfaceName,
            Self::ListSeparator,
            Self::MethodName,
            Self::Operator,
            Self::OwnedStorageType,
            Self::ParameterSeparator,
            Self::Predicate,
            Self::PredicateValue,
            Self::ProofClose,
            Self::ProofMember,
            Self::ProofRange,
            Self::ProofRelation,
            Self::ProofTerm,
            Self::QueryName,
            Self::ResultAlternative,
            Self::ResultType,
            Self::Tag,
            Self::Then,
            Self::Token,
            Self::TraitName,
            Self::TypeName,
            Self::TypeOrRange,
            Self::Variant,
        ]
        .into_iter()
    }
}

impl From<ExpectedSyntaxCode> for crate::DiagnosticCode {
    fn from(code: ExpectedSyntaxCode) -> Self {
        DiagnosticCode::Parse(ParseCode::Expected(code))
    }
}
