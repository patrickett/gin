mod codegen;
pub use codegen::CodegenCode;
mod evaluation;
pub use evaluation::EvaluationCode;
mod format;
pub use format::FormatCode;
mod import;
pub use import::ImportCode;
mod interface;
pub use interface::InterfaceCode;
mod interpreter;
pub use interpreter::InterpreterCode;
mod layout;
pub use layout::LayoutCode;
mod lex;
pub use lex::LexCode;
mod package;
pub use package::PackageCode;
mod parse;
pub use parse::{ExpectedSyntaxCode, ParseCode, RemovedSyntaxCode, SyntaxCode};
mod protocol;
pub use protocol::ProtocolCode;
mod representation;
pub use representation::RepresentationCode;
mod typecheck;
pub use typecheck::{
    AddressCode, ArrayCode, BindingCode, CallCode, DomainCode, EmissionCode, GenericCode,
    IntegerCode, IntrinsicCode, MemberCode, OperatorCode, OwnershipCode, PatternCode, ProofCode,
    PropositionCode, ReferenceCode, ResultCode, SequenceCode, SymbolCode, TargetGroupCode,
    TerminalCode, TextCode, TraitCode, TypeCode, UnionCode,
};

mod traits;
use traits::CodeDisplay;
pub use traits::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum DiagnosticCode {
    Codegen(#[from] CodegenCode),
    Evaluation(#[from] EvaluationCode),
    Format(#[from] FormatCode),
    Import(#[from] ImportCode),
    Interface(#[from] InterfaceCode),
    Interpreter(#[from] InterpreterCode),
    Layout(#[from] LayoutCode),
    Lex(#[from] LexCode),
    Package(#[from] PackageCode),
    Parse(#[from] ParseCode),
    Protocol(#[from] ProtocolCode),
    Representation(#[from] RepresentationCode),
    Type(#[from] TypeCode),
    UnfilledHole,
}

impl Prefix for DiagnosticCode {
    fn prefix(&self) -> &'static str {
        match self {
            Self::Codegen(code) => code.prefix(),
            Self::Evaluation(code) => code.prefix(),
            Self::Format(code) => code.prefix(),
            Self::Import(code) => code.prefix(),
            Self::Interface(code) => code.prefix(),
            Self::Interpreter(code) => code.prefix(),
            Self::Layout(code) => code.prefix(),
            Self::Lex(code) => code.prefix(),
            Self::Package(code) => code.prefix(),
            Self::Parse(code) => code.prefix(),
            Self::Protocol(code) => code.prefix(),
            Self::Representation(code) => code.prefix(),
            Self::Type(code) => code.prefix(),
            Self::UnfilledHole => "",
        }
    }
}

impl Code for DiagnosticCode {
    fn name(&self) -> &'static str {
        match self {
            Self::Codegen(code) => code.name(),
            Self::Evaluation(code) => code.name(),
            Self::Format(code) => code.name(),
            Self::Import(code) => code.name(),
            Self::Interface(code) => code.name(),
            Self::Interpreter(code) => code.name(),
            Self::Layout(code) => code.name(),
            Self::Lex(code) => code.name(),
            Self::Package(code) => code.name(),
            Self::Parse(code) => code.name(),
            Self::Protocol(code) => code.name(),
            Self::Representation(code) => code.name(),
            Self::Type(code) => code.name(),
            Self::UnfilledHole => "unfilled-hole",
        }
    }
}

impl DiagnosticCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        CodegenCode::all()
            .map(Self::Codegen)
            .chain(EvaluationCode::all().map(Self::Evaluation))
            .chain(FormatCode::all().map(Self::Format))
            .chain(ImportCode::all().map(Self::Import))
            .chain(InterfaceCode::all().map(Self::Interface))
            .chain(InterpreterCode::all().map(Self::Interpreter))
            .chain(LayoutCode::all().map(Self::Layout))
            .chain(LexCode::all().map(Self::Lex))
            .chain(PackageCode::all().map(Self::Package))
            .chain(ParseCode::all().map(Self::Parse))
            .chain(ProtocolCode::all().map(Self::Protocol))
            .chain(RepresentationCode::all().map(Self::Representation))
            .chain(TypeCode::all().map(Self::Type))
            .chain([Self::UnfilledHole].into_iter())
    }
}

impl serde::Serialize for DiagnosticCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}
