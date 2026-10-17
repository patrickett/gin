use ast::{BindValue, Expr};
use parser::query::SourceParseExt;

#[test]
fn bracket_indexing_reads_writes_and_chains() {
    for expression in [
        "pointer.[index]",
        "pointer .[index]",
        "pointer . [index]",
        "pointer.[index + 1]",
        "self.pointer.[index]",
        "make(0).[index].[next]",
        "[1, 2].[index]",
        "pointer.[\n        index\n    ]",
        "pointer.[index]:: value",
        "self.pointer.[index]:: value",
        "pointer.[index] +: value",
    ] {
        let parsed = format!("run:\n    {expression}\n    return\n").parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{expression}: {:?}",
            parsed.symptoms
        );
        let bind = parsed
            .ast
            .defs
            .values()
            .find(|bind| bind.name.as_str() == "run")
            .unwrap();
        let BindValue::Body { exprs, .. } = &bind.value else {
            panic!("body")
        };
        assert_eq!(exprs.len(), 1, "{expression}: {exprs:?}");
        assert!(
            matches!(exprs[0].value, Expr::BufGet { .. } | Expr::BufSet { .. }),
            "{expression}: {exprs:?}"
        );
    }
}

#[test]
fn indexing_keeps_type_annotations_and_separate_sequences_distinct() {
    let parsed = "run:\n    values [in 0 to 255; 2]: [1, 2]\n    pending [in 0 to 255; 2]\n    values.[0]\n    return values.[1]\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let bind = parsed
        .ast
        .defs
        .values()
        .find(|bind| bind.name.as_str() == "run")
        .unwrap();
    let BindValue::Body { exprs, ret } = &bind.value else {
        panic!("body")
    };
    assert!(matches!(exprs[0].value, Expr::Bind(_)));
    assert!(matches!(exprs[1].value, Expr::Bind(_)));
    assert!(matches!(exprs[2].value, Expr::BufGet { .. }));
    assert!(matches!(ret.values[0].value, Expr::BufGet { .. }));
    let parsed = "run:\n    use_value(value)\n    [1, 2]\n    return\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
}

#[test]
fn indexing_requires_one_index_and_a_closing_bracket() {
    for expression in ["pointer.[]", "pointer.[0, 1]", "pointer.[0"] {
        let parsed = format!("read: {expression}\n").parse_source_full();
        assert!(!parsed.symptoms.is_empty(), "{expression}");
    }
}

#[test]
fn removed_indexing_forms_are_rejected() {
    for expression in [
        "pointer.(0)",
        "pointer.(0):: value",
        "pointer[0]",
        "pointer[0]:: value",
        "make(0)[index]",
    ] {
        let parsed = format!("read: {expression}\n").parse_source_full();
        assert!(
            parsed
                .symptoms
                .iter()
                .any(|diagnostic| diagnostic.category == diagnostic::Category::Flaw),
            "{expression}: {:?}",
            parsed.symptoms
        );
    }
}

#[test]
fn array_type_annotations_do_not_depend_on_spacing() {
    for gap in ["", " "] {
        let parsed = format!("values{gap}[in 0 to 255; 2]: [1, 2]\n").parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        assert_eq!(parsed.ast.defs.len(), 1);
        let bind = parsed.ast.defs.values().next().unwrap();
        assert_eq!(bind.name.as_str(), "values");
        assert!(bind.return_tag.is_some());
    }
}

#[test]
fn slice_borrows_accept_start_and_count_expressions() {
    for expression in [
        "ref source.[0; count]",
        "mut source.[start; count]",
        "ref make(source).[start + 1; count(source)]",
        "ref source.[\n    0;\n    count\n]",
    ] {
        let parsed = format!("borrow: {expression}\n").parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{expression}: {:?}",
            parsed.symptoms
        );
        let bind = parsed
            .ast
            .defs
            .values()
            .find(|bind| bind.name.as_str() == "borrow")
            .unwrap();
        let BindValue::Expr(expression) = &bind.value else {
            panic!("expression body")
        };
        assert!(
            matches!(&expression.value, Expr::Ref { inner, .. } if matches!(inner.value, Expr::SliceGet { .. }))
        );
    }
}
