use super::*;
use ast::SpanId;
use parser::query::SourceParseExt;

fn parse(source: &str) -> FileAst {
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    parsed.ast
}

fn parent() -> ComptimeCall {
    ComptimeCall {
        name: Intern::from_ref("parent"),
        args: vec![ConstValue::ONE].into(),
    }
}

#[test]
fn a_failed_return_restores_the_parent_call_stack() {
    let file =
        parse("foreign(value): return unknown\ncompute(value):\n    return foreign(value)\n");
    let env = ConstEnv::default();
    let parent = parent();
    let mut stack = HashSet::from([parent.clone()]);
    let mut frame = EvalFrame::new(&env, &file, None, None, &mut stack);
    let bind = &file.defs[&Intern::from_ref("compute")];
    assert!(matches!(bind.value, BindValue::Body { .. }));
    assert_eq!(frame.eval_bind_call(bind, &[ConstValue::ONE], 0), None);
    assert_eq!(frame.call_stack.len(), 1);
    assert!(frame.call_stack.contains(&parent));
}

#[test]
fn completed_and_empty_returns_restore_the_parent_call_stack() {
    for (body, expected) in [
        ("return 7", Some(ConstValue::Int(7.into()))),
        (
            "return 1, 2",
            Some(ConstValue::ResultRow(
                vec![ConstValue::ONE, ConstValue::Int(2.into())].into(),
            )),
        ),
        ("return", None),
    ] {
        let file = parse(&format!("compute(value):\n    {body}\n"));
        let env = ConstEnv::default();
        let parent = parent();
        let mut stack = HashSet::from([parent.clone()]);
        let mut frame = EvalFrame::new(&env, &file, None, None, &mut stack);
        let bind = &file.defs[&Intern::from_ref("compute")];
        assert_eq!(frame.eval_bind_call(bind, &[ConstValue::ONE], 0), expected);
        assert_eq!(frame.call_stack.len(), 1);
        assert!(frame.call_stack.contains(&parent));
    }
}

#[test]
fn rejected_recursion_preserves_the_existing_active_call() {
    let file = parse("compute(value): return 7\n");
    let env = ConstEnv::default();
    let active = ComptimeCall {
        name: Intern::from_ref("compute"),
        args: vec![ConstValue::ONE].into(),
    };
    let mut stack = HashSet::from([active.clone()]);
    let mut frame = EvalFrame::new(&env, &file, None, None, &mut stack);
    assert_eq!(
        frame.eval_bind_call(&file.defs[&active.name], &[ConstValue::ONE], 0),
        None
    );
    assert_eq!(frame.call_stack.len(), 1);
    assert!(frame.call_stack.contains(&active));
}

#[test]
fn public_call_evaluation_rejects_missing_and_extra_arguments() {
    let file = parse("compute(value): return 7\n");
    let env = ConstEnv::default();
    let evaluator = CompTimeEvaluator::new(&env, &file);
    let bind = &file.defs[&Intern::from_ref("compute")];
    for args in [vec![], vec![ConstValue::ONE, ConstValue::ONE]] {
        assert_eq!(evaluator.eval_bind_call(bind, &args), None);
    }
    assert_eq!(
        evaluator.eval_bind_call(bind, &[ConstValue::ONE]),
        Some(ConstValue::Int(7.into()))
    );
}

#[test]
fn a_failed_call_can_be_retried_after_its_dependency_becomes_known() {
    let mut file =
        parse("foreign(value): return unknown\ncompute(value):\n    return foreign(value)\n");
    let env = ConstEnv::default();
    let mut stack = HashSet::new();
    let name = Intern::from_ref("compute");
    {
        let mut frame = EvalFrame::new(&env, &file, None, None, &mut stack);
        assert_eq!(
            frame.eval_bind_call(&file.defs[&name], &[ConstValue::ONE], 0),
            None
        );
    }
    file.defs
        .get_mut(&Intern::from_ref("foreign"))
        .unwrap()
        .value = BindValue::Expr(Box::new(Typed::infer(
        Expr::Lit(Literal::Int(7.into())),
        SpanId::INVALID,
    )));
    let mut frame = EvalFrame::new(&env, &file, None, None, &mut stack);
    assert_eq!(
        frame.eval_bind_call(&file.defs[&name], &[ConstValue::ONE], 0),
        Some(ConstValue::Int(7.into()))
    );
    assert!(frame.call_stack.is_empty());
}

#[test]
fn an_unknown_rebind_does_not_reuse_the_previous_local_value() {
    let file = parse(
        "foreign(value): return unknown\ncompute(input):\n    value: 1\n    value:: foreign(input)\n    return value\n",
    );
    let env = ConstEnv::default();
    let evaluator = CompTimeEvaluator::new(&env, &file);
    assert_eq!(
        evaluator.eval_bind_call(&file.defs[&Intern::from_ref("compute")], &[ConstValue::ONE]),
        None
    );
}

