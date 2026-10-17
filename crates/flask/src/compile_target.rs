//! Entry-package compile target from `mod.flask` and CLI overrides.

use std::borrow::Cow;

use target_lexicon::{Architecture, Triple};
use thiserror::Error;

use crate::FlaskConfig;

/// A parsed [`target_lexicon::Triple`] plus its canonical string form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetTriple {
    pub raw: String,
    triple: Triple,
}

/// Active compile target for a build / IDE session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileTarget {
    /// Package omits `target` or sets `"library"` — no flask triple merge.
    Library,
    /// Concrete triple from entry flask or CLI.
    Concrete(TargetTriple),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TargetError {
    #[error("entry package must set `target` to a full target triple in mod.flask")]
    MissingTarget,
    #[error("invalid target triple `{value}`: {reason}")]
    InvalidTriple { value: String, reason: String },
}

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

    pub fn arch_literal(&self) -> Option<Cow<'static, str>> {
        match self {
            Self::Library => None,
            Self::Concrete(t) => Some(t.arch_literal()),
        }
    }

    pub fn os_literal(&self) -> Option<Cow<'static, str>> {
        match self {
            Self::Library => None,
            Self::Concrete(t) => Some(t.os_literal()),
        }
    }

    pub fn triple_raw(&self) -> Option<&str> {
        match self {
            Self::Library => None,
            Self::Concrete(t) => Some(t.raw.as_str()),
        }
    }
}

impl TargetTriple {
    /// The triple of the machine running this code, for native toolchain defaults.
    pub fn host() -> Result<Self, TargetError> {
        let arch = match std::env::consts::ARCH {
            "aarch64" => "aarch64",
            "x86_64" => "x86_64",
            _ => {
                return Err(TargetError::InvalidTriple {
                    value: std::env::consts::ARCH.to_string(),
                    reason: "unsupported host architecture".into(),
                });
            }
        };
        let suffix = match std::env::consts::OS {
            "linux" => "unknown-linux-gnu",
            "macos" => "apple-darwin",
            "windows" => "pc-windows-msvc",
            _ => {
                return Err(TargetError::InvalidTriple {
                    value: std::env::consts::OS.to_string(),
                    reason: "unsupported host operating system".into(),
                });
            }
        };
        Self::parse(&format!("{arch}-{suffix}"))
    }

    pub fn parse(raw: &str) -> Result<Self, TargetError> {
        let raw = raw.trim();
        if raw == "library" {
            return Err(TargetError::InvalidTriple {
                value: raw.to_string(),
                reason: "library is not a target triple".into(),
            });
        }
        if raw.split('-').count() < 3 {
            return Err(TargetError::InvalidTriple {
                value: raw.to_string(),
                reason: "expected arch-vendor-os (at least three components)".into(),
            });
        }
        let triple = raw
            .parse::<Triple>()
            .map_err(|error| TargetError::InvalidTriple {
                value: raw.to_string(),
                reason: error.to_string(),
            })?;
        Ok(Self {
            raw: triple.to_string(),
            triple,
        })
    }

    pub fn architecture(&self) -> Architecture {
        self.triple.architecture
    }

    pub fn arch_literal(&self) -> Cow<'static, str> {
        self.triple.architecture.into_str()
    }

    pub fn os_literal(&self) -> Cow<'static, str> {
        self.triple.operating_system.into_str()
    }

    pub fn vendor_literal(&self) -> &str {
        self.triple.vendor.as_str()
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
