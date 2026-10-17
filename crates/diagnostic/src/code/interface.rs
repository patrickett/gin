use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum InterfaceCode {
    InvalidPackage,
    LocalResultFamilyEscapes,
    PublicPrivateIdentity,
}

impl Prefix for InterfaceCode {
    fn prefix(&self) -> &'static str {
        "interface"
    }
}

impl Code for InterfaceCode {
    fn name(&self) -> &'static str {
        match self {
            Self::InvalidPackage => "invalid-package",
            Self::LocalResultFamilyEscapes => "local-result-family-escapes",
            Self::PublicPrivateIdentity => "public-private-identity",
        }
    }
}

impl InterfaceCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::InvalidPackage,
            Self::LocalResultFamilyEscapes,
            Self::PublicPrivateIdentity,
        ]
        .into_iter()
    }
}
