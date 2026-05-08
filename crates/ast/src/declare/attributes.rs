use crate::ArchTarget;
use crate::OsTarget;

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct DeclareAttributes {
    /// OS filter: `#[os({ linux, macos })]`. `None` means no filter (included on all platforms).
    pub os: Option<Vec<OsTarget>>,
    /// Arch filter: `#[arch({ x86_64, arm64 })]`. `None` means no filter.
    pub arch: Option<Vec<ArchTarget>>,
}

impl DeclareAttributes {
    /// Returns `true` if this declaration should be compiled for the current build host.
    pub fn matches_current_platform(&self) -> bool {
        if let Some(targets) = &self.os
            && !targets.iter().any(|t| t.is_current_host())
        {
            return false;
        }
        if let Some(arches) = &self.arch
            && !arches.iter().any(|a| a.is_current_host())
        {
            return false;
        }
        true
    }
}
