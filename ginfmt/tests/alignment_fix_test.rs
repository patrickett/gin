//! Test for the alignment bug fix
//!
//! This test verifies that the alignment bug is fixed where:
//! - Input:  "Area is 0...999\nGroup is 0...99\nSerial is 0...9999"
//! - Output should be: "Area   is 0...999\nGroup  is 0...99\nSerial is 0...9999"
//! - NOT: "Area    s 0...999\nGroup   s 0...99\nSerial is 0...9999"
//! (where "is" becomes "s" - the "i" is incorrectly removed)

use ginfmt::{Config, format_with_config};

#[test]
fn test_alignment_bug_fix() {
    let source = "Area is 0...999\nGroup is 0...99\nSerial is 0...9999\n";
    let mut config = Config::default();
    config.align_binds = false;
    let output = format_with_config(source, config);

    // Verify "is" is intact in all lines
    assert!(output.contains("is"), "Output should contain 'is'");
    assert!(
        !output.contains(" s "),
        "Output should not contain ' s ' (missing 'i')"
    );

    // Verify proper alignment
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 3);

    // Check that all lines end with correct values
    assert!(lines[0].ends_with("0...999"));
    assert!(lines[1].ends_with("0...99"));
    assert!(lines[2].ends_with("0...9999"));

    // Verify exact alignment
    // "Area" (4 chars) should have 2 spaces before "is"
    // "Group" (5 chars) should have 1 space before "is"
    // "Serial" (6 chars) should have 0 spaces before "is"
    assert_eq!(lines[0], "Area   is 0...999");
    assert_eq!(lines[1], "Group  is 0...99");
    assert_eq!(lines[2], "Serial is 0...9999");
}

#[test]
fn test_alignment_idempotent() {
    let source = "Area is 0...999\nGroup is 0...99\nSerial is 0...9999\n";
    let mut config = Config::default();
    config.align_binds = false;

    // Format once
    let output1 = format_with_config(source, config.clone());

    // Format the output again
    let output2 = format_with_config(&output1, config);

    // Should be identical
    assert_eq!(output1, output2, "Format should be idempotent");
}
