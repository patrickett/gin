use crate::{ArchTarget, AttributeItem, OsTarget, extract_arch_targets, extract_os_targets};

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct DeclareAttributes {
    /// OS filter: `#[os({ linux, macos })]`. `None` means no filter (included on all platforms).
    pub os: Option<Vec<OsTarget>>,
    /// Arch filter: `#[arch({ x86_64, arm64 })]`. `None` means no filter.
    pub arch: Option<Vec<ArchTarget>>,
    /// Raw parsed attributes before semantic extraction.
    /// `None` means no `#[...]` block was present at all.
    /// `Some(vec![])` means an empty `#[]` was present.
    pub raw_attributes: Option<Vec<AttributeItem>>,
    /// Whether this type requires linear usage (`#[lin]`).
    pub is_lin: bool,
    /// Whether this type is explicitly non-copyable (`#[not_copy]`).
    pub is_not_copy: bool,
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

    /// Extract compiler-known intrinsic attributes from `raw_attributes` into typed fields.
    /// Should be called after parsing, before platform filtering.
    pub fn extract_intrinsic_attributes(&mut self) {
        let Some(items) = &self.raw_attributes else {
            return;
        };
        if items.is_empty() {
            return;
        }

        for item in items {
            if let AttributeItem::Call { name, args, .. } = item {
                match name.as_str() {
                    "os" => {
                        self.os = extract_os_targets(args);
                    }
                    "arch" => {
                        self.arch = extract_arch_targets(args);
                    }
                    _ => {}
                }
            } else if let AttributeItem::Flag { name, .. } = item {
                match name.as_str() {
                    "lin" => self.is_lin = true,
                    "not_copy" => self.is_not_copy = true,
                    _ => {}
                }
            }
        }
    }
}
