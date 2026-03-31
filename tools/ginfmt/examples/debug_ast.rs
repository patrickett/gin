// Debug AST structure for multiline sum type
fn main() {
    let source = "Maybe(thing) is
    Some(thing)
    or
    None
";

    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_gin::language()).unwrap();
    let tree = parser.parse(source, None).unwrap();

    println!("Root node: {}", tree.root_node().kind());
    print_tree(tree.root_node(), source, 0);
}

fn print_tree(node: tree_sitter::Node, source: &str, indent: usize) {
    let indent_str = "  ".repeat(indent);
    let text = &source[node.start_byte()..node.end_byte()];
    let text_preview = if text.len() > 40 {
        format!("{}...", &text[..40].replace('\n', "\\n"))
    } else {
        text.replace('\n', "\\n")
    };

    println!("{}[{}:{}] {} = {:?}", indent_str, node.start_byte(), node.end_byte(), node.kind(), text_preview);

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        print_tree(child, source, indent + 1);
    }
}
