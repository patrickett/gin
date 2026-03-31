//! Formatting for declaration nodes (e.g., `Area is 0...999`).

use crate::visitor::FmtVisitor;
use crate::align_ast::{AlignableNode, DelimiterKind};

/// Rewrite a declaration node with proper formatting and alignment.
///
/// Declarations have the form:
/// ```gin
/// Tag is <range_literal>
/// Tag params... is <range_literal>
/// ```
pub fn rewrite(visitor: &mut FmtVisitor, node: tree_sitter::Node) -> Option<String> {
    // For now, just preserve the original text
    // TODO: Implement alignment logic
    visitor.text_for_node(node)
}

/// Collect alignment information for a declaration node.
///
/// Returns None if the declaration is not alignable (e.g., multi-line, complex params).
pub fn collect_alignable(visitor: &FmtVisitor, node: tree_sitter::Node) -> Option<AlignableNode> {
    // Check if this is a multi-line declaration - skip alignment for multi-line
    let node_text = visitor.text_for_node(node)?;
    if node_text.contains('\n') {
        return None; // Skip multi-line declarations like sum types
    }

    // Walk the AST children to find the "is" keyword and prefix
    let mut cursor = node.walk();
    let mut prefix_end = node.start_byte();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "tag" => {
                // First part of prefix
                prefix_end = child.end_byte();
            }
            "params" => {
                // Includes parameters in prefix
                prefix_end = child.end_byte();
            }
            "type_params" => {
                // Includes type parameters in prefix
                prefix_end = child.end_byte();
            }
            "is" => {
                // Calculate prefix display width
                let line_start_byte = visitor.source[..prefix_end]
                    .rfind('\n')
                    .map(|p| p + 1)
                    .unwrap_or(0);
                let prefix_display_width = visitor.source[line_start_byte..prefix_end].len();

                // Calculate source line (0-indexed)
                let source_line = visitor.source[..prefix_end].matches('\n').count();

                return Some(AlignableNode {
                    node_id: visitor.next_node_id,
                    prefix_display_width,
                    kind: DelimiterKind::Is,
                    indent_level: visitor.indent_level,
                    source_line,
                });
            }
            _ => {}
        }
    }

    // No "is" keyword found, not alignable
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_simple_declaration() {
        let source = "Area is 0...999\n";
        let mut visitor = FmtVisitor::new(source, Config::default());
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_gin::language()).unwrap();
        let tree = parser.parse(source, None).unwrap();

        let root = tree.root_node();
        let decl = root.child(0).unwrap();
        assert_eq!(decl.kind(), "declaration");

        let result = rewrite(&mut visitor, decl);
        assert_eq!(result, Some("Area is 0...999".to_string()));
    }

    #[test]
    fn test_collect_alignable_simple() {
        let source = "Area is 0...999\n";
        let visitor = FmtVisitor::new(source, Config::default());
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_gin::language()).unwrap();
        let tree = parser.parse(source, None).unwrap();

        let root = tree.root_node();
        let decl = root.child(0).unwrap();

        let align_info = collect_alignable(&visitor, decl);
        assert!(align_info.is_some());
        let info = align_info.unwrap();
        assert_eq!(info.kind, DelimiterKind::Is);
        assert_eq!(info.prefix_display_width, 4); // "Area"
        assert_eq!(info.source_line, 0);
    }
}
