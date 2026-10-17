use ast::{BindValue, Expr};
use diagnostic::Category;
use parser::query::SourceParseExt;

fn assert_parses(source: &str) -> parser::query::ParseOutput {
    let parsed = source.parse_source_full();
    assert!(
        parsed
            .symptoms
            .iter()
            .all(|diagnostic| diagnostic.category != Category::Flaw),
        "{source}: {:?}",
        parsed.symptoms
    );
    parsed
}

#[test]
fn function_returns_are_inside_the_indented_body() {
    assert_parses("good Int:\n    value: 7\n    return value\n");
    assert_parses("good Int:\n    return 7\n");
}

#[test]
fn nested_expression_blocks_preserve_the_enclosing_return() {
    let parsed = assert_parses(
        "outer Int:\n    inner Int:\n        value: 7\n        value\n    return inner\nnext Int: 8\n",
    );
    assert_eq!(parsed.ast.defs.len(), 2);
    let BindValue::Body { exprs, ret } =
        &parsed.ast.defs[&internment::Intern::from_ref("outer")].value
    else {
        panic!("outer body");
    };
    assert_eq!(exprs.len(), 1);
    assert!(matches!(exprs[0].value, Expr::Bind(_)));
    assert_eq!(ret.values.len(), 1);
}

#[test]
fn when_return_does_not_consume_the_enclosing_function_return() {
    let parsed =
        assert_parses("choose(flag Int) Int:\n    when flag = 1 then\n        return 7\n    return 8\n");
    let BindValue::Body { exprs, ret } =
        &parsed.ast.defs[&internment::Intern::from_ref("choose")].value
    else {
        panic!("function body");
    };
    assert!(matches!(exprs[0].value, Expr::When(_)));
    assert_eq!(ret.values.len(), 1);
}

#[test]
fn aligned_returns_and_missing_returns_are_rejected() {
    for source in [
        "bad Int:\n    value: 7\nreturn value\n",
        "bad Int:\nreturn 7\n",
        "bad(flag Int) Int:\n    when flag = 1 then\n    return 7\n    return 8\n",
    ] {
        let parsed = source.parse_source_full();
        assert!(
            parsed
                .symptoms
                .iter()
                .any(|diagnostic| diagnostic.category == Category::Flaw),
            "{source}"
        );
    }
}

#[test]
fn inline_return_and_tab_indentation_are_accepted() {
    assert_parses("good Int: return 7\n");
    assert_parses("good Int:\n\twhen 1 = 1 then\n\t\treturn 7\n\treturn 8\n");
}

#[test]
fn malformed_body_recovers_before_the_next_function() {
    let parsed = "bad Int:\n    value: 7\ngood Int:\n    return 8\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    assert!(
        parsed
            .ast
            .defs
            .contains_key(&internment::Intern::from_ref("good"))
    );
}

#[test]
fn loops_end_on_dedent_and_preserve_following_statements() {
    let parsed = assert_parses(
        "run:\n    for index in 0 to 3\n        while index < 2 do\n            observe(index)\n        observe(index)\n    after\n    return 0\nnext: 1\n",
    );
    let BindValue::Body { exprs, .. } =
        &parsed.ast.defs[&internment::Intern::from_ref("run")].value
    else {
        panic!("function body");
    };
    assert_eq!(exprs.len(), 2);
    let Expr::Loop(ast::Loop::ForIn(loop_)) = &exprs[0].value else {
        panic!("for loop");
    };
    assert_eq!(loop_.exprs.len(), 2);
    assert!(matches!(
        loop_.exprs[0].value,
        Expr::Loop(ast::Loop::While(_))
    ));
    assert_eq!(parsed.ast.defs.len(), 2);
}

#[test]
fn loop_terminators_and_unindented_loop_bodies_are_rejected() {
    for source in [
        "run: for index in 0 to 3\n    observe(index)\nloop\n",
        "run: while ready do\n    observe\nloop\n",
        "run: for index in 0 to 3\nobserve(index)\n",
        "run: while ready do\nobserve\n",
    ] {
        let parsed = source.parse_source_full();
        assert!(
            parsed
                .symptoms
                .iter()
                .any(|diagnostic| diagnostic.category == Category::Flaw),
            "{source}"
        );
    }
}

#[test]
fn loop_keyword_spans_point_to_the_opening_keyword() {
    for (keyword, source) in [
        ("for", "run: for index in 0 to 3\n    observe(index)\n"),
        ("while", "run: while ready do\n    observe\n"),
    ] {
        let parsed = assert_parses(source);
        let BindValue::Expr(expression) =
            &parsed.ast.defs[&internment::Intern::from_ref("run")].value
        else {
            panic!("loop expression");
        };
        let keyword_span = match &expression.value {
            Expr::Loop(ast::Loop::ForIn(loop_)) => loop_.keyword_span,
            Expr::Loop(ast::Loop::While(loop_)) => loop_.keyword_span,
            _ => panic!("loop"),
        };
        let span = parsed.ast.span_table.get(keyword_span.into_inner());
        assert_eq!(&source[span.to_range()], keyword);
    }
}
