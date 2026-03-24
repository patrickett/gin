//! AST-based alignment logic for Gin formatter.
//!
//! This module provides the types and functions for aligning declarations
//! and bind statements using a two-pass approach:
//! 1. Collection pass: Walk AST and collect alignable nodes
//! 2. Alignment pass: Group nodes and calculate padding

/// The type of delimiter being aligned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DelimiterKind {
    /// For "is" declarations (e.g., `Area is 0...999`)
    Is,
    /// For ":" bind statements (e.g., `start: Instant.now`)
    Colon,
    /// For "---" comment separators (e.g., `Maybe(thing) is --- Used to represent...`)
    Dash,
}

/// Information about an alignable AST node.
///
/// This struct stores the information needed to align a node with others
/// of the same kind at the same indentation level.
#[derive(Debug, Clone)]
pub struct AlignableNode {
    /// Unique identifier for the node (assigned during collection)
    pub node_id: usize,
    /// Display width of the prefix (before the delimiter)
    pub prefix_display_width: usize,
    /// Type of delimiter
    pub kind: DelimiterKind,
    /// Indentation level (from AST structure)
    pub indent_level: usize,
    /// Source line number (0-indexed, for grouping consecutive lines)
    pub source_line: usize,
}

/// Group alignable nodes by kind, indent level, and consecutive lines.
///
/// Returns a vector of groups, where each group contains node indices that should
/// be aligned together.
pub fn group_alignable_nodes(nodes: &[AlignableNode]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut current_group: Vec<usize> = Vec::new();

    for (i, node) in nodes.iter().enumerate() {
        if let Some(&prev_i) = current_group.last() {
            let prev = &nodes[prev_i];
            let consecutive = node.source_line == prev.source_line + 1;
            let same_kind = node.kind == prev.kind;
            let same_indent = node.indent_level == prev.indent_level;

            if consecutive && same_kind && (same_indent || node.kind == DelimiterKind::Dash) {
                current_group.push(i);
            } else {
                if !current_group.is_empty() {
                    groups.push(current_group);
                }
                current_group = vec![i];
            }
        } else {
            current_group.push(i);
        }
    }

    if !current_group.is_empty() {
        groups.push(current_group);
    }

    groups
}

/// Calculate the display length of the prefix on a node's line.
///
/// Returns the stored display width of the prefix.
pub fn calculate_prefix_display_len(_source: &str, node: &AlignableNode) -> usize {
    node.prefix_display_width
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_group_alignable_nodes_consecutive() {
        let nodes = vec![
            AlignableNode {
                node_id: 0,
                prefix_display_width: 4,
                kind: DelimiterKind::Is,
                indent_level: 0,
                source_line: 0,
            },
            AlignableNode {
                node_id: 1,
                prefix_display_width: 5,
                kind: DelimiterKind::Is,
                indent_level: 0,
                source_line: 1,
            },
        ];

        let groups = group_alignable_nodes(&nodes);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 2);
    }

    #[test]
    fn test_group_alignable_nodes_non_consecutive() {
        let nodes = vec![
            AlignableNode {
                node_id: 0,
                prefix_display_width: 4,
                kind: DelimiterKind::Is,
                indent_level: 0,
                source_line: 0,
            },
            AlignableNode {
                node_id: 1,
                prefix_display_width: 6,
                kind: DelimiterKind::Is,
                indent_level: 0,
                source_line: 2, // Skip line 1 - not consecutive
            },
        ];

        let groups = group_alignable_nodes(&nodes);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].len(), 1);
        assert_eq!(groups[1].len(), 1);
    }

    #[test]
    fn test_group_alignable_nodes_different_kind() {
        let nodes = vec![
            AlignableNode {
                node_id: 0,
                prefix_display_width: 4,
                kind: DelimiterKind::Is,
                indent_level: 0,
                source_line: 0,
            },
            AlignableNode {
                node_id: 1,
                prefix_display_width: 5,
                kind: DelimiterKind::Colon, // Different kind
                indent_level: 0,
                source_line: 1,
            },
        ];

        let groups = group_alignable_nodes(&nodes);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn test_calculate_prefix_display_len() {
        let source = "Area is 0...999\n";
        let node = AlignableNode {
            node_id: 0,
            prefix_display_width: 4, // "Area"
            kind: DelimiterKind::Is,
            indent_level: 0,
            source_line: 0,
        };

        let len = calculate_prefix_display_len(source, &node);
        assert_eq!(len, 4); // "Area"
    }
}
