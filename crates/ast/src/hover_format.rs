//! Shared markdown layout for IDE hover content.
//!
//! This module provides [`HoverDoc`] — a builder for consistent hover markdown
//! used by the typed AST and IDE tooling.

use crate::doc_comment::DocComment;

/// Separator between hover sections (matches LSP markdown conventions in this codebase).
pub const SECTION_SEP: &str = "---";

/// One block of hover markdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoverSection {
    GinCode(String),
    /// Type string without surrounding backticks (render adds them).
    Type(String),
    Prose(String),
    SizeAlign {
        size: usize,
        align: usize,
    },
    /// Pre-formatted markdown (e.g. `` `x`: `Int` `` for expression hovers).
    Inline(String),
}

/// Builder for consistent hover markdown.
#[derive(Debug, Clone, Default)]
pub struct HoverDoc {
    sections: Vec<HoverSection>,
    /// Optional module path prefix (`path` line before the first section).
    module_prefix: Option<String>,
}

impl HoverDoc {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_module_prefix(mut self, prefix: String) -> Self {
        self.module_prefix = Some(prefix);
        self
    }

    pub fn gin(mut self, code: impl Into<String>) -> Self {
        self.sections.push(HoverSection::GinCode(code.into()));
        self
    }

    pub fn ty_display(mut self, ty: impl Into<String>) -> Self {
        self.sections.push(HoverSection::Type(ty.into()));
        self
    }

    pub fn prose(mut self, text: impl Into<String>) -> Self {
        self.sections.push(HoverSection::Prose(text.into()));
        self
    }

    pub fn inline(mut self, text: impl Into<String>) -> Self {
        self.sections.push(HoverSection::Inline(text.into()));
        self
    }

    pub fn size_align_from_ty(self, _ty: &crate::ty::Ty, _registry: &(), _typed: &()) -> Self {
        // Simplified stub — typecheck dependency was split out
        self
    }

    pub fn doc_opt(mut self, doc: Option<&DocComment>) -> Self {
        if let Some(doc) = doc {
            self.sections.push(HoverSection::Prose(doc.value.clone()));
        }
        self
    }

    pub fn render(&self) -> String {
        let mut result = String::new();
        if let Some(prefix) = &self.module_prefix {
            result.push_str(&format!("`{prefix}`\n\n"));
        }
        for (i, section) in self.sections.iter().enumerate() {
            if i > 0 {
                result.push_str(&format!("\n{SECTION_SEP}\n\n"));
            }
            match section {
                HoverSection::GinCode(code) => {
                    result.push_str(&format!("```gin\n{code}\n```"));
                }
                HoverSection::Type(ty) => {
                    result.push_str(&format!("`{ty}`"));
                }
                HoverSection::Prose(text) => {
                    result.push_str(text);
                }
                HoverSection::Inline(text) => {
                    result.push_str(text);
                }
                HoverSection::SizeAlign { size, align } => {
                    result.push_str(&format!("size = {size}, align = {align}"));
                }
            }
        }
        result
    }
}
