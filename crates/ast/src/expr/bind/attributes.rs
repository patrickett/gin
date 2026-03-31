/// Target operating systems for `#[os({ ... })]` cfg filters.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OsTarget {
    Linux,
    MacOS,
    Windows,
    Unknown,
}

impl OsTarget {
    fn is_current_host(&self) -> bool {
        match self {
            OsTarget::Linux => cfg!(target_os = "linux"),
            OsTarget::MacOS => cfg!(target_os = "macos"),
            OsTarget::Windows => cfg!(target_os = "windows"),
            OsTarget::Unknown => false,
        }
    }
}

/// Target CPU architectures for `#[arch({ ... })]` cfg filters.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ArchTarget {
    X86_64,
    Arm64,
    Wasm32,
}

impl ArchTarget {
    fn is_current_host(&self) -> bool {
        match self {
            ArchTarget::X86_64 => cfg!(target_arch = "x86_64"),
            ArchTarget::Arm64 => cfg!(target_arch = "aarch64"),
            ArchTarget::Wasm32 => cfg!(target_arch = "wasm32"),
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash)]
pub struct BindAttributes {
    /// Always run in tests (`#[test]`).
    pub test: bool,
    /// Always inline (`#[inline]`).
    pub inline_always: bool,
    /// OS filter: `#[os({ linux, macos })]`. `None` means no filter (included on all platforms).
    pub os: Option<Vec<OsTarget>>,
    /// Arch filter: `#[arch({ x86_64, arm64 })]`. `None` means no filter.
    pub arch: Option<Vec<ArchTarget>>,
    /// Strip in release builds (`#[debug]`).
    pub debug_only: bool,
}

impl BindAttributes {
    /// Returns `true` if this bind should be compiled for the current build host.
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
