use ast::FileAst;
use parser::query::SourceParseExt;
use std::collections::HashMap;
use typecheck::completions::{
    CursorContext, FileAstCompletionExt, detect_cursor_context, expected_type_name_at,
    literal_union_completions_at,
};

#[test]
fn completion_requests_reject_invalid_byte_offsets() {
    let ast = FileAst::default();
    let source = "é";
    for offset in [1, source.len() + 1, usize::MAX] {
        assert!(ast.completions_for_context(source, offset, None).is_empty());
        assert!(
            ast.completions_for_context(source, offset, Some(&HashMap::new()))
                .is_empty()
        );
    }
}

#[test]
fn cursor_context_rejects_invalid_byte_offsets() {
    let ast = FileAst::default();
    let source = "use é";
    for offset in [5, source.len() + 1, usize::MAX] {
        assert_eq!(
            detect_cursor_context(&ast, source, offset),
            CursorContext::Root
        );
    }
}

#[test]
fn expected_type_queries_reject_invalid_byte_offsets() {
    let source = "value Color: 'é'";
    let inside_character = source.find('é').unwrap() + 1;
    for offset in [inside_character, source.len() + 1, usize::MAX] {
        assert_eq!(expected_type_name_at(source, offset), None);
        assert!(literal_union_completions_at(source, offset, &HashMap::new()).is_empty());
    }
}

#[test]
fn valid_utf8_boundaries_and_end_of_source_remain_supported() {
    let ast = FileAst::default();
    let source = "use é";
    for offset in [0, 4, source.len()] {
        assert_eq!(
            detect_cursor_context(&ast, source, offset),
            CursorContext::Use
        );
        assert!(ast.completions_for_context(source, offset, None).is_empty());
    }
    let source = "é\nvalue Color: 'é'";
    assert_eq!(
        expected_type_name_at(source, source.len())
            .unwrap()
            .as_str(),
        "Color"
    );
    let queries = ast.completions_for_context("#", 1, None);
    assert!(!queries.is_empty());
    assert!(!ast.completions_for_context("", 0, None).is_empty());
}

#[test]
fn function_body_completion_uses_when_instead_of_removed_if_keyword() {
    let source = "route(priority):\n    return priority\n";
    let parsed = source.parse_source_full();
    let position = source.find("return").unwrap();
    let completions = parsed.ast.completions_for_context(source, position, None);
    assert!(completions.iter().all(|entry| entry.label != "if"));
    let conditionals: Vec<_> = completions
        .iter()
        .filter(|entry| entry.label == "when")
        .collect();
    assert_eq!(conditionals.len(), 1, "{completions:?}");
    assert!(conditionals[0].is_snippet);
    assert!(
        conditionals[0]
            .insert_text
            .as_ref()
            .is_some_and(|text| text.starts_with("when "))
    );
}
