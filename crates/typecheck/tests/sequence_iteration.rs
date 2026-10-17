mod support;

use internment::Intern;

use ast::Ty;
use support::{check_unflawed, evaluate, flaw_codes};
use typecheck::{SequenceTraversal, TypedExprKind, TypedLoopKind};

#[test]
fn fixed_sequences_include_named_heterogeneous_empty_and_nested_shapes() {
    for source in [
        "main:\n    for value in [1, 2, 3]\n        observed: value\n    return 0\n",
        "main:\n    for value in [first: 1, second: 2]\n        observed: value\n    return 0\n",
        "Flag is On or Off\nmain:\n    for value in [1, On]\n        observed: value\n    return 0\n",
        "main:\n    for value in []\n        observed: value\n    return 0\n",
        "main:\n    for value in [[1, 2], [3]]\n        observed: value\n    return 0\n",
    ] {
        let typed = check_unflawed(source);
        assert!(
            flaw_codes(&typed).is_empty(),
            "{source}: {:?}",
            typed.all_flaws()
        );
        assert!(
            typed.exprs.kind.iter().any(|kind| matches!(
                kind,
                TypedExprKind::Loop(typecheck::TypedLoop {
                    kind: TypedLoopKind::ForIn {
                        traversal: SequenceTraversal::Fixed { .. },
                        ..
                    },
                    ..
                })
            )),
            "{source}"
        );
    }
}

#[test]
fn heterogeneous_positions_receive_distinct_binders_and_types() {
    let typed = check_unflawed(
        "Flag is On or Off\nmain:\n    for value in [1, On]\n        saved: value\n    return 0\n",
    );
    assert!(flaw_codes(&typed).is_empty(), "{:?}", typed.all_flaws());
    let iterations = typed
        .exprs
        .kind
        .iter()
        .find_map(|kind| match kind {
            TypedExprKind::Loop(typecheck::TypedLoop {
                kind:
                    TypedLoopKind::ForIn {
                        traversal: SequenceTraversal::Fixed { iterations },
                        ..
                    },
                ..
            }) => Some(iterations),
            _ => None,
        })
        .unwrap();
    assert_eq!(iterations.len(), 2);
    let first = iterations[0][0];
    let second = iterations[1][0];
    assert_ne!(typed.exprs.place_of(first), typed.exprs.place_of(second));
    assert_ne!(typed.exprs.ty_of(first), typed.exprs.ty_of(second));
    assert!(matches!(
        typed.exprs.ty_of(first),
        Some(Ty::AnonymousInteger { .. })
    ));
}

