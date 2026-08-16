use parser::query::{ParseOutput, SourceParseExt};

fn has_unexpected_token_symptom(output: &ParseOutput, token: &str) -> bool {
    output
        .symptoms
        .iter()
        .any(|d| d.message == format!("unexpected token {token}"))
}

#[test]
fn standalone_equals_is_unexpected_token() {
    let output = "main:\n  =\n    return".parse_source_full();
    assert!(has_unexpected_token_symptom(&output, "`=`"));
}

#[test]
fn standalone_double_equals_is_unexpected_token() {
    let output = "main:\n  ==\n    return".parse_source_full();
    assert!(has_unexpected_token_symptom(&output, "`==`"));
}

#[test]
fn standalone_not_equals_is_unexpected_token() {
    let output = "main:\n  /=\n    return".parse_source_full();
    assert!(has_unexpected_token_symptom(&output, "`/=`"));
}

#[test]
fn standalone_equals_after_blank_lines_uses_equals_span() {
    let output = "main:\n  =\n    return".parse_source_full();
    assert!(has_unexpected_token_symptom(&output, "`=`"));
}

#[test]
fn infix_equals_parses_as_equality() {
    let output = "x: a = b\n".parse_source_full();
    assert!(!has_unexpected_token_symptom(&output, "`=`"));
    let bind = output
        .ast
        .defs
        .get(&internment::Intern::from_ref("x"))
        .expect("x definition");
    let ast::BindValue::Expr(expr) = &bind.value else {
        panic!("expected expression definition");
    };
    assert!(matches!(
        &expr.value,
        ast::Expr::Binary(binary) if binary.op == ast::BinOp::Equal
    ));
}

#[test]
fn equality_in_call_argument_is_an_expression() {
    let output = "result: choose(a = b)\n".parse_source_full();
    assert!(output.symptoms.is_empty(), "{:?}", output.symptoms);
    let bind = output
        .ast
        .defs
        .get(&internment::Intern::from_ref("result"))
        .expect("result definition");
    let ast::BindValue::Expr(expr) = &bind.value else {
        panic!("expected expression bind");
    };
    let ast::Expr::FnCall(call) = &expr.value else {
        panic!("expected function call");
    };
    assert!(matches!(
        call.args.as_deref(),
        Some([arg])
            if matches!(
                arg.value,
                ast::Expr::Binary(ref binary) if binary.op == ast::BinOp::Equal
            )
    ));
}

#[test]
fn infix_double_equals_is_unexpected_token() {
    let output = "x: a == b\n".parse_source_full();
    assert!(has_unexpected_token_symptom(&output, "`==`"));
}

#[test]
fn infix_not_equals_is_unexpected_token() {
    let output = "x: a /= b\n".parse_source_full();
    assert!(has_unexpected_token_symptom(&output, "`/=`"));
}

#[test]
fn less_than_still_works() {
    let output = "x: a < b\n".parse_source_full();
    assert!(!has_unexpected_token_symptom(&output, "`<`"));
}

#[test]
fn greater_than_still_works() {
    let output = "x: a > b\n".parse_source_full();
    assert!(!has_unexpected_token_symptom(&output, "`>`"));
}

#[test]
fn less_than_or_equal_still_works() {
    let output = "x: a <= b\n".parse_source_full();
    assert!(!has_unexpected_token_symptom(&output, "`<=`"));
}

#[test]
fn greater_than_or_equal_still_works() {
    let output = "x: a >= b\n".parse_source_full();
    assert!(!has_unexpected_token_symptom(&output, "`>=`"));
}
