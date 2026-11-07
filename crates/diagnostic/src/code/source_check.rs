use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

/// Failures from the independent source checker, not production type-check diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum SourceCheckCode {
    Budget,
    Malformed,
    Unsupported,
    Duplicate,
    UnknownName,
    MissingBody,
    Type,
    Unproved,
    Overflow,
    UseAfterMove,
    Unconsumed,
    Privacy,
    InstructionTarget,
    WrapTarget,
    UnrestrictedTarget,
    UnrestrictedField,
    MethodReceiver,
    MissingTerminal,
    UnrestrictedArgument,
    InfixOperator,
    RedundantTerminal,
    UnreachableCode,
    MoveOutOfBorrow,
    EndEscape,
    BorrowConflict,
    Immutable,
    OverwriteLinear,
    BorrowEscape,
    /// An unchecked operation is not inside `#unchecked(...)`.
    MissingAcceptance,
    /// `#unchecked(...)` surrounds no operation that skips a proof.
    UnneededAcceptance,
    /// A function's callee outcomes multiply past the cap on combinations.
    WorldLimit,
    /// An explicit `#todo` hole: accepted, and runs until it is reached.
    Hole,
}

impl Prefix for SourceCheckCode {
    fn prefix(&self) -> &'static str {
        "source-check"
    }
}

impl Code for SourceCheckCode {
    fn category(&self) -> crate::Category {
        match self {
            Self::RedundantTerminal | Self::UnreachableCode | Self::Hole => crate::Category::Warn,
            _ => crate::Category::Flaw,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Budget => "budget",
            Self::Malformed => "malformed",
            Self::Unsupported => "unsupported",
            Self::Duplicate => "duplicate",
            Self::UnknownName => "unknown-name",
            Self::MissingBody => "missing-body",
            Self::Type => "type",
            Self::Unproved => "unproved",
            Self::Overflow => "overflow",
            Self::UseAfterMove => "use-after-move",
            Self::Unconsumed => "unconsumed",
            Self::Privacy => "privacy",
            Self::InstructionTarget => "instruction-target",
            Self::WrapTarget => "wrap-target",
            Self::UnrestrictedTarget => "unrestricted-target",
            Self::UnrestrictedField => "unrestricted-field",
            Self::MethodReceiver => "method-receiver",
            Self::MissingTerminal => "missing-terminal",
            Self::UnrestrictedArgument => "unrestricted-argument",
            Self::InfixOperator => "infix-operator",
            Self::RedundantTerminal => "redundant-terminal",
            Self::UnreachableCode => "unreachable-code",
            Self::MoveOutOfBorrow => "move-out-of-borrow",
            Self::EndEscape => "end-escape",
            Self::BorrowConflict => "borrow-conflict",
            Self::Immutable => "immutable",
            Self::OverwriteLinear => "overwrite-linear",
            Self::BorrowEscape => "borrow-escape",
            Self::MissingAcceptance => "missing-acceptance",
            Self::UnneededAcceptance => "unneeded-acceptance",
            Self::WorldLimit => "world-limit",
            Self::Hole => "hole",
        }
    }
}

impl SourceCheckCode {
    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::Budget,
            Self::Malformed,
            Self::Unsupported,
            Self::Duplicate,
            Self::UnknownName,
            Self::MissingBody,
            Self::Type,
            Self::Unproved,
            Self::Overflow,
            Self::UseAfterMove,
            Self::Unconsumed,
            Self::Privacy,
            Self::InstructionTarget,
            Self::WrapTarget,
            Self::UnrestrictedTarget,
            Self::UnrestrictedField,
            Self::MethodReceiver,
            Self::MissingTerminal,
            Self::UnrestrictedArgument,
            Self::InfixOperator,
            Self::RedundantTerminal,
            Self::UnreachableCode,
            Self::MoveOutOfBorrow,
            Self::EndEscape,
            Self::BorrowConflict,
            Self::Immutable,
            Self::OverwriteLinear,
            Self::BorrowEscape,
            Self::MissingAcceptance,
            Self::UnneededAcceptance,
            Self::WorldLimit,
            Self::Hole,
        ]
        .into_iter()
    }
}