#[test]
fn heterogeneous_body_resolves_each_elements_method() {
    let typed = check_unflawed(&format!(
        "{}First has\n    score(self) Word: return 1\nSecond has\n    score(self) Word: return 2\nmain:\n    for value in [First(), Second()]\n        score: value.score\n    return 0\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(flaw_codes(&typed).is_empty(), "{:?}", typed.all_flaws());
    for name in ["First.score", "Second.score"] {
        assert!(
            typed.exprs.kind.iter().any(|kind| matches!(kind,
                TypedExprKind::FnCall { target, .. } if target.0.as_str() == name
            )),
            "missing {name}"
        );
    }
}

#[test]
fn body_must_be_valid_for_each_element() {
    let typed = check_unflawed(&format!(
        "{}First has\n    score(self) Word: return 1\nSecond has\n    finish(self):\n        return\nmain:\n    for value in [First(), Second()]\n        score: value.score\n    return 0\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(
        flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-unknown-field" || code == "type-unknown-symbol"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn singleton_traversal_consumes_a_capture_once() {
    let typed = check_unflawed(
        "Token has\n    finish(self):\n        return\nmain:\n    token: Token()\n    for value in [1]\n        token.finish\n    return 0\n",
    );
    assert!(flaw_codes(&typed).is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn repeated_traversal_rejects_reusing_a_consumed_capture() {
    let typed = check_unflawed(
        "Token has\n    finish(self):\n        return\nmain:\n    token: Token()\n    for value in [1, 2]\n        token.finish\n    return 0\n",
    );
    assert!(
        flaw_codes(&typed).iter().any(|code| code.contains("move")),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn empty_traversal_does_not_consume_a_capture() {
    let typed = check_unflawed(
        "Token has\n    finish(self):\n        return\nmain:\n    token: Token()\n    for value in []\n        token.finish\n    token.finish\n    return 0\n",
    );
    assert!(flaw_codes(&typed).is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn each_position_can_transfer_its_owned_element() {
    let typed = check_unflawed(
        "Token has\n    finish(self):\n        return\nmain:\n    values: [Token(), Token()]\n    for value in values\n        value.finish\n    return 0\n",
    );
    assert!(flaw_codes(&typed).is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn unsupported_source_reports_a_sequence_requirement() {
    let typed =
        check_unflawed("main:\n    for value in 1\n        observed: value\n    return 0\n");
    assert!(
        flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-for-requires-sequence"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn compile_time_sequence_traversal_preserves_rebindings_and_order() {
    let source = format!(
        "{}replace(previous Word, next Word) Word: return next\nrun() Word:\n    total Word: 0\n    for value in [first: 1, second: 2, third: 3]\n        previous is in 0 to 255: total\n        digit is in 0 to 255: value\n        total:: replace(total, (previous * 10 + digit) as Word)\n    return total\n",
        test_fixtures::preludes::WORD_PRELUDE
    );
    assert_eq!(evaluate(&source), ast::ConstValue::Int(123.into()));
}

#[test]
fn sequence_element_narrowing_checks_each_initialized_value() {
    for (values, valid) in [("[1, 2, 255]", true), ("[1, 256, 3]", false)] {
        let typed = check_unflawed(&format!(
            "run:\n    for value in {values}\n        digit is in 0 to 255: value\n    return\n"
        ));
        assert_eq!(
            !flaw_codes(&typed)
                .iter()
                .any(|code| code.starts_with("type-integer-narrowing-")),
            valid,
            "{values}: {:?}",
            typed.all_flaws()
        );
        if valid {
            assert!(flaw_codes(&typed).is_empty(), "{:?}", typed.all_flaws());
        }
    }
}

#[test]
fn sequence_element_narrowing_does_not_reuse_values_before_mutation() {
    for update in ["change(values)", "values.[0]:: 256", "values:: [256]"] {
        let source = format!(
            "change(values mut [in 0 to 65535; 1]):\n    values.[0]:: 256\n    return\nrun:\n    values [in 0 to 65535; 1]: [1]\n    for value in values\n        digit is in 0 to 255: value\n    {update}\n    for value in values\n        digit is in 0 to 255: value\n    return\n"
        );
        let typed = check_unflawed(&source);
        let flaws = typed.all_flaws();
        let narrowing = flaws
            .iter()
            .filter(|(_, flaw)| flaw.code.slug().starts_with("type-integer-narrowing-"))
            .collect::<Vec<_>>();
        assert!(
            !narrowing.is_empty()
                && narrowing
                    .iter()
                    .all(|(_, flaw)| flaw.span.start() == source.rfind("digit").unwrap()),
            "{update}: {:?}",
            flaws
        );
    }
}

#[test]
fn compile_time_integer_range_does_not_materialize_before_an_early_return() {
    let source = "run() is in 0 to 255:\n    for value in 0 to 1000000000\n        when value is 0 then\n            return 42\n    return 99\n";
    assert_eq!(evaluate(source), ast::ConstValue::Int(42.into()));
}

#[test]
fn compile_time_range_body_still_spends_fuel() {
    let typed = check_unflawed(
        "run():\n    for value in 0 to 1000000000\n        observed: value\n    return\n",
    );
    let failure = typecheck::Evaluator::new(
        typed.resolved_program(),
        typecheck::CompileTimeHost::new(&flask::CompileTarget::Library),
    )
    .call(
        typecheck::DefId::named(internment::Intern::from_ref("run")),
        &[],
    )
    .unwrap_err();
    assert_eq!(failure.code.slug().as_str(), "compile-time-fuel-exhausted");
}

#[test]
fn compile_time_range_preserves_an_outer_binding() {
    let source = "run is in 0 to 255:\n    value is in 0 to 255: 42\n    for item in 0 to 3\n        observed: item\n    return value\n";
    assert_eq!(evaluate(source), ast::ConstValue::Int(42.into()));
}

#[test]
fn sequence_elements_preserve_reference_targets() {
    let source = format!(
        "{}get(value ref{{a}} Word) selected ref{{a}} Word: return value\nread(value ref Word) Word: return deref value\nmain:\n    first Word: 17\n    second Word: 25\n    for value in [get(first).selected, get(second).selected]\n        read(value)\n    first.finish\n    second.finish\n    return\n",
        test_fixtures::preludes::WORD_PRELUDE
    );
    let typed = check_unflawed(&source);
    assert!(flaw_codes(&typed).is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn sequence_element_references_are_invalidated_by_consuming_their_targets() {
    let source = format!(
        "{}get(value ref{{a}} Word) selected ref{{a}} Word: return value\nread(value ref Word) Word: return deref value\nmain:\n    target Word: 17\n    for value in [get(target).selected]\n        target.finish\n        read(value)\n    return\n",
        test_fixtures::preludes::WORD_PRELUDE
    );
    let typed = check_unflawed(&source);
    assert!(
        flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-use-of-invalidated-reference"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn compile_time_ranges_choose_direction_and_include_the_endpoint() {
    for (range, expected) in [
        ("2 to 5", 4),
        ("2 to 2", 1),
        ("5 to 2", 4),
        ("1 to 0", 2),
        ("0 to 1", 2),
        ("128 to 127", 2),
        ("-2 to 1", 4),
        ("1 to -2", 4),
        ("9223372036854775806 to 9223372036854775807", 2),
        ("-9223372036854775807 to -9223372036854775808", 2),
        ("18446744073709551615 to 18446744073709551613", 3),
        (
            "340282366920938463463374607431768211455 to 340282366920938463463374607431768211453",
            3,
        ),
    ] {
        let source = format!(
            "{}replace(previous Word, next Word) Word: return next\nrun Word:\n    total Word: 0\n    for value in {range}\n        previous is in Word: total\n        total:: replace(total, (previous + 1) as Word)\n    return total\n",
            test_fixtures::preludes::WORD_PRELUDE
        );
        let typed = check_unflawed(&source);
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let target = flask::CompileTarget::Library;
        let result = typecheck::eval::Evaluator::new(
            typed.resolved_program(),
            typecheck::eval::CompileTimeHost::new(&target),
        )
        .call(typecheck::DefId::named(Intern::from_ref("run")), &[])
        .unwrap();
        assert_eq!(result, ast::ConstValue::Int(expected.into()));
    }
}

#[test]
fn compile_time_descending_range_preserves_order_and_rebindings() {
    let source = format!(
        "{}replace(previous Word, next Word) Word: return next\nrun Word:\n    total Word: 0\n    for value in 3 to 0\n        previous is in Word: total\n        total:: replace(total, (previous * 4 + value) as Word)\n    return total\n",
        test_fixtures::preludes::WORD_PRELUDE
    );
    assert_eq!(evaluate(&source), ast::ConstValue::Int(228.into()));
}

#[test]
fn compile_time_descending_range_returns_without_materializing_the_range() {
    let source = "run is in 0 to 255:\n    for value in 1000000000 to 0\n        when value is 1000000000 then\n            return 42\n    return 99\n";
    assert_eq!(evaluate(source), ast::ConstValue::Int(42.into()));
}

#[test]
fn compile_time_runtime_bounds_select_direction_with_mixed_integer_domains() {
    let typed = check_unflawed(
        "run(start is in 0 to 255, end is in -128 to 127) is in 0 to 255:\n    for value in start to end\n        when value is in -1 to -1 then\n            return 42\n    return 99\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let result = typecheck::Evaluator::new(
        typed.resolved_program(),
        typecheck::CompileTimeHost::new(&flask::CompileTarget::Library),
    )
    .call(
        typecheck::DefId::named(Intern::from_ref("run")),
        &[
            ast::ConstValue::Int(1.into()),
            ast::ConstValue::Int((-2).into()),
        ],
    )
    .unwrap();
    assert_eq!(result, ast::ConstValue::Int(42.into()));
}

#[test]
fn range_values_use_the_same_direction_and_endpoints_as_loops() {
    for (range, values) in [
        ("0 to 3", vec![0, 1, 2, 3]),
        ("3 to 0", vec![3, 2, 1, 0]),
        ("1 to -2", vec![1, 0, -1, -2]),
        ("-2 to 1", vec![-2, -1, 0, 1]),
        ("3 to 3", vec![3]),
    ] {
        let typed = check_unflawed(&format!("run: {range}\n"));
        let range_expr = typed
            .exprs
            .kind
            .iter()
            .position(|kind| matches!(kind, TypedExprKind::Range { .. }))
            .unwrap();
        let result = typecheck::Evaluator::new(
            typed.resolved_program(),
            typecheck::CompileTimeHost::new(&flask::CompileTarget::Library),
        )
        .eval(typecheck::ExprId(range_expr as u32))
        .unwrap();
        let typecheck::EvalOutcome::Complete(result) = result else {
            panic!("{range}: {result:?}");
        };
        let expected = ast::ConstValue::Sequence(
            values
                .into_iter()
                .map(|value| ast::ConstValue::Int(value.into()))
                .collect(),
        );
        assert_eq!(result, expected, "{range}");
    }
}
