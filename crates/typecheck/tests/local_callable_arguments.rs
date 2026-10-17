mod support;

use support::check_lexed;

#[test]
fn local_calls_accept_values_in_the_parameter_domain() {
    for arguments in ["0", "1"] {
        let typed = check_lexed(&format!(
            "outer():\n    local(value is in 0 to 1): return value\n    return local({arguments})\n"
        ));
        assert!(
            typed.all_flaws().is_empty(),
            "{arguments}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn local_calls_reject_values_outside_the_parameter_domain() {
    for arguments in ["2", "-1"] {
        let typed = check_lexed(&format!(
            "outer():\n    local(value is in 0 to 1): return value\n    return local({arguments})\n"
        ));
        assert!(
            typed.all_flaws().iter().any(|(_, flaw)| matches!(
                flaw.code.slug().as_str(),
                "type-out-of-range" | "type-integer-literal-out-of-range"
            )),
            "accepted {arguments}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn local_calls_reject_wrong_argument_counts() {
    for arguments in ["", "0, 1"] {
        let typed = check_lexed(&format!(
            "outer():\n    local(value is in 0 to 1): return value\n    return local({arguments})\n"
        ));
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.code.slug() == "type-call-argument-count"),
            "accepted {arguments:?}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn local_calls_use_their_contract_when_a_global_callable_has_the_same_name() {
    let typed = check_lexed(
        "local(value is in 0 to 0): return value\nouter():\n    local(value is in 0 to 1): return value\n    return local(1)\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn local_calls_reject_values_allowed_by_a_shadowed_global_contract() {
    let typed = check_lexed(
        "local(value is in 0 to 3): return value\nouter():\n    local(value is in 0 to 1): return value\n    return local(2)\n",
    );
    assert!(
        typed.all_flaws().iter().any(|(_, flaw)| matches!(
            flaw.code.slug().as_str(),
            "type-out-of-range" | "type-integer-literal-out-of-range"
        )),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn named_local_arguments_follow_parameter_order() {
    let typed = check_lexed(
        "outer():\n    local(low is in 0 to 1, high is in 10 to 11): return low\n    return local(high: 10, low: 1)\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let args = typed
        .exprs
        .kind
        .iter()
        .find_map(|kind| match kind {
            typecheck::TypedExprKind::FnCall {
                target,
                args: Some(args),
                ..
            } if target.0.as_str() == "local" => Some(args),
            _ => None,
        })
        .expect("local call");
    assert_eq!(args.len(), 2);
    for (argument, expected) in args.iter().zip([1_i128, 10]) {
        assert_eq!(
            typed.exprs.const_value_of(*argument),
            Some(&Some(ast::ConstValue::Int(expected.into())))
        );
    }
}

#[test]
fn local_calls_reject_duplicate_and_unknown_names() {
    for arguments in ["value: 0, value: 1", "missing: 0"] {
        let typed = check_lexed(&format!(
            "outer():\n    local(value is in 0 to 1): return value\n    return local({arguments})\n"
        ));
        assert!(
            typed.all_flaws().iter().any(|(_, flaw)| matches!(
                flaw.code.slug().as_str(),
                "type-call-argument-name" | "type-call-argument-count"
            )),
            "{arguments}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn local_calls_materialize_omitted_defaults() {
    for arguments in ["", "7", "value: 7"] {
        let typed = check_lexed(&format!(
            "outer():\n    local(value: 7): return value\n    return local({arguments})\n"
        ));
        assert!(
            typed.all_flaws().is_empty(),
            "{arguments}: {:?}",
            typed.all_flaws()
        );
        let args = typed
            .exprs
            .kind
            .iter()
            .find_map(|kind| match kind {
                typecheck::TypedExprKind::FnCall {
                    target,
                    args: Some(args),
                    ..
                } if target.0.as_str() == "local" => Some(args),
                _ => None,
            })
            .expect("local call");
        assert_eq!(args.len(), 1);
        assert_eq!(
            typed.exprs.const_value_of(args[0]),
            Some(&Some(ast::ConstValue::Int(7_i128.into())))
        );
    }
}

#[test]
fn local_calls_accept_guarded_integer_domains() {
    let typed = check_lexed(
        "outer(value is in 0 to 255):\n    local(small is in 0 to 1): return small\n    when value is in 0 to 1 then\n        return local(value)\n    return 0\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}
