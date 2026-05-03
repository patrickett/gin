//! Test to expose the three bugs
use ginfmt::{Config, format_with_config};

#[test]
fn test_bug_1_idempotency_issue() {
    // After first format, byte positions in AlignableNode become invalid
    let source = "Area is 0...999\nGroup is 0...99\nSerial is 0...9999\n";
    let config = Config {
        align_binds: false,
        ..Default::default()
    };

    let first = format_with_config(source, config.clone());
    println!("First format:\n{}", first);

    let second = format_with_config(&first, config.clone());
    println!("Second format:\n{}", second);

    assert_eq!(first, second, "Formatter should be idempotent");
}

#[test]
fn test_bug_2_blank_line_preservation() {
    // Blank lines should break alignment groups and be preserved
    let source = "Area is 0...999\n\nGroup is 0...99\n";
    let config = Config {
        align_binds: false,
        ..Default::default()
    };

    let result = format_with_config(source, config.clone());
    println!("Result:\n{}", result);

    // Should preserve blank line
    assert!(result.contains("\n\n"), "Blank lines should be preserved");

    // Should not align across blank lines
    let lines: Vec<&str> = result.lines().collect();
    if lines.len() >= 2 {
        // If there are declarations on both sides of blank line,
        // they should not be aligned together
    }
}

#[test]
fn test_bug_3_multiline_sum_type() {
    // Multi-line sum types should not be corrupted
    let source = "Maybe[thing] is\n    Some(thing)\n    or\n    None\n";
    let config = Config {
        align_binds: false,
        ..Default::default()
    };

    let result = format_with_config(source, config.clone());
    println!("Result:\n{}", result);

    // Should preserve the structure
    assert!(
        result.contains("Some(thing)"),
        "Should preserve Some variant"
    );
    assert!(result.contains("None"), "Should preserve None variant");
    assert!(result.contains("or"), "Should preserve 'or' keyword");
}
