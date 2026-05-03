#[test]
fn test_inline_dash_formatting() {
    use ginfmt::Config;
    use ginfmt::format_with_config;

    let source = "Maybe[x] is  --- Used to represent values that may or may not be present.
    Some(x) or --- has a value
    None  --- has no value
";

    let output = format_with_config(source, Config::default());
    println!("=== Original ===");
    println!("{}", source);
    println!("\n=== Formatted ===");
    println!("{}", output);
}
