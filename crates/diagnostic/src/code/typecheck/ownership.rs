use super::TypeCode;
use crate::DiagnosticCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum OwnershipCode {
    CyclicPlaceRequiresContract,
    MoveOutOfBorrow,
    MutableBorrowFromImmutable,
    MutableBorrowFromShared,
    ParameterNotConsumed,
    StorageAuthority,
    StorageBorrowedPayload,
    StorageElementMismatch,
    StorageFieldInvalid,
    StorageOrigin,
    UseAfterMove,
    UseOfMaybeMovedValue,
    UseOfMovedValue,
    ValueNotConsumed,
}

impl Prefix for OwnershipCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for OwnershipCode {
    fn name(&self) -> &'static str {
        match self {
            Self::CyclicPlaceRequiresContract => "cyclic-place-requires-explicit-contract",
            Self::MoveOutOfBorrow => "move-out-of-borrow",
            Self::MutableBorrowFromImmutable => "mutable-borrow-from-immutable",
            Self::MutableBorrowFromShared => "mutable-borrow-from-shared",
            Self::ParameterNotConsumed => "owned-param-not-consumed",
            Self::StorageAuthority => "storage-authority",
            Self::StorageBorrowedPayload => "storage-borrowed-payload",
            Self::StorageElementMismatch => "owned-storage-element-mismatch",
            Self::StorageFieldInvalid => "invalid-owned-storage-field",
            Self::StorageOrigin => "owned-storage-origin",
            Self::UseAfterMove => "use-after-move",
            Self::UseOfMaybeMovedValue => "use-of-maybe-moved-value",
            Self::UseOfMovedValue => "use-of-moved-value",
            Self::ValueNotConsumed => "owned-value-not-consumed",
        }
    }
}

impl OwnershipCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::CyclicPlaceRequiresContract,
            Self::MoveOutOfBorrow,
            Self::MutableBorrowFromImmutable,
            Self::MutableBorrowFromShared,
            Self::ParameterNotConsumed,
            Self::StorageAuthority,
            Self::StorageBorrowedPayload,
            Self::StorageElementMismatch,
            Self::StorageFieldInvalid,
            Self::StorageOrigin,
            Self::UseAfterMove,
            Self::UseOfMaybeMovedValue,
            Self::UseOfMovedValue,
            Self::ValueNotConsumed,
        ]
        .into_iter()
    }
}

impl From<OwnershipCode> for crate::DiagnosticCode {
    fn from(code: OwnershipCode) -> Self {
        DiagnosticCode::Type(TypeCode::Ownership(code))
    }
}
