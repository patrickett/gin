mod support;

use ast::ConstValue;
use internment::Intern;
use support::check_with;
use typecheck::DefId;
use typecheck::eval::{CompileTimeHost, Evaluator};

const PRELUDE: &str = "Int is in -128 to 127\nInt.finish(self): return\n";

fn evaluate(body: &str, arguments: &[i128], expected: i128) {
    let typed = check_with(PRELUDE, body);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let target = flask::CompileTarget::Library;
    let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
    let actual = evaluator
        .call(
            DefId::named(Intern::from_ref("choose")),
            &arguments
                .iter()
                .map(|v| ConstValue::Int((*v).into()))
                .collect::<Vec<_>>(),
        )
        .unwrap();
    assert_eq!(actual, ConstValue::Int(expected.into()));
}

#[test]
fn multiline_branches_produce_their_final_expression() {
    let body = "choose(captured Int) Int:\n    value: when\n        captured < 0 then\n            adjusted: -captured\n            adjusted\n        else\n            captured\n    return value\n";
    evaluate(body, &[-7], 7);
    evaluate(body, &[4], 4);
}

#[test]
fn return_inside_a_value_initializer_exits_the_function() {
    let body = "choose(captured Int) Int:\n    value: when captured < 0 then\n        return 9\n    else\n        captured\n    return 5\n";
    evaluate(body, &[-1], 9);
    evaluate(body, &[4], 5);
    evaluate(
        "choose(captured Int) Int:\n    value Int:\n        return 9\n    return 5\n",
        &[4],
        9,
    );
}

#[test]
fn nonexhaustive_effect_branches_continue() {
    let body =
        "choose(captured Int) Int:\n    when captured < 0 then\n        return 9\n    return 3\n";
    evaluate(body, &[-1], 9);
    evaluate(body, &[4], 3);
}

#[test]
fn shared_subject_comparisons_and_returning_branches() {
    let body = "choose(captured Int) Int:\n    when captured\n        < 0 then return 1\n        = 0 then return 2\n        else return 3\n";
    evaluate(body, &[-4], 1);
    evaluate(body, &[0], 2);
    evaluate(body, &[4], 3);
}

#[test]
fn values_require_coverage_and_block_functions_require_returns() {
    for (body, code) in [
        (
            "choose(captured Int) Int:\n    captured\n",
            "type-missing-return",
        ),
        (
            "choose(captured Int) Int:\n    value: when captured < 0 then 1\n    return value\n",
            "type-missing-else-arm",
        ),
    ] {
        let typed = check_with(PRELUDE, body);
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.code.slug() == code),
            "{:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn pattern_alternatives_share_bindings_and_guards() {
    let body = "Choice is First(Int) or Second(Int) or Neither\nChoice.finish(self): return\nchoose(captured Int) Int:\n    result: when Choice.First(captured) is\n        First(value) or Second(value) and value >= 0 then\n            adjusted: value\n            adjusted\n        First(value) then 7\n        Second(value) then 8\n        Neither then 0\n    return result\n";
    evaluate(body, &[2], 2);
    evaluate(body, &[-2], 7);
    let second = body.replace("Choice.First(captured)", "Choice.Second(captured)");
    evaluate(&second, &[2], 2);
    evaluate(&second, &[-2], 8);
}

#[test]
fn chained_patterns_bind_names_for_later_guards() {
    let body = "Choice is First(Int) or Second(Int) or Neither\nChoice.finish(self): return\nchoose(captured Int) Int: return when\n    Choice.First(captured) is First(value)\n        and Choice.Second(value) is Second(final)\n        and final >= 0\n    then final\n    else 7\n";
    evaluate(body, &[2], 2);
    evaluate(body, &[-2], 7);
}

#[test]
fn nested_callable_returns_do_not_exit_the_outer_function() {
    let body = "choose(captured Int) Int:\n    local(value Int) Int:\n        when value < 0 then return 9\n        return value\n    value: local(captured)\n    return 5\n";
    evaluate(body, &[-1], 5);
    evaluate(body, &[4], 5);
}

#[test]
fn expression_blocks_preserve_reference_targets() {
    let valid = check_with(
        PRELUDE,
        "choose(input ref Int) ref Int:\n    ref value Int:\n        input\n    return value\n",
    );
    assert!(valid.all_flaws().is_empty(), "{:?}", valid.all_flaws());
    let invalid = check_with(
        PRELUDE,
        "choose(captured Int) ref Int:\n    value:\n        local Int: 7\n        ref local\n    return value\n",
    );
    assert!(
        invalid
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-reference-return-local-target"),
        "{:?}",
        invalid.all_flaws()
    );
}

#[test]
fn guards_do_not_count_as_coverage_and_alternatives_must_agree() {
    for (body, code) in [
        (
            "choose(captured Int) Int:\n    when captured < 0 then return\n    return captured\n",
            "type-return-mismatch",
        ),
        (
            "Choice is First(Int) or Second(Int)\nChoice.finish(self): return\nchoose(captured Int) Int: return when Choice.First(captured) is\n    First(value) and value > 0 then value\n    Second(value) then value\n",
            "type-missing-else-arm",
        ),
        (
            "Choice is First(Int) or Second(Int)\nChoice.finish(self): return\nchoose(captured Int) Int: return when Choice.First(captured) is\n    First(left) or Second(right) then 0\n    else 1\n",
            "type-incompatible-alternative-bindings",
        ),
    ] {
        let typed = check_with(PRELUDE, body);
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.code.slug() == code),
            "{:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn a_return_in_a_call_argument_skips_the_call() {
    let body = "discard(value Int) Int: return 5\nchoose(captured Int) Int: return discard(when captured < 0 then\n    return 9\nelse\n    captured\n)\n";
    evaluate(body, &[-1], 9);
    evaluate(body, &[4], 5);
}

