use tree_sitter::Parser;

fn main() {
    let source = "Maybe(x) is  --- Used to represent values that may or may not be present.
    Some(x) or --- has a value
    None  --- has no value
";

    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_gin::language()).unwrap();
    let tree = parser.parse(source, None).unwrap();

    println!("=== Tree-sitter Parse Tree for inline dashes ===");
    println!("Source:\n{}", source);
    println!("\nParse tree:");
    print_tree(tree.root_node(), source, 0);
}

fn print_tree(node: tree_sitter::Node, source: &str, indent: usize) {
    let indent_str = "  ".repeat(indent);
    let text = &source[node.start_byte()..node.end_byte()];
    let text_preview = if text.len() > 50 {
        format!("{}...", &text[..50].replace('\n', "\\n"))
    } else {
        text.replace('\n', "\\n")
    };

    println!(
        "{}[{}:{}] {:15} = {:?}",
        indent_str,
        node.start_byte(),
        node.end_byte(),
        node.kind(),
        text_preview
    );

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        print_tree(child, source, indent + 1);
    }
}
