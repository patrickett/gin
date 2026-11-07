//! `#entry` is the one recognised top-level directive; every other `#name` is rejected.
use ast::Item;

const PROGRAM: &str = "#entry\nmain() in 0 to 255: return 0\n";

#[test]
fn entry_is_a_marker_item_before_the_binding() {
    let file = parser::parse_strict(PROGRAM).unwrap();
    assert!(matches!(file.items[0].value, Item::Entry));
    assert!(matches!(file.items[1].value, Item::Binding(_)));
    assert_eq!(
        file.span_table.get(file.items[0].span_id).extract(PROGRAM),
        "#entry"
    );
}

#[test]
fn entry_survives_blank_lines_and_comments_before_its_binding() {
    let source = "#entry\n\n-- the program\nmain() in 0 to 255: return 0\n";
    let file = parser::parse_strict(source).unwrap();
    assert!(matches!(file.items[0].value, Item::Entry));
    assert!(matches!(file.items[1].value, Item::Binding(_)));
}

#[test]
fn unknown_directives_are_unknown_compiler_queries() {
    for name in ["main", "inline", "Entry", "entryx", "size"] {
        let source = format!("#{name}\nmain() in 0 to 255: return 0\n");
        let diagnostics = parser::parse_strict(&source).unwrap_err();
        let text = format!("{diagnostics:?}");
        assert!(text.contains("UnknownCompilerQuery"), "{name}: {text}");
    }
}

#[test]
fn recovery_keeps_the_following_binding_after_an_unknown_directive() {
    let output = parser::parse("#inline\nmain() in 0 to 255: return 0\n").unwrap();
    assert!(
        output
            .file
            .items
            .iter()
            .any(|item| item.value == Item::Error)
    );
    assert!(
        output
            .file
            .items
            .iter()
            .any(|item| matches!(item.value, Item::Binding(_)))
    );
}

#[test]
fn a_bare_hash_and_trailing_tokens_are_rejected() {
    for source in [
        "#\nmain() in 0 to 255: return 0\n",
        "#entry main\n",
        "#entry 3\n",
    ] {
        assert!(parser::parse_strict(source).is_err(), "{source:?}");
    }
}

#[test]
fn the_todo_hole_is_still_an_expression() {
    let file = parser::parse_strict("main() in 0 to 255: return #todo\n").unwrap();
    assert!(matches!(file.items[0].value, Item::Binding(_)));
}

#[test]
fn load_and_store_directives_mark_the_line_that_follows() {
    let file = parser::parse_strict("#load\nload(element Type, pointer ref @element, index in 0 to 3) element\n").unwrap();
    assert!(matches!(file.items[0].value, Item::Load));
    let file = parser::parse_strict("#store\nstore(element Type, pointer ref @element, index in 0 to 3, value element) ()\n").unwrap();
    assert!(matches!(file.items[0].value, Item::Store));
    // Any other directive is still an unknown query.
    assert!(parser::parse_strict("#fetch\nf(): return\n").is_err());
}