const NESTED_CHOICES: &str = "Inner is Left(Int) or Right(Int)\nInner.finish(self): return\nOuter is Some(Inner) or None\nOuter.finish(self):\n    when self is\n        Some(value) then\n            value.finish\n            return\n        None then return\n";

#[test]
fn nested_is_guards_fall_through_to_outer_alternatives() {
    let body = format!(
        "{NESTED_CHOICES}choose(captured Int) Int:\n    return when Outer.Some(Inner.Left(captured)) is\n        Some(x) and x is\n            Left(y) and y > 0 then\n                adjusted: y\n                adjusted\n            Right(y) and y > 0 then y\n        Some(_) then 7\n        None then 9\n"
    );
    evaluate(&body, &[4], 4);
    evaluate(&body, &[-4], 7);
    let right = body.replace("Inner.Left(captured)", "Inner.Right(captured)");
    evaluate(&right, &[4], 4);
    evaluate(&right, &[-4], 7);
}

#[test]
fn flat_nested_constructors_bind_guard_and_body_names() {
    let body = format!(
        "{NESTED_CHOICES}choose(captured Int) Int: return when Outer.Some(Inner.Left(captured)) is\n    Some(Left(y)) and y > 0 then y\n    Some(_) then 7\n    None then 9\n"
    );
    evaluate(&body, &[4], 4);
    evaluate(&body, &[-4], 7);
    evaluate(
        &body.replace("Inner.Left(captured)", "Inner.Right(captured)"),
        &[4],
        7,
    );
}

#[test]
fn nested_unguarded_patterns_collectively_prove_coverage() {
    let body = format!(
        "{NESTED_CHOICES}choose(captured Int) Int: return when Outer.Some(Inner.Left(captured)) is\n    Some(x) and x is\n        Left(y) then y\n        Right(y) then y\n    None then 9\n"
    );
    evaluate(&body, &[4], 4);
    evaluate(
        &body.replace("Inner.Left(captured)", "Inner.Right(captured)"),
        &[-4],
        -4,
    );
    let flat = body.replace(
        "    Some(x) and x is\n        Left(y) then y\n        Right(y) then y",
        "    Some(Left(y)) then y\n    Some(Right(y)) then y",
    );
    evaluate(&flat, &[4], 4);
}

#[test]
fn partial_nested_guards_and_independent_inner_matches_require_coverage() {
    for arms in [
        "    Some(x) and x is\n        Left(y) and y > 0 then y\n    None then 9\n",
        "    Some(x) and x is\n        Left(y) then y\n    None then 9\n",
        "    Some(x) then\n        when x is\n            Left(y) then y\n    Some(_) then 7\n    None then 9\n",
    ] {
        let body = format!(
            "{NESTED_CHOICES}choose(captured ref Outer) Int: return when captured is\n{arms}"
        );
        let typed = check_with(PRELUDE, &body);
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.code.slug() == "type-missing-else-arm"),
            "{body}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn nested_body_results_commit_and_local_else_stays_in_its_group() {
    let body = format!(
        "{NESTED_CHOICES}choose(captured Int) Int: return when Outer.Some(Inner.Left(captured)) is\n    Some(x) and x is\n        Left(y) and y > 0 then 0\n        else 2\n    Some(_) then 7\n    None then 9\n"
    );
    evaluate(&body, &[4], 0);
    evaluate(&body, &[-4], 2);
    evaluate(
        &body.replace("Inner.Left(captured)", "Inner.Right(captured)"),
        &[4],
        2,
    );
}

#[test]
fn nested_pattern_bindings_do_not_escape_into_outer_alternatives() {
    let body = format!(
        "{NESTED_CHOICES}choose(captured ref Outer) Int: return when captured is\n    Some(x) and x is\n        Left(y) and y > 0 then deref y\n    Some(_) then y\n    None then 9\n"
    );
    let typed = check_with(PRELUDE, &body);
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-unknown-symbol"
                && flaw.arg("name") == Some("y")),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn nested_constructor_alternatives_share_user_bindings() {
    let body = format!(
        "{NESTED_CHOICES}choose(captured Int) Int: return when Outer.Some(Inner.Left(captured)) is\n    Some(Left(y)) or Some(Right(y)) and y > 0 then y\n    Some(_) then 7\n    None then 9\n"
    );
    evaluate(&body, &[4], 4);
    evaluate(
        &body.replace("Inner.Left(captured)", "Inner.Right(captured)"),
        &[4],
        4,
    );
    evaluate(&body, &[-4], 7);
}

#[test]
fn failed_nested_guards_preserve_owned_payloads_for_later_alternatives() {
    let body = format!(
        "{NESTED_CHOICES}select(value Outer) Outer: return when value is\n    Some(x) and x is\n        Left(y) and y > 0 then Outer.Some(Inner.Left(y))\n    Some(x) then Outer.Some(x)\n    None then Outer.None\n"
    );
    let typed = check_with(PRELUDE, &body);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn deeper_nested_failures_try_inner_siblings_before_outer_alternatives() {
    let body = format!(
        "{NESTED_CHOICES}choose(captured Int) Int: return when Outer.Some(Inner.Left(captured)) is\n    Some(x) and x is\n        Left(y) and y is\n            0 then 1\n            1 then 2\n        Left(_) then 3\n        Right(_) then 4\n    None then 9\n"
    );
    evaluate(&body, &[0], 1);
    evaluate(&body, &[1], 2);
    evaluate(&body, &[4], 3);
    evaluate(
        &body.replace("Inner.Left(captured)", "Inner.Right(captured)"),
        &[4],
        4,
    );
}
