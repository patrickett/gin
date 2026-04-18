//! Integration tests for comment alignment with `---` separator.

use ginfmt::{Config, format_with_config};

#[test]
fn test_dash_comment_alignment() {
    // Test basic `---` comment alignment
    let input = "--- First comment\n--- Second longer comment\n";
    let output = format_with_config(input, Config::default());

    // The comments should be aligned on the `---`
    // After the first pass, they should have the same spacing before `---`
    let lines: Vec<&str> = output.lines().collect();
    assert!(lines.len() >= 2);

    // Find the position of `---` in each line
    let first_dash_pos = lines[0].find("---").unwrap();
    let second_dash_pos = lines[1].find("---").unwrap();

    // They should be aligned (same position)
    assert_eq!(first_dash_pos, second_dash_pos);
}

#[test]
fn test_mixed_content_with_comments() {
    // Test that regular code and `---` comments coexist properly
    let input =
        "Area is 0...999\n--- Comment about Area\nGroup is 0...99\n--- Comment about Group\n";
    let output = format_with_config(input, Config::default());

    // Should not crash and should produce output
    assert!(!output.is_empty());
    assert!(output.contains("Area"));
    assert!(output.contains("Group"));
}

#[test]
fn test_comment_alignment_disabled() {
    // Test that comment alignment can be disabled
    let input = "--- Short\n--- Much longer comment here\n";
    let output = format_with_config(
        input,
        Config {
            align_comments: false,
            ..Default::default()
        },
    );

    // Comments should remain as-is (no alignment applied)
    assert!(output.contains("--- Short"));
}

#[test]
fn test_comment_alignment_idempotent() {
    // Test that formatting is idempotent for comment alignment
    let input = "--- First\n--- Second\n";
    let first = format_with_config(input, Config::default());
    let second = format_with_config(&first, Config::default());

    assert_eq!(first, second);
}
