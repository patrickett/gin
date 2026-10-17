use ast::{BindValue, Expr, SequenceLiteral};
use internment::Intern;
use parser::query::SourceParseExt;

#[test]
fn binding_sequences_preserve_names_and_nesting() {
    for (source, names) in [
        ("values: 10, 20\n", vec![None, None]),
        ("values: [10, 20]\n", vec![None, None]),
        ("values: x: 10, y: 20\n", vec![Some("x"), Some("y")]),
        ("values: [x: 10, 20]\n", vec![Some("x"), None]),
        ("values: [10, 20], [30, 40]\n", vec![None, None]),
    ] {
        let parsed = source.parse_source_full();
        assert!(
            parsed
                .symptoms
                .iter()
                .all(|d| d.category != diagnostic::Category::Flaw),
            "{source}: {:?}",
            parsed.symptoms
        );
        let BindValue::Expr(expr) = &parsed.ast.defs[&Intern::from_ref("values")].value else {
            panic!("{source}")
        };
        let Expr::SequenceLiteral(SequenceLiteral::Elements(elems)) = &expr.value else {
            panic!("{expr:?}")
        };
        assert_eq!(
            elems
                .iter()
                .map(|elem| elem.name.as_ref().map(|name| name.as_str()))
                .collect::<Vec<_>>(),
            names
        );
    }
}

#[test]
fn call_commas_do_not_enter_a_named_argument_or_nested_sequence() {
    for source in ["value: call(x: 1, y: 2)\n", "value: call([1, 2], [3, 4])\n"] {
        let parsed = source.parse_source_full();
        assert!(
            parsed
                .symptoms
                .iter()
                .all(|d| d.category != diagnostic::Category::Flaw),
            "{source}: {:?}",
            parsed.symptoms
        );
        let BindValue::Expr(expr) = &parsed.ast.defs[&Intern::from_ref("value")].value else {
            panic!()
        };
        let Expr::FnCall(call) = &expr.value else {
            panic!("{expr:?}")
        };
        assert_eq!(call.args.as_ref().unwrap().len(), 2);
    }
}

#[test]
fn result_signatures_allow_optional_names_and_nested_array_types() {
    for source in [
        "make() Int, Int: return 1, 2\n",
        "make() first Int, Int: return 1, 2\n",
        "make() Int, second Int: return 1, 2\n",
        "make() values [Int; 2], count Int: return [1, 2], 2\n",
    ] {
        let parsed = source.parse_source_full();
        assert!(
            parsed
                .symptoms
                .iter()
                .all(|d| d.category != diagnostic::Category::Flaw),
            "{source}: {:?}",
            parsed.symptoms
        );
        assert_eq!(
            parsed.ast.defs[&Intern::from_ref("make")]
                .named_results
                .len(),
            2
        );
    }
}

#[test]
fn duplicate_names_and_removed_parenthesized_values_are_rejected() {
    for (source, code) in [
        ("value: [x: 1, x: 2]\n", "parse-duplicate-sequence-name"),
        (
            "get(value [x Int, x Int]): return value\n",
            "parse-duplicate-sequence-name",
        ),
        ("value: (1, 2)\n", "parse-parenthesized-sequence"),
        ("value: (x: 1, y: 2)\n", "parse-parenthesized-sequence"),
        ("value: (0; 2)\n", "parse-parenthesized-sequence"),
        ("value: ()\n", "parse-empty-parens-removed"),
    ] {
        let parsed = source.parse_source_full();
        assert!(
            parsed.symptoms.iter().any(|d| d.code.slug() == code),
            "{source}: {:?}",
            parsed.symptoms
        );
    }
}
