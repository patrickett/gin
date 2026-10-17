use ast::{ConstValue, Expr, Literal, ParameterKind, Pattern, SpanId, TagCall, Typed};
use internment::Intern;
use typecheck::analysis::pattern_matches_public;

fn tag(name: &str, args: Vec<ConstValue>) -> ConstValue {
    ConstValue::Tag {
        name: Intern::from_ref(name),
        qual_path: None,
        args: args.into(),
    }
}

fn nested_pattern() -> Pattern {
    Pattern::from_expr(Expr::TagCall(TagCall {
        name: Intern::from_ref("Outer"),
        qual_path: None,
        args: vec![Typed::infer(
            Expr::TagCall(TagCall {
                name: Intern::from_ref("Inner"),
                qual_path: None,
                args: vec![Typed::infer(
                    Expr::Lit(Literal::Int(7.into())),
                    SpanId::INVALID,
                )],
            }),
            SpanId::INVALID,
        )],
    }))
}

#[test]
fn nested_constructor_literals_match_their_complete_payload() {
    assert!(pattern_matches_public(
        &nested_pattern(),
        &tag("Outer", vec![tag("Inner", vec![ConstValue::Int(7.into())])])
    ));
}

#[test]
fn nested_constructor_constraints_reject_other_payloads_and_arities() {
    for value in [
        tag("Outer", vec![tag("Inner", vec![ConstValue::Int(0.into())])]),
        tag("Outer", vec![tag("Other", vec![ConstValue::Int(7.into())])]),
        tag("Outer", vec![tag("Inner", vec![])]),
        tag(
            "Outer",
            vec![tag(
                "Inner",
                vec![ConstValue::Int(7.into()), ConstValue::Int(7.into())],
            )],
        ),
    ] {
        assert!(
            !pattern_matches_public(&nested_pattern(), &value),
            "{value:?}"
        );
    }
}

#[test]
fn missing_constructor_metadata_is_not_a_capture() {
    let pattern = Pattern::Generic {
        name: Intern::from_ref("Outer"),
        params: vec![],
        param_spans: vec![(Intern::from_ref("value"), SpanId::INVALID)],
        span: SpanId::INVALID,
    };
    assert!(!pattern_matches_public(
        &pattern,
        &tag("Outer", vec![ConstValue::Int(7.into())])
    ));
}

#[test]
fn explicit_constructor_captures_still_match() {
    let name = Intern::from_ref("value");
    let pattern = Pattern::Generic {
        name: Intern::from_ref("Outer"),
        params: vec![(name, ParameterKind::Generic)],
        param_spans: vec![(name, SpanId::INVALID)],
        span: SpanId::INVALID,
    };
    assert!(pattern_matches_public(
        &pattern,
        &tag("Outer", vec![ConstValue::Int(7.into())])
    ));
}

#[test]
fn nested_qualified_constructor_keeps_its_identity() {
    let value = ConstValue::Tag {
        name: Intern::from_ref("Ready"),
        qual_path: Some("network.State".into()),
        args: vec![ConstValue::Int(7.into())].into(),
    };
    let pattern = Pattern::from_expr(Expr::TagCall(TagCall {
        name: Intern::from_ref("Outer"),
        qual_path: None,
        args: vec![Typed::infer(
            typecheck::const_value_to_expr(&value),
            SpanId::INVALID,
        )],
    }));
    assert!(pattern_matches_public(&pattern, &tag("Outer", vec![value])));
    for qualifier in [None, Some("other.State".to_string())] {
        let value = ConstValue::Tag {
            name: Intern::from_ref("Ready"),
            qual_path: qualifier,
            args: vec![ConstValue::Int(7.into())].into(),
        };
        assert!(!pattern_matches_public(
            &pattern,
            &tag("Outer", vec![value])
        ));
    }
}

fn evaluate(source: &str) -> ConstValue {
    let ast = parser::cursor::TokenCursor::parse_source(&format!("value: {source}\n"));
    let bind = ast
        .defs
        .get(&Intern::from_ref("value"))
        .expect("value definition");
    let ast::BindValue::Expr(expr) = &bind.value else {
        panic!("expression definition");
    };
    typecheck::analysis::CompTimeEvaluator::new(&Default::default(), &ast)
        .eval(&expr.value)
        .expect("constant expression")
}

#[test]
fn parsed_nested_constructor_patterns_select_the_matching_arm() {
    assert_eq!(
        evaluate("when Outer(Inner(7)) is Outer(Inner(7)) then 1 else 0"),
        ConstValue::Int(1.into())
    );
}

#[test]
fn parsed_qualified_nested_constructor_patterns_select_the_matching_arm() {
    assert_eq!(
        evaluate("when Outer(State.Ready(7)) is Outer(State.Ready(7)) then 1 else 0"),
        ConstValue::Int(1.into())
    );
}

#[test]
fn matching_constructor_capture_metadata_is_available_to_binding_extraction() {
    assert_eq!(
        evaluate("when Some(7) is Some(x) then x else 0"),
        ConstValue::Int(7.into())
    );
}
