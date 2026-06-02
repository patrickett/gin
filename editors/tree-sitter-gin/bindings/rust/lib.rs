//! This crate provides gin language support for the [tree-sitter][] parsing library.
//!
//! Typically, you will use the [language][language func] function to add this language to a
//! tree-sitter [Parser][], and then use the parser to parse some code:
//!
//! ```
//! let code = "";
//! let mut parser = tree_sitter::Parser::new();
//! parser.set_language(tree_sitter_gin::language()).expect("Error loading gin grammar");
//! let tree = parser.parse(code, None).unwrap();
//! ```
//!
//! [Language]: https://docs.rs/tree-sitter/*/tree_sitter/struct.Language.html
//! [language func]: fn.language.html
//! [Parser]: https://docs.rs/tree-sitter/*/tree_sitter/struct.Parser.html
//! [tree-sitter]: https://tree-sitter.github.io/

use tree_sitter::Language;

unsafe extern "C" {
    fn tree_sitter_gin() -> Language;
}

/// Get the tree-sitter [Language][] for this grammar.
///
/// [Language]: https://docs.rs/tree-sitter/*/tree_sitter/struct.Language.html
pub fn language() -> Language {
    unsafe { tree_sitter_gin() }
}

/// The content of the [`node-types.json`][] file for this grammar.
///
/// [`node-types.json`]: https://tree-sitter.github.io/tree-sitter/using-parsers#static-node-types
pub const NODE_TYPES: &str = include_str!("../../src/node-types.json");

// Uncomment these to include any queries that this grammar contains

pub const HIGHLIGHTS_QUERY: &str = include_str!("../../queries/highlights.scm");
// pub const INJECTIONS_QUERY: &'static str = include_str!("../../queries/injections.scm");
// pub const LOCALS_QUERY: &'static str = include_str!("../../queries/locals.scm");
// pub const TAGS_QUERY: &'static str = include_str!("../../queries/tags.scm");

#[cfg(test)]
mod tests {
    use tree_sitter::{Parser, Query, QueryCursor};

    const REFLECT_TYPE_DECL: &str = r#"Type is Primitive(width BigInt, signed Bool)
     or Record(name String, fields List(NamedTy))
     or Union(name String, variants List(VariantShape))
     or Tuple(elems List(Type))
     or Ptr(inner Type)
     or Ref(inner Type, mutable Bool)
     or Array(elem Type, size BigInt)
     or Opaque(name String)
"#;

    fn parser() -> Parser {
        let mut parser = Parser::new();
        parser
            .set_language(super::language())
            .expect("Error loading gin language");
        parser
    }

    #[test]
    fn test_can_load_grammar() {
        let _ = parser();
    }

    #[test]
    fn constructor_union_variants_parse_without_errors() {
        let mut parser = parser();
        let tree = parser
            .parse(REFLECT_TYPE_DECL, None)
            .expect("tree-sitter should parse source");

        assert!(
            !tree.root_node().has_error(),
            "constructor-style union variants should parse without errors:\n{}",
            tree.root_node().to_sexp()
        );
    }

    #[test]
    fn js_style_list_rest_patterns_parse_without_errors() {
        let source = r#"check(xs List(Int)) Bool := when xs is
    [] then True
    [head, ...tail] then False
"#;
        let mut parser = parser();
        let tree = parser
            .parse(source, None)
            .expect("tree-sitter should parse source");

        assert!(
            !tree.root_node().has_error(),
            "JS-style list rest patterns should parse without errors:\n{}",
            tree.root_node().to_sexp()
        );
    }

    #[test]
    fn every_or_in_constructor_union_highlights_as_keyword_operator() {
        let mut parser = parser();
        let tree = parser
            .parse(REFLECT_TYPE_DECL, None)
            .expect("tree-sitter should parse source");
        assert!(
            !tree.root_node().has_error(),
            "highlight regression input must parse cleanly:\n{}",
            tree.root_node().to_sexp()
        );

        let query = Query::new(super::language(), super::HIGHLIGHTS_QUERY)
            .expect("highlights query should compile");
        let keyword_operator_index = query
            .capture_names()
            .iter()
            .position(|name| name == "keyword.operator")
            .expect("highlights query should define @keyword.operator")
            as u32;

        let mut cursor = QueryCursor::new();
        let captures = cursor.captures(&query, tree.root_node(), REFLECT_TYPE_DECL.as_bytes());
        let highlighted_ors = captures
            .filter(|(query_match, capture_index)| {
                let capture = query_match.captures[*capture_index];
                capture.index == keyword_operator_index
                    && capture.node.utf8_text(REFLECT_TYPE_DECL.as_bytes()) == Ok("or")
            })
            .count();

        assert_eq!(
            highlighted_ors, 7,
            "all seven union `or` separators should be @keyword.operator"
        );
    }
}