#[test]
fn an_unknown_local_initializer_does_not_reuse_a_global_value() {
    let file = parse(
        "value := 7\nforeign(input): return unknown\ncompute(input):\n    value := foreign(input)\n    return value\n",
    );
    let env: ConstEnv = [(Intern::from_ref("value"), Some(ConstValue::Int(7.into())))]
        .into_iter()
        .collect();
    let evaluator = CompTimeEvaluator::new(&env, &file);
    assert_eq!(
        evaluator.eval_bind_call(&file.defs[&Intern::from_ref("compute")], &[ConstValue::ONE]),
        None
    );
}

#[test]
fn a_known_rebind_replaces_the_previous_local_value() {
    let file = parse("compute(input):\n    value: input\n    value:: 2\n    return value\n");
    let env = ConstEnv::default();
    let evaluator = CompTimeEvaluator::new(&env, &file);
    assert_eq!(
        evaluator.eval_bind_call(&file.defs[&Intern::from_ref("compute")], &[ConstValue::ONE]),
        Some(ConstValue::Int(2.into()))
    );
}

#[test]
fn an_unknown_initializer_restores_the_parent_call_stack() {
    let file = parse(
        "foreign(value): return unknown\ncompute(input):\n    value := foreign(input)\n    return value\n",
    );
    let env = ConstEnv::default();
    let parent = parent();
    let mut stack = HashSet::from([parent.clone()]);
    let mut frame = EvalFrame::new(&env, &file, None, None, &mut stack);
    assert_eq!(
        frame.eval_bind_call(
            &file.defs[&Intern::from_ref("compute")],
            &[ConstValue::ONE],
            0
        ),
        None
    );
    assert_eq!(frame.call_stack.len(), 1);
    assert!(frame.call_stack.contains(&parent));
}

#[test]
fn compound_assignments_apply_the_operator_to_the_existing_value() {
    for (assignment, expected) in [
        ("value+: 2", 10),
        ("value-: 2", 6),
        ("value*: 2", 16),
        ("value/: 2", 4),
        ("value%: 3", 2),
        ("value<<: 1", 16),
        ("value>>: 1", 4),
    ] {
        let file = parse(&format!(
            "compute(input):\n    value: input\n    {assignment}\n    return value\n"
        ));
        let env = ConstEnv::default();
        let evaluator = CompTimeEvaluator::new(&env, &file);
        assert_eq!(
            evaluator.eval_bind_call(
                &file.defs[&Intern::from_ref("compute")],
                &[ConstValue::Int(8.into())]
            ),
            Some(ConstValue::Int(expected.into())),
            "{assignment}"
        );
    }
}

#[test]
fn compound_assignment_reads_the_old_value_before_replacing_it() {
    let file = parse(
        "compute(input):\n    value: input\n    value+: value\n    value*: 3\n    return value\n",
    );
    let env = ConstEnv::default();
    let evaluator = CompTimeEvaluator::new(&env, &file);
    assert_eq!(
        evaluator.eval_bind_call(&file.defs[&Intern::from_ref("compute")], &[ConstValue::ONE]),
        Some(ConstValue::Int(6.into()))
    );
}

#[test]
fn invalid_or_unsupported_compound_operations_do_not_become_rhs_values() {
    for assignment in [
        "value/: 0",
        "value%: 0",
        "value<<: 256",
        "value>>: -1",
        "value&: 1",
        "value|: 1",
        "value^: 1",
    ] {
        let file = parse(&format!(
            "compute(input):\n    value: input\n    {assignment}\n    return value\n"
        ));
        let env = ConstEnv::default();
        let evaluator = CompTimeEvaluator::new(&env, &file);
        assert_eq!(
            evaluator.eval_bind_call(&file.defs[&Intern::from_ref("compute")], &[ConstValue::ONE]),
            None,
            "{assignment}"
        );
    }
}

#[test]
fn a_failed_compound_operation_restores_the_parent_call_stack() {
    let file = parse("compute(input):\n    value: input\n    value/: 0\n    return value\n");
    let env = ConstEnv::default();
    let parent = parent();
    let mut stack = HashSet::from([parent.clone()]);
    let mut frame = EvalFrame::new(&env, &file, None, None, &mut stack);
    assert_eq!(
        frame.eval_bind_call(
            &file.defs[&Intern::from_ref("compute")],
            &[ConstValue::ONE],
            0
        ),
        None
    );
    assert_eq!(frame.call_stack.len(), 1);
    assert!(frame.call_stack.contains(&parent));
}

#[test]
fn compound_assignments_preserve_text_and_float_binary_semantics() {
    for (body, expected) in [
        (
            "value: \"a\"\n    value+: \"b\"",
            ConstValue::Text("ab".into()),
        ),
        (
            "value: 1.5\n    value*: 2.0",
            ConstValue::Float(HashFloat(3.0)),
        ),
    ] {
        let file = parse(&format!("compute(input):\n    {body}\n    return value\n"));
        let env = ConstEnv::default();
        let evaluator = CompTimeEvaluator::new(&env, &file);
        assert_eq!(
            evaluator.eval_bind_call(&file.defs[&Intern::from_ref("compute")], &[ConstValue::ONE]),
            Some(expected)
        );
    }
}
