//! Consuming sequence loops account for each original position and captured owner.
use ast::{Block, Expr, Item, LoopExpression, SourceFile, Statement};
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_file, check_module};

const LIBRARY: &str = include_str!("../examples/consuming-for.gin");
const CLIENT: &str = include_str!("../examples/consuming-for-client.gin");

fn check(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid consuming-for source");
    let before = source.clone();
    let result = check_module(name, &source, dependencies, Limits::default());
    assert_eq!(source, before);
    result
}

#[test]
fn consuming_for_checks_real_cleanup_bodies_and_separate_clients() {
    let library = check("library.loops", LIBRARY, &[]).unwrap();
    check("app.loops", CLIENT, &[&library]).unwrap();
}

#[test]
fn current_elements_and_iteration_local_owners_cannot_be_abandoned() {
    for body in [
        "        ()\n",
        "        alias: value\n",
        "        consume(value)\n        extra: Token(number: 1)\n",
    ] {
        let text = format!(
            "{LIBRARY}
bad(values (Token, Token)):
    for value in values do
{body}    return
"
        );
        assert_eq!(
            check("bad.drop", &text, &[]).unwrap_err(),
            SourceCheckError::Unconsumed,
            "{text}"
        );
    }
}

#[test]
fn an_element_cannot_be_consumed_twice_in_one_iteration() {
    let text = format!(
        "{LIBRARY}
bad(values (Token, Token)):
    for value in values do
        consume(value)
        consume(value)
    return
"
    );
    assert_eq!(
        check("bad.double", &text, &[]).unwrap_err(),
        SourceCheckError::UseAfterMove
    );
}

#[test]
fn the_original_collection_is_unavailable_during_and_after_consuming_iteration() {
    for body in [
        concat!(
            "    for value in values do\n",
            "        consume(value)\n",
            "        alias: values\n",
            "    return\n",
        ),
        concat!(
            "    for value in values do\n",
            "        consume(value)\n",
            "    return values\n",
        ),
    ] {
        let text = format!("{LIBRARY}\nbad(values (Token, Token)) (Token, Token):\n{body}");
        assert_eq!(
            check("bad.collection", &text, &[]).unwrap_err(),
            SourceCheckError::UseAfterMove
        );
    }
}

#[test]
fn an_outer_owner_cannot_disappear_across_the_back_edge() {
    let text = format!(
        "{LIBRARY}
bad(outer Token, values (Token, Token)):
    for value in values do
        consume(value)
        consume(outer)
    return
"
    );
    assert_eq!(
        check("bad.capture", &text, &[]).unwrap_err(),
        SourceCheckError::UseAfterMove
    );
}

#[test]
fn loop_binders_shadow_outer_values_without_reviving_them() {
    let text = format!(
        "{LIBRARY}
shadow(values (Token, Token)):
    for values in values do
        consume(values)
    return
"
    );
    check("app.shadow", &text, &[]).unwrap();
}

#[test]
fn dependency_scheduling_respects_the_loop_binder() {
    let text = format!(
        "{LIBRARY}
drain_shadow(values (Token, Token)):
    for finish in values do
        consume(finish)
    return
finish:
    drain_shadow((Token(number: 1), Token(number: 2)))
    return
"
    );
    check("app.dependencies", &text, &[]).unwrap();
}

#[test]
fn early_returns_do_not_silently_discard_pending_positions() {
    let text = format!(
        "{LIBRARY}
bad(values (Token, Token)):
    for value in values do
        consume(value)
        return
    return
"
    );
    assert_eq!(
        check("bad.early", &text, &[]).unwrap_err(),
        SourceCheckError::Unsupported
    );
}

#[test]
fn the_final_iteration_can_account_for_a_captured_owner_once() {
    let text = format!(
        "{LIBRARY}
once(outer Token, values (Token,)):
    for value in values do
        consume(value)
        consume(outer)
    return
"
    );
    check("app.once", &text, &[]).unwrap();
    // Only mutate the final return of once, leaving helper bodies unchanged.
    let end = text.rfind("    return\n").unwrap();
    let reused = format!("{}    consume(outer)\n    return\n", &text[..end]);
    assert_eq!(
        check("bad.once", &reused, &[]).unwrap_err(),
        SourceCheckError::UseAfterMove
    );
}

#[test]
fn empty_sequence_iteration_requires_an_explicit_no_op_body() {
    let text = "
empty:
    for value in () do
        ()
    return
";
    check("app.empty", text, &[]).unwrap();
    let unsupported = format!(
        "{LIBRARY}
empty:
    for value in () do
        consume(value)
    return
"
    );
    assert_eq!(
        check("bad.empty_body", &unsupported, &[]).unwrap_err(),
        SourceCheckError::Unsupported
    );
}

#[test]
fn scalar_and_unit_positions_are_accounted_for_by_consuming_iteration() {
    let text = "
consume_free:
    for value in (1, (), 2) do
        ()
    return
";
    check("app.free_positions", text, &[]).unwrap();
}

#[test]
fn an_iterable_expression_is_evaluated_exactly_once() {
    let text = format!(
        "{LIBRARY}
produce(first Token, second Token) (Token, Token):
    return first, second
run(first Token, second Token):
    for value in produce(first, second) do
        consume(value)
    return
"
    );
    check("app.once_iterable", &text, &[]).unwrap();
}

#[test]
fn a_shadowed_outer_owner_is_restored_without_consuming_or_losing_it() {
    let text = format!(
        "{LIBRARY}
shadow_original(value Token, values (Token, Token)):
    for value in values do
        consume(value)
    consume(value)
    return
"
    );
    check("app.outer_shadow", &text, &[]).unwrap();
}

fn loop_body(source: &mut SourceFile) -> &mut Block {
    let Item::Binding(binding) = &mut source.items.last_mut().unwrap().value else {
        panic!("loop function");
    };
    let Expr::Block(function) = &mut binding.value.as_mut().unwrap().expression.value else {
        panic!("function block");
    };
    let Statement::Expression(expression) = &mut function.statements[0].value else {
        panic!("loop statement");
    };
    let Expr::Loop(loop_expression) = &mut expression.value else {
        panic!("loop");
    };
    let LoopExpression::For { body, .. } = loop_expression.as_mut() else {
        panic!("for");
    };
    let Expr::Block(body) = &mut body.value else {
        panic!("loop block");
    };
    body
}

#[test]
fn manual_asts_cannot_hide_a_written_early_return() {
    let text = format!(
        "{LIBRARY}
bad(values (Token, Token)):
    for value in values do
        consume(value)
        return ()
    return
"
    );
    let source = parser::parse_strict(&text).unwrap();
    assert_eq!(
        check_file(&source, Limits::default()).unwrap_err(),
        SourceCheckError::Unsupported
    );
    for erase in [false, true] {
        let mut changed = source.clone();
        let body = loop_body(&mut changed);
        if erase {
            body.statements.remove(1);
        } else {
            let Statement::Return(returned) = &body.statements[1].value else {
                panic!("written return");
            };
            let unit = returned.value.clone().unwrap();
            body.statements[1].value = Statement::Expression(unit);
        }
        assert_eq!(
            check_file(&changed, Limits::default()).unwrap_err(),
            SourceCheckError::Malformed
        );
    }
}

#[test]
fn empty_iteration_accepts_a_block_of_validated_no_ops() {
    let text = "
empty:
    for value in () do
        ()
        (())
    return
";
    check("app.empty_block", text, &[]).unwrap();
}
