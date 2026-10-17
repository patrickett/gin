use parser::query::SourceParseExt;

mod support;
use support::*;

#[test]
fn ranges_use_to_across_surface_forms() {
    for source in [
        "Byte is in 0 to 255\n",
        "Window is -10 to 10\n",
        "Offset(unit) is in min_offset to max_count\n",
        "read(value in 0 to max_count): return value\n",
        "values := low to max\n",
        "check(value) True thus value in low to max: return True\n",
    ] {
        let out = source.parse_source_full();
        assert_eq!(count_errors(&out), 0, "{source}: {:?}", out.symptoms);
        let legacy = source.replace(" to ", "...").parse_source_full();
        assert!(
            count_errors(&legacy) > 0,
            "accepted obsolete range: {source}"
        );
    }
}

#[test]
fn parse_local_and_package_import_forms() {
    let out = "use 'core' Int, Byte\nuse self Bool\nuse Bool\n".parse_source_full();
    assert!(count_errors(&out) <= 1);
    assert_eq!(out.ast.uses.len(), 3);
}

#[test]
fn pointer_types_are_valid_parameter_annotations() {
    let out = "Byte is in 0 to 255\nwrite(buf @Byte) SignedBigInt: return 0\n".parse_source_full();
    assert_eq!(count_errors(&out), 0, "symptoms: {:?}", out.symptoms);
    assert!(has_def(&out, "write"));
}

#[test]
fn pointer_bindings_accept_explicit_pointer_types() {
    let out = "Byte is in 0 to 255\nvalue Byte := 1\npointer @Byte := @value\n".parse_source_full();
    assert_eq!(count_errors(&out), 0, "symptoms: {:?}", out.symptoms);
    assert!(has_def(&out, "pointer"));
}

#[test]
fn bracket_literals_parse_as_neutral_sequence_nodes() {
    let out = "empty := []\nelements := [1, 2, 3, 4]\nrepeated := [0; 4]\n".parse_source_full();
    assert_eq!(count_errors(&out), 0, "symptoms: {:?}", out.symptoms);
    assert!(matches!(
        &out.ast.defs[&intern("empty")].value,
        ast::BindValue::Expr(expr)
            if matches!(
                &expr.value,
                ast::Expr::SequenceLiteral(ast::SequenceLiteral::Elements(items)) if items.is_empty()
            )
    ));
    assert!(matches!(
        &out.ast.defs[&intern("elements")].value,
        ast::BindValue::Expr(expr)
            if matches!(
                &expr.value,
                ast::Expr::SequenceLiteral(ast::SequenceLiteral::Elements(items)) if items.len() == 4
            )
    ));
    assert!(matches!(
        &out.ast.defs[&intern("repeated")].value,
        ast::BindValue::Expr(expr)
            if matches!(&expr.value, ast::Expr::SequenceLiteral(ast::SequenceLiteral::Repeat { .. }))
    ));
}

#[test]
fn for_loop_ends_on_dedent() {
    let source = "sum: for i in items\n    i\n".parse_source_full();
    assert_eq!(count_errors(&source), 0, "{:?}", source.symptoms);
    assert!(has_def(&source, "sum"));
}

#[test]
fn while_loop_ends_on_dedent() {
    let source = "run: while x is True do\n    0\n".parse_source_full();
    assert_eq!(count_errors(&source), 0, "{:?}", source.symptoms);
    assert!(has_def(&source, "run"));
}

#[test]
fn removed_if_is_rejected_and_when_accepts_value_blocks() {
    let missing = "is_true: if x is True\n    1\n".parse_source_full();
    assert!(count_errors(&missing) > 0);

    let with_return = "is_true: when x is True then\n    1\n    return\n".parse_source_full();
    assert_eq!(count_errors(&with_return), 0);
}

#[test]
fn when_subject_is_patterns_parse() {
    let out = "what: when x is\n    1 then 'yes'\n    else 'no'\n";
    let out = out.parse_source_full();
    assert_eq!(count_errors(&out), 0);

    let out2 = "x: when x is\n    1 then 2\n    2 then 3\n";
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
pair : [\n    1\n    2\n]\nxs : [\n    1\n    2\n]\n";
    let list_and_tuple = list_and_tuple.parse_source_full();
    assert_eq!(count_errors(&list_and_tuple), 0);
}

#[test]
fn parse_record_and_format_interpolation() {
    let out = "msg: \"x={(n)} and y={f}\"\nmsg2: [a: 1, b: 2]\n".parse_source_full();
    assert_eq!(count_errors(&out), 0);
}
