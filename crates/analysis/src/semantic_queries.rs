//! Markdown formatting helpers for hover text.

/// Prefix hover body with a gin code block showing the module path (for tree-sitter
/// syntax highlighting).
///
/// `defining_module` is a qualified type path when known (e.g. `core.primitive.Bool` for `False`).
pub fn format_hover_markdown(hover_text: &str, defining_module: Option<&str>) -> String {
    match defining_module {
        Some(name) => format!("```gin\n{name}\n```\n\n{hover_text}"),
        None => hover_text.to_string(),
    }
}
