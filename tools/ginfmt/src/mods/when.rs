//! Formatting for when_expression nodes.
//!
//! Aligns `then` and `else` to the same column as `when`:
//!
//! ```gin
//! Maybe.is_empty: when self is Some(x)
//!                 then True
//!                 else False
//! ```

use crate::visitor::FmtVisitor;

/// Rewrite a `when_expression` node, aligning `then` and `else` to `when_col`.
///
/// `when_col` is the column index (0-based) where `when` starts in the output
/// buffer — callers compute this as the current line length before emitting the node.
pub fn rewrite(visitor: &FmtVisitor, node: tree_sitter::Node, when_col: usize) -> Option<String> {
    let mut cursor = node.walk();

    let mut subject: Option<String> = None;
    let mut pattern: Option<String> = None;
    let mut then_body: Option<String> = None;
    let mut else_body: Option<String> = None;

    let mut seen_when = false;
    let mut seen_is = false;
    let mut seen_then = false;
    let mut seen_else = false;

    for child in node.children(&mut cursor) {
        match child.kind() {
            "when" => seen_when = true,
            "is" => seen_is = true,
            "then" => seen_then = true,
            "else" => seen_else = true,
            _ if child.is_named() => {
                let text = visitor.text_for_node(child)?;
                if seen_when && !seen_is && subject.is_none() {
                    subject = Some(text);
                } else if seen_is && !seen_then && pattern.is_none() {
                    pattern = Some(text);
                } else if seen_then && !seen_else && then_body.is_none() {
                    then_body = Some(text);
                } else if seen_else && else_body.is_none() {
                    else_body = Some(text);
                }
            }
            _ => {}
        }
    }

    let subject = subject?;
    let pattern = pattern?;
    let then_body = then_body?;
    let else_body = else_body?;

    let indent = " ".repeat(when_col);
    Some(format!(
        "when {subject} is {pattern}\n{indent}then {then_body}\n{indent}else {else_body}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn parse_when(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_gin::language()).unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    #[ignore]
    fn debug_print_when_ast() {
        let source = "Maybe:\n  is_empty: when self is Some then True else False\nreturn\n";
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_gin::language()).unwrap();
        let tree = parser.parse(source, None).unwrap();

        fn print_tree(node: tree_sitter::Node, indent: &str) {
            println!(
                "{}{} ({:?})",
                indent,
                node.kind(),
                (node.start_byte(), node.end_byte())
            );
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                print_tree(child, &format!("{}  ", indent));
            }
        }

        print_tree(tree.root_node(), "");
    }

    // TODO: fix this test
    // #[test]
    // fn test_rewrite_inline() {
    //     // Method with inline when - using valid impl_block syntax
    //     let source = "Maybe:\n  is_empty: when self is Some then True else False\nreturn\n";
    //     let tree = parse_when(source);
    //     let root = tree.root_node();

    //     // Find the when_expression node within the impl_block
    //     let impl_block = root.child(0).unwrap();
    //     assert_eq!(
    //         impl_block.kind(),
    //         "impl_block",
    //         "expected impl_block, got {}",
    //         impl_block.kind()
    //     );

    //     let mut cursor = impl_block.walk();
    //     let method = impl_block
    //         .children(&mut cursor)
    //         .find(|c| c.kind() == "method_statement")
    //         .expect("no method_statement in impl_block");
    //     let mut cursor2 = method.walk();
    //     let when_node = method
    //         .children(&mut cursor2)
    //         .find(|c| c.kind() == "when_expression")
    //         .expect("no when_expression in method_statement");

    //     let visitor = FmtVisitor::new(source, Config::default());
    //     // when_col = len("  ") = 2 (indent level)
    //     let result = rewrite(&visitor, when_node, 2).unwrap();
    //     assert_eq!(result, "when self is Some\n  then True\n  else False");
    // }
}
