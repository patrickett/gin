// Example to verify the alignment fix
fn main() {
    let source = "Area is 0...999
Group is 0...99
Serial is 0...9999";

    let config = ginfmt::Config {
        align_declarations: true,
        ..Default::default()
    };
    let output = ginfmt::format_with_config(source, config);

    println!("Input:");
    println!("{}", source);
    println!("\nOutput:");
    println!("{}", output);

    // Verify the fix
    assert!(output.contains("is"), "Output should contain 'is'");
    assert!(!output.contains(" s "), "Output should not contain ' s ' (missing 'i')");

    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines[0], "Area   is 0...999");
    assert_eq!(lines[1], "Group  is 0...99");
    assert_eq!(lines[2], "Serial is 0...9999");

    println!("\n✓ Alignment fix verified!");
}
