//! Test idempotency with more complex cases
use ginfmt::{Config, format_with_config};

#[test]
fn test_idempotency_with_different_spacing() {
    // Test with already-aligned input
    let source = "Area   is 0...999\nGroup  is 0...99\nSerial is 0...9999\n";
    let config = Config {
        align_binds: false,
        ..Default::default()
    };

    let first = format_with_config(source, config.clone());
    println!("Input:\n{}", source);
    println!("First format:\n{}", first);

    let second = format_with_config(&first, config.clone());
    println!("Second format:\n{}", second);

    assert_eq!(first, second, "Formatter should be idempotent");
}

#[test]
fn test_idempotency_after_alignment() {
    // Test that formatting already-formatted code doesn't change it
    let source = "Area is 0...999\nGroup is 0...99\nSerial is 0...9999\n";
    let config = Config {
        align_declarations: true,
        align_binds: false,
        ..Default::default()
    };

    let first = format_with_config(source, config.clone());
    let second = format_with_config(&first, config.clone());
    let third = format_with_config(&second, config);

    assert_eq!(first, second, "First and second should match");
    assert_eq!(second, third, "Second and third should match");
}
