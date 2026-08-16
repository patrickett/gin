use crate::PublicInterface;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InterfacePublication {
    Published(PublicInterface),
    Stale(PublicInterface),
    Unavailable,
}

impl InterfacePublication {
    pub fn resolve(
        candidate: PublicInterface,
        has_fatal_flaws: bool,
        previous: Option<PublicInterface>,
    ) -> Self {
        if !has_fatal_flaws {
            return Self::Published(candidate);
        }
        previous.map_or(Self::Unavailable, Self::Stale)
    }

    pub fn authoritative(&self) -> Option<&PublicInterface> {
        match self {
            Self::Published(interface) | Self::Stale(interface) => Some(interface),
            Self::Unavailable => None,
        }
    }

    pub fn is_stale(&self) -> bool {
        matches!(self, Self::Stale(_))
    }
}
