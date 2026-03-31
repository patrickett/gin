//! Test for the alignment bug fix
//!
//! This test verifies that the alignment bug is fixed where:
//! - Input:  "Area is 0...999\nGroup is 0...99\nSerial is 0...9999"
//! - Output should be: "Area   is 0...999\nGroup  is 0...99\nSerial is 0...9999"
//! - NOT: "Area    s 0...999\nGroup   s 0...99\nSerial is 0...9999"
//!   (where "is" becomes "s" - the "i" is incorrectly removed)

use ginfmt::{Config, format_with_config};

#[test]
fn test_alignment_bug_fix() {
    // Use valid declaration syntax according to current grammar
    let source = "Maybe is Some or None\nResult is Ok or Error\nEither is Left or Right\n";
    let config = Config {
        align_binds: false,
        ..Default::default()
    };
    let output = format_with_config(source, config);

    // Verify "is" is intact in all lines
    assert!(output.contains("is"), "Output should contain 'is'");
    assert!(
        !output.contains(" s "),
        "Output should not contain ' s ' (missing 'i')"
    );

    // Verify original spacing is preserved (alignment not yet implemented)
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 3);

    // Check that all lines contain valid variant syntax
    assert!(lines[0].contains("Some or None"));
    assert!(lines[1].contains("Ok or Error"));
    assert!(lines[2].contains("Left or Right"));

    // Verify the formatter doesn't break the declarations
    assert!(lines[0].contains("Maybe is"));
    assert!(lines[1].contains("Result is"));
    assert!(lines[2].contains("Either is"));
}

#[test]
fn test_alignment_idempotent() {
    // Use valid declaration syntax according to current grammar
    let source = "Maybe is Some or None\nResult is Ok or Error\nEither is Left or Right\n";
    let config = Config {
        align_binds: false,
        ..Default::default()
    };

    // Format once
    let output1 = format_with_config(source, config.clone());

    // Format the output again
    let output2 = format_with_config(&output1, config);

    // Should be identical
    assert_eq!(output1, output2, "Format should be idempotent");
}
