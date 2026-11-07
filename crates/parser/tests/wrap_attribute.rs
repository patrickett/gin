//! `#wrap` is a marker item on its own line, before the named domain it marks.
use ast::{Item, TypeBody};

const WORD: &str = "#wrap\nWord is in 0 to 18446744073709551615\n";

#[test]
fn wrap_is_a_marker_item_before_the_domain() {
    let file = parser::parse_strict(WORD).unwrap();
    assert!(matches!(file.items[0].value, Item::Wrap));
    let Item::Type(declaration) = &file.items[1].value else {
        panic!("the marked type follows");
    };
    assert!(matches!(declaration.body.value, TypeBody::Domain(_)));
    assert_eq!(
        file.span_table.get(file.items[0].span_id).extract(WORD),
        "#wrap"
    );
}

#[test]
fn wrap_survives_blank_lines_and_comments_before_its_domain() {
    let source = "#wrap\n\n-- the register width\nWord is in 0 to 255\n";
    let file = parser::parse_strict(source).unwrap();
    assert!(matches!(file.items[0].value, Item::Wrap));
    assert!(matches!(file.items[1].value, Item::Type(_)));
}

#[test]
fn the_parser_does_not_judge_the_target() {
    // Whether a `#wrap` marks something it can mark is the checker's flaw to report.
    let source = "#wrap\nPair has first in 0 to 1\n";
    let file = parser::parse_strict(source).unwrap();
    assert!(matches!(file.items[0].value, Item::Wrap));
}
