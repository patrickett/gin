use ginfmt::Config;

mod common;

use common::assert_formatted_allowing_warnings;

#[test]
fn preserves_one_optional_blank_line_between_type_declarations() {
    for gap in ["\n", "\n\n", "\n\n\n\n", "\n   \n"] {
        let source = format!("First is in 0 to 10{gap}Second is in 0 to 20\n");
        let gap = if gap == "\n" { "\n" } else { "\n\n" };
        assert_formatted_allowing_warnings(
            &source,
            &format!("First is in 0 to 10{gap}Second is in 0 to 20\n"),
            Config::default(),
        );
    }
}

#[test]
fn preserves_blank_lines_around_top_level_comments() {
    for source in [
        "First is in 0 to 10\n\n--- Second type.\nSecond is in 0 to 20\n",
        "First is in 0 to 10\n\n-- Group two.\nSecond is in 0 to 20\n",
        "First is in 0 to 10\n-- Group two.\n\nSecond is in 0 to 20\n",
        "First is in 0 to 10\n\n#inline\nSecond is in 0 to 20\n",
    ] {
        assert_formatted_allowing_warnings(source, source, Config::default());
    }
}

#[test]
fn doc_comment_gaps_do_not_become_item_separators() {
    for gap in ["\n", "\n\n", "\n\n\n\n", "\n   \n"] {
        let separator = if gap == "\n" { "\n" } else { "\n\n" };
        for (doc, expected_doc) in [
            ("--- Second type.\n\n", "--- Second type.\n"),
            (
                "--- Second type.\n\n--- Another paragraph.\n",
                "--- Second type.\n--- Another paragraph.\n",
            ),
            (
                "--- Second type.\n---\n--- Another paragraph.\n",
                "--- Second type.\n---\n--- Another paragraph.\n",
            ),
        ] {
            let source = format!("First is in 0 to 10{gap}{doc}Second is in 0 to 20\n");
            assert_formatted_allowing_warnings(
                &source,
                &format!("First is in 0 to 10{separator}{expected_doc}Second is in 0 to 20\n"),
                Config::default(),
            );
        }
    }
}

#[test]
fn preserves_blank_lines_after_type_method_bodies() {
    let source = "Box(element) has\n    value element\n    take(self) element: return self.value\n\n--- Another type.\nOther is Empty or Full\n";
    assert_formatted_allowing_warnings(source, source, Config::default());
}

#[test]
fn keeps_documented_items_adjacent_after_multiline_type_bodies() {
    let source = "Box(element) has\n    value element\n    take(self) element: return self.value\n--- Another type.\n---\n--- With two alternatives.\nOther is Empty or Full\n";
    assert_formatted_allowing_warnings(source, source, Config::default());
}

#[test]
fn preserves_spacing_before_combined_prefix_and_suffix_docs() {
    assert_formatted_allowing_warnings(
        "First is in 0 to 10\n--- Prefix.\nSecond is --- Suffix.\n    Empty or Full\n",
        "First is in 0 to 10\n--- Prefix.\n---\n--- Suffix.\nSecond is\n    Empty\n    or Full\n",
        Config::default(),
    );
}

#[test]
fn keeps_bind_docs_attached_when_collapsing_blank_lines() {
    assert_formatted_allowing_warnings(
        "First is in 0 to 10\n\n\n--- A value.\n\nvalue: 1\n",
        "First is in 0 to 10\n\n--- A value.\nvalue: 1\n",
        Config::default(),
    );
}

#[test]
fn preserves_blank_lines_between_import_groups() {
    let source = "use Alpha\n\nuse Beta\n";
    assert_formatted_allowing_warnings(source, source, Config::default());
}

#[test]
fn preserves_blank_lines_between_top_level_expressions() {
    assert_formatted_allowing_warnings("1\n\n2\n", "1\n\n2\n", Config::default());
}

#[test]
fn does_not_add_blank_lines_for_multiline_type_bodies() {
    for source in [
        "Box(element) has\n    value element\nOther is Empty or Full\n",
        "Box(element) has\n    value element\n\n    take(self) element: return self.value\nOther is Empty or Full\n",
    ] {
        assert_formatted_allowing_warnings(source, source, Config::default());
    }
}

#[test]
fn normalizes_existing_bind_spacing_and_file_edges() {
    assert_formatted_allowing_warnings(
        "\n\nfirst: 1\n\n\nsecond: 2\n\n",
        "first: 1\n\nsecond: 2\n",
        Config::default(),
    );
}
