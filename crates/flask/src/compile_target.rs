//! Entry-package compile target from `flask.jsonc` and CLI overrides.

use std::fmt;

use crate::FlaskConfig;

/// Parsed target triple components for inject into `target Target`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetTriple {
    pub raw: String,
    pub arch: TargetArch,
    pub vendor: String,
    pub os: TargetOs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetArch {
    X86_64,
    Arm64,
    Wasm32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetOs {
    Linux,
    MacOs,
    Windows,
    Unknown,
}

/// Active compile target for a build / IDE session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileTarget {
    /// Package omits `target` or sets `"library"` — no flask triple merge.
    Library,
    /// Concrete triple from entry flask or CLI.
    Concrete(TargetTriple),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetError {
    MissingTarget,
    InvalidTriple { value: String, reason: String },
}

impl fmt::Display for TargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTarget => {
                write!(
                    f,
                    "entry package must set `target` to a full target triple in flask.jsonc"
                )
            }
            Self::InvalidTriple { value, reason } => {
                write!(f, "invalid target triple `{value}`: {reason}")
            }
        }
    }
}

impl std::error::Error for TargetError {}

#[derive(Debug, Clone)]
pub enum TargetKindField {
    Library,
    Triple(String),
}

impl CompileTarget {
    /// Resolve from entry package config and optional CLI triple override.
    pub fn resolve(config: &FlaskConfig, cli_triple: Option<&str>) -> Result<Self, TargetError> {
        if let Some(triple) = cli_triple {
            return TargetTriple::parse(triple).map(CompileTarget::Concrete);
        }
        match config.target_kind()? {
            TargetKindField::Library => Ok(CompileTarget::Library),
            TargetKindField::Triple(s) => TargetTriple::parse(&s).map(CompileTarget::Concrete),
        }
    }

    pub fn is_concrete(&self) -> bool {
        matches!(self, Self::Concrete(_))
    }

    pub fn arch_literal(&self) -> Option<&'static str> {
        match self {
            Self::Library => None,
            Self::Concrete(t) => Some(t.arch.as_str()),
        }
    }

    pub fn os_literal(&self) -> Option<&'static str> {
        match self {
            Self::Library => None,
            Self::Concrete(t) => Some(t.os.as_str()),
        }
    }

    pub fn vendor_literal(&self) -> Option<&str> {
        match self {
            Self::Library => None,
            Self::Concrete(t) => Some(t.vendor.as_str()),
        }
    }

    pub fn triple_raw(&self) -> Option<&str> {
        match self {
            Self::Library => None,
            Self::Concrete(t) => Some(t.raw.as_str()),
        }
    }
}

impl TargetArch {
    pub fn parse(s: &str) -> Result<Self, TargetError> {
        match s {
            "x86_64" | "amd64" => Ok(TargetArch::X86_64),
            "aarch64" | "arm64" => Ok(TargetArch::Arm64),
            "wasm32" => Ok(TargetArch::Wasm32),
            _ => Err(TargetError::InvalidTriple {
                value: s.to_string(),
                reason: format!("unknown arch `{s}`"),
            }),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Arm64 => "arm64",
            Self::Wasm32 => "wasm32",
        }
    }
}

impl TargetOs {
    pub fn parse(s: &str) -> Result<Self, TargetError> {
        match s.to_lowercase().as_str() {
            "linux" => Ok(TargetOs::Linux),
            "darwin" | "macos" => Ok(TargetOs::MacOs),
            "windows" => Ok(TargetOs::Windows),
            "unknown" => Ok(TargetOs::Unknown),
            _ => Err(TargetError::InvalidTriple {
                value: s.to_string(),
                reason: format!("unknown os `{s}`"),
            }),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::MacOs => "macOS",
            Self::Windows => "windows",
            Self::Unknown => "unknown",
        }
    }
}

impl TargetTriple {
    pub fn parse(raw: &str) -> Result<Self, TargetError> {
        let raw = raw.trim();
        if raw == "library" {
            return Err(TargetError::InvalidTriple {
                value: raw.to_string(),
                reason: "library is not a target triple".into(),
            });
        }
        let parts: Vec<&str> = raw.split('-').collect();
        if parts.len() < 3 {
            return Err(TargetError::InvalidTriple {
                value: raw.to_string(),
                reason: "expected arch-vendor-os (at least three components)".into(),
            });
        }
        let arch = TargetArch::parse(parts[0])?;
        let vendor = parts[1].to_string();
        let os = TargetOs::parse(parts[2])?;
        Ok(Self {
            raw: raw.to_string(),
            arch,
            vendor,
            os,
        })
    }

    /// Field overrides for merging into a `Target` const record.
    pub fn field_overrides(&self) -> [(&'static str, String); 3] {
        [
            ("arch", self.arch.as_str().to_string()),
            ("vendor", self.vendor.clone()),
            ("os", self.os.as_str().to_string()),
        ]
    }
}

impl FlaskConfig {
    /// `target` from flask: missing → library.
    pub fn target_kind(&self) -> Result<TargetKindField, TargetError> {
        match self.target.as_deref() {
            None => Ok(TargetKindField::Library),
            Some("library") => Ok(TargetKindField::Library),
            Some(s) => Ok(TargetKindField::Triple(s.to_string())),
        }
    }

    /// Entry apps must declare a concrete triple.
    pub fn require_entry_triple(&self) -> Result<(), TargetError> {
        match self.target_kind()? {
            TargetKindField::Library => Err(TargetError::MissingTarget),
            TargetKindField::Triple(s) => {
                TargetTriple::parse(&s)?;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_linux_triple() {
        let t = TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(t.arch, TargetArch::X86_64);
        assert_eq!(t.os, TargetOs::Linux);
    }

    #[test]
    fn parse_wasm_triple() {
        let t = TargetTriple::parse("wasm32-unknown-unknown").unwrap();
        assert_eq!(t.arch, TargetArch::Wasm32);
        assert_eq!(t.os, TargetOs::Unknown);
    }

    #[test]
    fn reject_bare_arch() {
        assert!(TargetTriple::parse("x86_64").is_err());
    }

    #[test]
    fn deny_unknown_targets_field() {
        let raw = r#"{"name":"x","version":"0","authors":[],"targets":[]}"#;
        let err = json5::from_str::<FlaskConfig>(raw).unwrap_err();
        assert!(err.to_string().contains("targets"));
    }
}
