//! Markdown formatting helpers for hover text.
//!
//! These are exposed via the [`HoverDocExt`] extension trait so callers can
//! use them as `ast::HoverDoc::format_markdown(...)`.

use ast::HoverDoc;

/// Extension trait providing markdown formatting helpers on [`HoverDoc`].
///
/// These are associated functions (not methods) because they operate on
/// already-rendered hover text rather than a `HoverDoc` builder.
pub trait HoverDocExt {
    /// Prefix hover body with a gin code block showing the module path (for tree-sitter
    /// syntax highlighting).
    ///
    /// `defining_module` is a qualified type path when known (e.g. `core.primitive.Bool` for `False`).
    fn format_markdown(hover_text: &str, defining_module: Option<&str>) -> String;
}

impl HoverDocExt for HoverDoc {
    fn format_markdown(hover_text: &str, defining_module: Option<&str>) -> String {
        match defining_module {
            Some(name) => format!("```gin\n{name}\n```\n\n{hover_text}"),
            None => hover_text.to_string(),
        }
    }
}
