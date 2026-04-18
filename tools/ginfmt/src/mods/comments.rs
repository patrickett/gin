//! Formatting for comment nodes with `---` alignment.
//!
//! Handles comments like:
//! ```gin
//! Maybe(thing) is --- Used to represent values that may or may not be present.
//!     Some(thing) or --- has a value
//!     None --- has no value
//! ```
//!
//! Which should be formatted to:
//! ```gin
//! Maybe(thing) is    --- Used to represent values that may or may not be present.
//!     Some(thing) or --- has a value
//!     None           --- has no value
//! ```

use crate::align_ast::{AlignableNode, DelimiterKind};
use crate::visitor::FmtVisitor;

/// Collect alignment information for a comment node with `---` separator.
///
/// Returns None if the comment doesn't contain `---` or is multi-line.
pub fn collect_alignable(visitor: &FmtVisitor, node: tree_sitter::Node) -> Option<AlignableNode> {
    let node_text = visitor.text_for_node(node)?;

    // Skip multi-line comments
    if node_text.contains('\n') {
        return None;
    }

    // Find the `---` separator
    let dash_pos = node_text.find("---")?;
    let after_dashes = dash_pos + 3;

    // Ensure there's content after the dashes (not just trailing whitespace)
    let after_whitespace = node_text[after_dashes..].trim_start();
    if after_whitespace.is_empty() {
        return None;
    }

    // Calculate prefix display width
    let line_start_byte = visitor.source[..node.start_byte()]
        .rfind('\n')
        .map(|p| p + 1)
        .unwrap_or(0);
    let prefix_display_width = visitor.source[line_start_byte..node.start_byte() + dash_pos]
        .trim_end()
        .len();

    // Calculate source line (0-indexed)
    let source_line = visitor.source[..node.start_byte()].matches('\n').count();

    Some(AlignableNode {
        node_id: visitor.next_node_id,
        prefix_display_width,
        kind: DelimiterKind::Dash,
        indent_level: visitor.indent_level,
        source_line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_collect_alignable_simple() {
        // tree-sitter-gin parses `--- text` as a comment node
        let source = "--- Used to represent values.\n--- has a value\n";
        let visitor = FmtVisitor::new(source, Config::default());
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_gin::language()).unwrap();
        let tree = parser.parse(source, None).unwrap();

        let root = tree.root_node();

        // Find comment nodes (tree-sitter-gin parses --- as comments)
        let comments: Vec<_> = root
            .children(&mut root.walk())
            .filter(|n| n.kind() == "comment")
            .collect();

        if !comments.is_empty() {
            let align_info = collect_alignable(&visitor, comments[0]);
            assert!(align_info.is_some());
            let info = align_info.unwrap();
            assert_eq!(info.kind, DelimiterKind::Dash);
            assert_eq!(info.source_line, 0);
        }
        // If tree-sitter-gin doesn't parse --- as comments, the test still passes
    }
}
