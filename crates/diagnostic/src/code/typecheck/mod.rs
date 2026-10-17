use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

mod address;
pub use address::AddressCode;
mod array;
pub use array::ArrayCode;
mod binding;
pub use binding::BindingCode;
mod call;
pub use call::CallCode;
mod domain;
pub use domain::DomainCode;
mod emission;
pub use emission::EmissionCode;
mod generic;
pub use generic::GenericCode;
mod integer;
pub use integer::IntegerCode;
mod intrinsic;
pub use intrinsic::IntrinsicCode;
mod member;
pub use member::MemberCode;
mod operator;
pub use operator::OperatorCode;
mod ownership;
pub use ownership::OwnershipCode;
mod pattern;
pub use pattern::PatternCode;
mod proof;
pub use proof::ProofCode;
mod proposition;
pub use proposition::PropositionCode;
mod reference;
pub use reference::ReferenceCode;
mod result;
pub use result::ResultCode;
mod sequence;
pub use sequence::SequenceCode;
mod symbol;
pub use symbol::SymbolCode;
mod target_group;
pub use target_group::TargetGroupCode;
mod terminal;
pub use terminal::TerminalCode;
mod text;
pub use text::TextCode;
mod traits;
pub use traits::TraitCode;
mod union;
pub use union::UnionCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum TypeCode {
    Address(#[from] AddressCode),
    Array(#[from] ArrayCode),
    Binding(#[from] BindingCode),
    Call(#[from] CallCode),
    Domain(#[from] DomainCode),
    Emission(#[from] EmissionCode),
    Generic(#[from] GenericCode),
    Integer(#[from] IntegerCode),
    Intrinsic(#[from] IntrinsicCode),
    Member(#[from] MemberCode),
    Operator(#[from] OperatorCode),
    Ownership(#[from] OwnershipCode),
    Pattern(#[from] PatternCode),
    Proof(#[from] ProofCode),
    Proposition(#[from] PropositionCode),
    Reference(#[from] ReferenceCode),
    Result(#[from] ResultCode),
    Sequence(#[from] SequenceCode),
    Symbol(#[from] SymbolCode),
    TargetGroup(#[from] TargetGroupCode),
    Terminal(#[from] TerminalCode),
    Text(#[from] TextCode),
    Trait(#[from] TraitCode),
    Union(#[from] UnionCode),
    Mismatch,
}

impl Prefix for TypeCode {
    fn prefix(&self) -> &'static str {
        match self {
            Self::Address(code) => code.prefix(),
            Self::Array(code) => code.prefix(),
            Self::Binding(code) => code.prefix(),
            Self::Call(code) => code.prefix(),
            Self::Domain(code) => code.prefix(),
            Self::Emission(code) => code.prefix(),
            Self::Generic(code) => code.prefix(),
            Self::Integer(code) => code.prefix(),
            Self::Intrinsic(code) => code.prefix(),
            Self::Member(code) => code.prefix(),
            Self::Operator(code) => code.prefix(),
            Self::Ownership(code) => code.prefix(),
            Self::Pattern(code) => code.prefix(),
            Self::Proof(code) => code.prefix(),
            Self::Proposition(code) => code.prefix(),
            Self::Reference(code) => code.prefix(),
            Self::Result(code) => code.prefix(),
            Self::Sequence(code) => code.prefix(),
            Self::Symbol(code) => code.prefix(),
            Self::TargetGroup(code) => code.prefix(),
            Self::Terminal(code) => code.prefix(),
            Self::Text(code) => code.prefix(),
            Self::Trait(code) => code.prefix(),
            Self::Union(code) => code.prefix(),
            Self::Mismatch => "type",
        }
    }
}

impl Code for TypeCode {
    fn name(&self) -> &'static str {
        match self {
            Self::Address(code) => code.name(),
            Self::Array(code) => code.name(),
            Self::Binding(code) => code.name(),
            Self::Call(code) => code.name(),
            Self::Domain(code) => code.name(),
            Self::Emission(code) => code.name(),
            Self::Generic(code) => code.name(),
            Self::Integer(code) => code.name(),
            Self::Intrinsic(code) => code.name(),
            Self::Member(code) => code.name(),
            Self::Operator(code) => code.name(),
            Self::Ownership(code) => code.name(),
            Self::Pattern(code) => code.name(),
            Self::Proof(code) => code.name(),
            Self::Proposition(code) => code.name(),
            Self::Reference(code) => code.name(),
            Self::Result(code) => code.name(),
            Self::Sequence(code) => code.name(),
            Self::Symbol(code) => code.name(),
            Self::TargetGroup(code) => code.name(),
            Self::Terminal(code) => code.name(),
            Self::Text(code) => code.name(),
            Self::Trait(code) => code.name(),
            Self::Union(code) => code.name(),
            Self::Mismatch => "mismatch",
        }
    }
}

impl TypeCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        AddressCode::all()
            .map(Self::Address)
            .chain(ArrayCode::all().map(Self::Array))
            .chain(BindingCode::all().map(Self::Binding))
            .chain(CallCode::all().map(Self::Call))
            .chain(DomainCode::all().map(Self::Domain))
            .chain(EmissionCode::all().map(Self::Emission))
            .chain(GenericCode::all().map(Self::Generic))
            .chain(IntegerCode::all().map(Self::Integer))
            .chain(IntrinsicCode::all().map(Self::Intrinsic))
            .chain(MemberCode::all().map(Self::Member))
            .chain(OperatorCode::all().map(Self::Operator))
            .chain(OwnershipCode::all().map(Self::Ownership))
            .chain(PatternCode::all().map(Self::Pattern))
            .chain(ProofCode::all().map(Self::Proof))
            .chain(PropositionCode::all().map(Self::Proposition))
            .chain(ReferenceCode::all().map(Self::Reference))
            .chain(ResultCode::all().map(Self::Result))
            .chain(SequenceCode::all().map(Self::Sequence))
            .chain(SymbolCode::all().map(Self::Symbol))
            .chain(TargetGroupCode::all().map(Self::TargetGroup))
            .chain(TerminalCode::all().map(Self::Terminal))
            .chain(TextCode::all().map(Self::Text))
            .chain(TraitCode::all().map(Self::Trait))
            .chain(UnionCode::all().map(Self::Union))
            .chain([Self::Mismatch].into_iter())
    }
}
