use ast::ParameterKind;
use ast::{Expr, FnCall, HashFloat, Literal, ModPath, Pattern, SpanId, Spanned, TypeExpr, ty::Ty};
use internment::Intern;
use std::collections::HashMap;

fn intern(s: &str) -> Intern<String> {
    Intern::new(s.to_owned())
}

fn simple_var(name: &str) -> Expr {
    let n = intern(name);
    Expr::FnCall(FnCall {
        syntax: ast::CallSyntax::Path,
        definition: None,
        path: Spanned::new(ModPath::new(n, Vec::new()), SpanId::new(0)),
        args: None,
    })
}

fn list_cons_pat(head_name: &str, tail_name: &str) -> Pattern {
    Pattern::ListCons {
        head: Box::new(Spanned {
            value: Pattern::Nominal(intern(head_name), SpanId::new(0)),
            span_id: SpanId::new(0),
        }),
        tail: Box::new(Spanned {
            value: Pattern::Nominal(intern(tail_name), SpanId::new(1)),
            span_id: SpanId::new(1),
        }),
    }
}

#[test]
fn for_loop_pattern_cases() {
    #[allow(clippy::type_complexity)]
    let cases: Vec<(&str, Vec<Expr>, Option<Vec<&str>>, Option<&str>)> = vec![
        (
            "single_name",
            vec![simple_var("i")],
            Some(vec!["i"]),
            Some("i"),
        ),
        (
            "rejects_calls",
            vec![Expr::FnCall(FnCall {
                syntax: ast::CallSyntax::Path,
                definition: None,
                path: Spanned::new(ModPath::new(intern("f"), Vec::new()), SpanId::new(0)),
                args: Some(vec![]),
            })],
            None,
            None,
        ),
    ];

    for (name, exprs, expected_names, expected_single) in cases {
        let e = exprs.into_iter().next().unwrap();
        let result = e.for_loop_pattern_names();
        let expected: Option<Vec<Intern<String>>> =
            expected_names.map(|v| v.into_iter().map(intern).collect());
        assert_eq!(result, expected, "for_loop_pattern_names({name})");

        let single_result = e.for_loop_single_binding();
        let expected = expected_single.map(intern);
        assert_eq!(single_result, expected, "for_loop_single_binding({name})");
    }
}

#[test]
fn pattern_type_binding_names_cases() {
    let cases: Vec<(&str, Pattern, Vec<&str>)> = vec![
        (
            "generic",
            Pattern::Generic {
                name: intern("Some"),
                params: vec![(intern("v"), ParameterKind::Generic)],
                param_spans: vec![(intern("v"), SpanId::new(0))],
                span: SpanId::new(0),
            },
            vec!["v"],
        ),
        (
            "excludes_wildcard",
            Pattern::Generic {
                name: intern("Record"),
                params: vec![
                    (intern("_"), ParameterKind::Generic),
                    (intern("fields"), ParameterKind::Generic),
                ],
                param_spans: vec![
                    (intern("_"), SpanId::new(0)),
                    (intern("fields"), SpanId::new(1)),
                ],
                span: SpanId::new(0),
            },
            vec!["fields"],
        ),
        (
            "list_cons_lowercase",
            list_cons_pat("head", "tail"),
            vec!["head", "tail"],
        ),
        ("list_cons_uppercase", list_cons_pat("Head", "Tail"), vec![]),
        ("list_empty", Pattern::ListEmpty, vec![]),
    ];

    for (name, pat, expected_names) in cases {
        let result = pat.binding_names();
        let expected: Vec<Intern<String>> = expected_names.into_iter().map(intern).collect();
        assert_eq!(
            result.len(),
            expected.len(),
            "{name}: expected {} bindings, got {}",
            expected.len(),
            result.len()
        );
        for en in &expected {
            assert!(
                result.contains(en),
                "{name}: expected binding `{}` not found in {:?}",
                en.as_str(),
                result
            );
        }
    }
}

#[test]
fn surface_mangle_name_cases() {
    let cases: Vec<(&str, TypeExpr)> = vec![
        ("U32", TypeExpr::Nominal(intern("U32"), SpanId::new(1))),
        (
            "__literal_float",
            TypeExpr::Literal(Literal::Float(HashFloat(314.0 / 100.0)), SpanId::new(0)),
        ),
        (
            "__literal_int",
            TypeExpr::Literal(Literal::Int(42.into()), SpanId::new(0)),
        ),
    ];

    for (expected, expr) in cases {
        let pattern = Pattern::from(expr);
        assert_eq!(
            pattern.surface_mangle_name(),
            expected,
            "surface_mangle_name({pattern:?})"
        );
    }
}

#[test]
fn pattern_binding_types_list_cons() {
    let pattern = list_cons_pat("item", "rest");
    let subject = Ty::Array {
        elem: Box::new(Ty::u8()),
        size: 3.into(),
    };
    let bindings = pattern.pattern_binding_types(
        Some(&subject),
        &HashMap::new(),
        &HashMap::new(),
        &Ty::sequence_element_type,
    );
    assert_eq!(bindings.len(), 2);
    assert_eq!(bindings[&intern("item")], Ty::u8());
    assert!(
        matches!(&bindings[&intern("rest")], Ty::Array { elem, size }
        if **elem == Ty::u8() && size.as_const_int() == Some(2))
    );
}
