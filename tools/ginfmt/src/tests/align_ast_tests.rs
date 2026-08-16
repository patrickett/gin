use super::*;

#[test]
fn test_group_alignable_nodes_consecutive() {
    let nodes = vec![
        AlignableNode {
            prefix_display_width: 4,
            kind: DelimiterKind::Is,
            indent_level: 0,
            source_line: 0,
        },
        AlignableNode {
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
            prefix_display_width: 4,
            kind: DelimiterKind::Is,
            indent_level: 0,
            source_line: 0,
        },
        AlignableNode {
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
            prefix_display_width: 4,
            kind: DelimiterKind::Is,
            indent_level: 0,
            source_line: 0,
        },
        AlignableNode {
            prefix_display_width: 5,
            kind: DelimiterKind::Colon, // Different kind
            indent_level: 0,
            source_line: 1,
        },
    ];

    let groups = group_alignable_nodes(&nodes);
    assert_eq!(groups.len(), 2);
}
