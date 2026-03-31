// Test the original user example
fn main() {
    let source = "Area is 0...999
Group is 0...99
Serial is 0...9999

Maybe(thing) is --- Used to represent values that may or may not be present.
    Some(thing) or --- has a value
    None --- has no value


do_something: 'hello'";

    let config = ginfmt::Config {
        align_declarations: true,
        align_binds: false,
        ..Default::default()
    };

    println!("Original source:");
    println!("{}", source);
    println!("\n========================================\n");

    let output1 = ginfmt::format_with_config(source, config.clone());
    println!("Format 1:");
    println!("{}", output1);
    println!("\n========================================\n");

    let output2 = ginfmt::format_with_config(&output1, config.clone());
    println!("Format 2:");
    println!("{}", output2);
    println!("\n========================================\n");

    // Check idempotency
    if output1 == output2 {
        println!("✓ IDEMPOTENT: Format is stable");
    } else {
        println!("✗ NOT IDEMPOTENT: Format changed on second run");
    }

    // Check blank line preservation
    if output1.contains("\n\n") {
        println!("✓ BLANK LINES: Blank lines are preserved");
    } else {
        println!("✗ BLANK LINES: Blank lines were lost");
    }

    // Check multi-line preservation
    if output1.contains("Some(thing)") && output1.contains("None") {
        println!("✓ MULTI-LINE: Multi-line declarations preserved");
    } else {
        println!("✗ MULTI-LINE: Multi-line declarations were corrupted");
    }
}
