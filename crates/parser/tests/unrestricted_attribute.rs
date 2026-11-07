//! `#unrestricted` is a marker item on its own line, before the type it marks.
use ast::{Item, TypeBody};

const SMALL: &str = "#unrestricted\nSmall is in 0 to 255\n";

#[test]
fn unrestricted_is_a_marker_item_before_the_type() {
    let file = parser::parse_strict(SMALL).unwrap();
    assert!(matches!(file.items[0].value, Item::Unrestricted));
    let Item::Type(declaration) = &file.items[1].value else {
        panic!("the marked type follows");
    };
    assert!(matches!(declaration.body.value, TypeBody::Domain(_)));
    assert_eq!(
        file.span_table.get(file.items[0].span_id).extract(SMALL),
        "#unrestricted"
    );
}

#[test]
fn unrestricted_composes_with_wrap_in_either_order() {
    for source in [
        "#wrap\n#unrestricted\nWord is in 0 to 255\n",
        "#unrestricted\n#wrap\nWord is in 0 to 255\n",
    ] {
        let file = parser::parse_strict(source).unwrap();
        assert!(matches!(
            file.items[0].value,
            Item::Wrap | Item::Unrestricted
        ));
        assert!(matches!(
            file.items[1].value,
            Item::Wrap | Item::Unrestricted
        ));
        assert!(matches!(file.items[2].value, Item::Type(_)));
    }
}

#[test]
fn the_parser_does_not_judge_the_target() {
    // Whether the marked type may be unrestricted is the checker's flaw to report.
    let source = "#unrestricted\nPair has first in 0 to 1\n";
    let file = parser::parse_strict(source).unwrap();
    assert!(matches!(file.items[0].value, Item::Unrestricted));
}

#[test]
fn unrestricted_must_end_its_line() {
    assert!(parser::parse_strict("#unrestricted Small is in 0 to 255\n").is_err());
}

#[test]
fn commas_separate_directives_like_newlines() {
    let source = "#unrestricted, #wrap\nSmall is in 0 to 255\n";
    let file = parser::parse_strict(source).unwrap();
    assert!(matches!(file.items[0].value, Item::Unrestricted));
    assert!(matches!(file.items[1].value, Item::Wrap));
    assert!(matches!(file.items[2].value, Item::Type(_)));
    assert_eq!(
        file.span_table.get(file.items[0].span_id).extract(source),
        "#unrestricted"
    );
    assert_eq!(
        file.span_table.get(file.items[1].span_id).extract(source),
        "#wrap"
    );
}

#[test]
fn dangling_or_mixed_commas_are_rejected() {
    for source in [
        "#unrestricted,\nSmall is in 0 to 255\n",
        "#unrestricted, Small is in 0 to 255\n",
        "#unrestricted,, #wrap\nSmall is in 0 to 255\n",
    ] {
        assert!(parser::parse_strict(source).is_err(), "{source:?}");
    }
}
