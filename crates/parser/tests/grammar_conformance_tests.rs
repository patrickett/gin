use parser::query::SourceParseExt;

mod support;
use support::*;

#[test]
fn parse_local_and_package_import_forms() {
    let out = "use 'core'.(Int, Byte)\nuse self.(Bool)\nuse Bool\n".parse_source_full();
    assert!(count_errors(&out) <= 1);
    assert_eq!(out.ast.uses.len(), 3);
}

#[test]
fn for_loop_requires_loop_keyword() {
    let without_loop = "sum: for i in items\n    i\n".parse_source_full();
    assert!(
        count_errors(&without_loop) > 0,
        "`for` loop without trailing `loop` must be rejected"
    );

    let with_loop = "sum: for i in items\n    i\nloop\n".parse_source_full();
    assert_eq!(count_errors(&with_loop), 0);
    assert!(has_def(&with_loop, "sum"));
}

#[test]
fn while_loop_requires_loop_keyword() {
    let without_loop = "run: while x is True\n    0\n".parse_source_full();
    assert!(count_errors(&without_loop) > 0);

    let with_loop = "run: while x is True\n    0\nloop\n".parse_source_full();
    assert_eq!(count_errors(&with_loop), 0);
    assert!(has_def(&with_loop, "run"));
}

#[test]
fn if_requires_trailing_return_clause() {
    let missing = "is_true: if x is True\n    1\n".parse_source_full();
    assert!(count_errors(&missing) > 0);

    let with_return = "is_true: if x is True\n    1\nreturn\n".parse_source_full();
    assert_eq!(count_errors(&with_return), 0);
}

#[test]
fn when_subject_is_patterns_parse() {
    let out = "\
what: when x is\n    1 then 'yes'\n    else 'no'\n";
    let out = out.parse_source_full();
    assert_eq!(count_errors(&out), 0);

    let out2 = "\
x: when x is\n    1 then 2\n    2 then 3\n";
    let out2 = out2.parse_source_full();
    assert_eq!(count_errors(&out2), 0);
}

#[test]
fn unsupported_keywords_are_identifiers() {
    let out = "x: try\ny: catch\n".parse_source_full();
    assert_eq!(count_errors(&out), 0);
    assert!(has_def(&out, "x"));
    assert!(has_def(&out, "y"));
}

#[test]
fn unsupported_not_equal_token_is_rejected_as_infix() {
    let out = "x: a /= b\n".parse_source_full();
    assert!(
        count_errors(&out) > 0,
        "lexer token /= is tokenized but parser should reject as infix comparison"
    );

    let eqeq = "x: a == b\n".parse_source_full();
    assert!(count_errors(&eqeq) > 0);
}

#[test]
fn newline_and_comma_separator_rules_are_accepted() {
    let list_and_tuple = "\
pair : (\n    1\n    2\n)\nxs : [\n    1\n    2\n]\n";
    let list_and_tuple = list_and_tuple.parse_source_full();
    assert_eq!(count_errors(&list_and_tuple), 0);
}

#[test]
fn parse_record_and_format_interpolation() {
    let out = "msg: \"x={(n)} and y={f}\"\nmsg2: (a: 1, b: 2)\n".parse_source_full();
    assert_eq!(count_errors(&out), 0);
}
