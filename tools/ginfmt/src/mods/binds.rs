//! Formatting for bind statement nodes (e.g., `start: Instant.now`).

use crate::align_ast::{AlignableNode, DelimiterKind};
use crate::visitor::FmtVisitor;

/// Rewrite a bind_statement node with proper formatting and alignment.
///
/// Bind statements have the form:
/// ```gin
/// name: <value>
/// ```
pub fn rewrite(visitor: &mut FmtVisitor, node: tree_sitter::Node) -> Option<String> {
    // For now, just preserve the original text
    // TODO: Implement alignment logic
    visitor.text_for_node(node)
}

/// Collect alignment information for a bind statement.
///
/// Bind statements in Gin have: identifier ':' repeat1(statement) optional('return')
/// For alignment, we look for the ':' after the identifier.
pub fn collect_alignable(visitor: &FmtVisitor, node: tree_sitter::Node) -> Option<AlignableNode> {
    // Check if this is a multi-line bind - skip alignment for multi-line
    let node_text = visitor.text_for_node(node)?;
    if node_text.contains('\n') {
        return None; // Skip multi-line bind statements
    }

    // Walk AST to find colon
    let mut cursor = node.walk();
    let mut colon_pos = None;

    for child in node.children(&mut cursor) {
        if child.kind() == ":" {
            colon_pos = Some(child.start_byte());
        }
    }

    let colon = colon_pos?;

    // Find where the value starts (after colon and whitespace)
    // Skip the colon itself first
    let after_colon = colon + 1; // +1 to skip the colon character
    let value_start = if after_colon < visitor.source.len() {
        visitor.source[after_colon..]
            .find(|c: char| !c.is_whitespace() && c != '\n')
            .map(|offset| after_colon + offset)
            .unwrap_or(after_colon)
    } else {
        return None;
    };

    // Only align if there's content after colon on the same line
    if value_start >= visitor.source.len() || visitor.source[colon..value_start].contains('\n') {
        return None;
    }

    // Calculate prefix display width (includes the colon)
    let line_start_byte = visitor.source[..colon]
        .rfind('\n')
        .map(|p| p + 1)
        .unwrap_or(0);
    // For colon binds, prefix includes the colon
    let prefix_display_width = (colon + 1) - line_start_byte;

    // Calculate source line (0-indexed)
    let source_line = visitor.source[..colon].matches('\n').count();

    Some(AlignableNode {
        node_id: visitor.next_node_id,
        prefix_display_width,
        kind: DelimiterKind::Colon,
        indent_level: visitor.indent_level,
        source_line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_simple_bind() {
        // Based on the tree-sitter-gin grammar, a valid bind_statement has:
        // identifier ':' repeat1(statement) optional('return')
        // The statements must be expr, for_statement, or if_statement
        let source = "main:\n    print('hello')\nreturn\n";
        let mut visitor = FmtVisitor::new(source, Config::default());
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_gin::language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();

        let root = tree.root_node();

        // Find the bind_statement
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() == "bind_statement" {
                let result = rewrite(&mut visitor, child);
                assert!(result.is_some());
                return;
            }
        }

        panic!("No bind_statement found in parsed tree");
    }

    #[test]
    fn test_collect_alignable_simple_bind() {
        // Test with a simple bind statement at top level
        // Note: Based on the grammar, nested bind statements like "start: Instant.now"
        // inside another bind are not parsed as bind_statement nodes - they're parsed
        // as part of the expression. So we test the lexer-based alignment directly.
        let source = "main:\n    print('hello')\nreturn\n";
        let visitor = FmtVisitor::new(source, Config::default());
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_gin::language())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();

        let root = tree.root_node();

        // Find the bind_statement
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() == "bind_statement" {
                // The main bind has a colon - let's verify we can collect align info
                // Note: This bind won't be alignable since the value starts on the next line
                let align_info = collect_alignable(&visitor, child);
                // Should return None since the value is on the next line
                assert!(align_info.is_none());
                return;
            }
        }

        panic!("No bind_statement found in parsed tree");
    }
}
