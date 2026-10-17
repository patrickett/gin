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
    Prose(String),
    /// Pre-formatted markdown (e.g. `` `x`: `Int` `` for expression hovers).
    Inline(String),
}

/// Builder for consistent hover markdown.
#[derive(Debug, Clone, Default)]
pub struct HoverDoc {
    sections: Vec<HoverSection>,
}

impl HoverDoc {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn gin(mut self, code: impl Into<String>) -> Self {
        self.sections.push(HoverSection::GinCode(code.into()));
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

    pub fn doc_opt(mut self, doc: Option<&DocComment>) -> Self {
        if let Some(doc) = doc {
            self.sections.push(HoverSection::Prose(doc.value.clone()));
        }
        self
    }

    pub fn render(&self) -> String {
        let mut result = String::new();
        for (i, section) in self.sections.iter().enumerate() {
            if i > 0 {
                result.push_str(&format!("\n{SECTION_SEP}\n\n"));
            }
            match section {
                HoverSection::GinCode(code) => {
                    result.push_str(&format!("```gin\n{code}\n```"));
                }
                HoverSection::Prose(text) => {
                    result.push_str(text);
                }
                HoverSection::Inline(text) => {
                    result.push_str(text);
                }
            }
        }
        result
    }
}
