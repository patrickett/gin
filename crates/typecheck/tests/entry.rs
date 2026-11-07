//! `#entry` marks the program's start function and its contract is checked.
use typecheck::{Limits, SourceCheckError, check_file, check_module_files};

fn check(text: &str) -> Result<typecheck::CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict entry fixture");
    check_file(&file, Limits::default())
}

#[test]
fn the_marked_function_is_the_entry() {
    let checked = check("#entry\nmain() in 0 to 255: return 0\n").unwrap();
    assert_eq!(checked.entry, Some(0));
    assert_eq!(checked.functions[0].name, "main");
}

#[test]
fn the_entry_index_follows_declaration_order_not_marker_order() {
    let checked = check("five() in 0 to 16: return 5\n#entry\nstart() in 0 to 16: return five()\n");
    assert_eq!(checked.unwrap().entry, Some(1));
}

#[test]
fn a_unit_entry_is_allowed_and_no_marker_means_no_entry() {
    assert_eq!(
        check("#entry\nmain():\n    return\n").unwrap().entry,
        Some(0)
    );
    assert_eq!(check("main() in 0 to 255: return 0\n").unwrap().entry, None);
}

#[test]
fn an_entry_with_parameters_or_a_wide_result_is_a_type_error() {
    for text in [
        "#entry\nmain(x in 0 to 255) in 0 to 255: return x\n",
        "#entry\nmain() in 0 to 256: return 0\n",
        "#entry\nmain() in 0 to 1000: return 0\n",
    ] {
        assert_eq!(check(text).unwrap_err(), SourceCheckError::Type, "{text:?}");
    }
}

#[test]
fn a_marker_must_pair_with_exactly_one_function() {
    let cases = [
        (
            "#entry\n#entry\nmain() in 0 to 255: return 0\n",
            SourceCheckError::Duplicate,
        ),
        (
            "#entry\na() in 0 to 255: return 0\n#entry\nb() in 0 to 255: return 0\n",
            SourceCheckError::Duplicate,
        ),
        (
            "a() in 0 to 255: return 0\n#entry\n",
            SourceCheckError::Malformed,
        ),
        (
            "#entry\nuse lib thing\na() in 0 to 255: return 0\n",
            SourceCheckError::Malformed,
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(check(text).unwrap_err(), expected, "{text:?}");
    }
}

#[test]
fn a_marker_cannot_reach_across_files() {
    let files = [
        parser::parse_strict_file("#entry\n", 0).unwrap(),
        parser::parse_strict_file("main() in 0 to 255: return 0\n", 1).unwrap(),
    ];
    let failure = check_module_files("app", &files, &[], Limits::default());
    assert_eq!(failure.unwrap_err(), SourceCheckError::Malformed);
}

#[test]
fn the_entry_in_a_later_file_indexes_the_whole_module() {
    let files = [
        parser::parse_strict_file("five() in 0 to 16: return 5\n", 0).unwrap(),
        parser::parse_strict_file("#entry\nmain() in 0 to 16: return five()\n", 1).unwrap(),
    ];
    let module = check_module_files("app", &files, &[], Limits::default()).unwrap();
    assert_eq!(module.checked().entry, Some(1));
}
