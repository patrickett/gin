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
        parser
            .set_language(&tree_sitter_gin::LANGUAGE_FN.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_rewrite_inline() {
        // Method with inline when — tree-sitter now parses method_statement
        let source = "Maybe.is_empty: when self is Some(x) then True else False\n";
        let tree = parse_when(source);
        let root = tree.root_node();

        // Find the when_expression node
        let method = root.child(0).unwrap();
        assert_eq!(method.kind(), "method_statement", "expected method_statement, got {}", method.kind());

        let mut cursor = method.walk();
        let stmt_node = method
            .children(&mut cursor)
            .find(|c| c.kind() == "statement")
            .expect("no statement in method_statement");
        let mut cursor2 = stmt_node.walk();
        let when_node = stmt_node
            .children(&mut cursor2)
            .find(|c| c.kind() == "when_expression")
            .expect("no when_expression in statement");

        let visitor = FmtVisitor::new(source, Config::default());
        // when_col = len("Maybe.is_empty: ") = 16
        let result = rewrite(&visitor, when_node, 16).unwrap();
        assert_eq!(
            result,
            "when self is Some(x)\n                then True\n                else False"
        );
    }
}
