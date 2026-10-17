mod support;
use support::check_parsed;

#[test]
fn stored_comparisons_are_propositions_without_result_families() {
    for operator in ["=", "<", "<=", ">", ">="] {
        let typed = check_parsed(&format!(
            "run(value is in 0 to 255):\n    comparison: value {operator} 16\n    return when comparison then 1 else 0\n"
        ));
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        assert!(
            typed
                .exprs
                .ty
                .iter()
                .any(|ty| matches!(ty, ast::Ty::Proposition(_)))
        );
        assert!(
            typed
                .exprs
                .ty
                .iter()
                .all(|ty| !matches!(ty, ast::Ty::ResultFamily { .. }))
        );
    }
}

#[test]
fn stored_and_copied_propositions_refine_both_branches() {
    for guard in [
        "when small then\n        return accept(value)\n    return 0",
        "when not small then\n        return 0\n    return accept(value)",
        "when small and value >= 0 then\n        return accept(value)\n    return 0",
    ] {
        let typed = check_parsed(&format!(
            "accept(value is in 0 to 15) is in 0 to 15: return value\nrun(value is in 0 to 255):\n    original: value < 16\n    small: original\n    {guard}\n"
        ));
        assert!(
            typed.all_flaws().is_empty(),
            "{guard}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn propositions_have_a_predicate_shaped_source_type() {
    let source = "run(value is in 0 to 255):\n    small is value < 16: value < 16\n    return when small then 1 else 0\n";
    let typed = check_parsed(source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let wrong = check_parsed(&source.replace(": value < 16", ": value > 16"));
    assert!(!wrong.all_flaws().is_empty());
}

#[test]
fn named_functions_can_return_and_accept_propositions() {
    let typed = check_parsed(
        "less(left ref is in 0 to 255, right is in 0 to 255) is left < right: return left < right\nconsume(value is in 0 to 255, check is value < 16): return when check then 1 else 0\nrun(value is in 0 to 255):\n    small: less(value, 16)\n    return consume(value, small)\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn named_proposition_results_prove_numeric_bounds() {
    let typed = check_parsed(
        "less(left ref is in 0 to 255, right is in 0 to 255): return left < right\naccept(value is in 0 to 15) is in 0 to 15: return value\nrun(value is in 0 to 255):\n    small: less(value, 16)\n    when small then\n        return accept(value)\n    return 0\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn old_boolean_patterns_are_rejected_for_propositions() {
    let typed = check_parsed(
        "run(value is in 0 to 255):\n    small: value < 16\n    return when small is True then 1 else 0\n",
    );
    assert!(
        support::flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-proposition-pattern")
    );
}

#[test]
fn user_defined_booleans_keep_explicit_pattern_matching() {
    let typed = check_parsed(
        "Bool is True or False\nrun(flag Bool): return when flag is True then 1 else 0\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    for source in [
        "run(flag is in 0 to 1): return when flag then 1 else 0\n",
        "Bool is True or False\nrun(flag Bool): return when flag then 1 else 0\n",
    ] {
        let typed = check_parsed(source);
        assert!(
            support::flaw_codes(&typed)
                .iter()
                .any(|code| code == "type-condition-requires-proposition")
        );
    }
}

#[test]
fn captured_propositions_do_not_refine_mutated_values() {
    let typed = check_parsed(
        "grow(value mut is in 0 to 255):\n    value :: 200\n    return\naccept(value is in 0 to 15) is in 0 to 15: return value\nrun(value is in 0 to 255):\n    small: value < 16\n    grow(value)\n    when small then\n        return accept(value)\n    return 0\n",
    );
    assert!(
        support::flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-parameter-refinement-unproven"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn typed_proposition_parameters_prove_bounds() {
    let typed = check_parsed(
        "accept(value is in 0 to 15) is in 0 to 15: return value\nconsume(value is in 0 to 255, small is value < 16):\n    when small then\n        return accept(value)\n    return 0\nrun(value is in 0 to 255):\n    small: value < 16\n    return consume(value, small)\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn proposition_arguments_must_test_the_supplied_values() {
    for comparison in ["value > 16", "other < 16"] {
        let typed = check_parsed(&format!(
            "consume(value is in 0 to 255, check is value < 16): return when check then 1 else 0\nrun(value is in 0 to 255, other is in 0 to 255):\n    check: {comparison}\n    return consume(value, check)\n"
        ));
        assert!(
            support::flaw_codes(&typed)
                .iter()
                .any(|code| code == "type-proposition-mismatch"
                    || code == "type-proposition-argument-unproven"),
            "{comparison}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn typed_proposition_parameters_do_not_refine_changed_values() {
    let typed = check_parsed(
        "grow(value mut is in 0 to 255):\n    value :: 200\n    return\naccept(value is in 0 to 15) is in 0 to 15: return value\nconsume(value mut is in 0 to 255, small is value < 16):\n    grow(value)\n    when small then\n        copy is in 0 to 255: value\n        return accept(copy)\n    return 0\n",
    );
    assert!(
        support::flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-parameter-refinement-unproven"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn stale_proposition_arguments_are_rejected() {
    let typed = check_parsed(
        "grow(value mut is in 0 to 255):\n    value :: 200\n    return\nconsume(value ref is in 0 to 255, check is value < 16): return when check then 1 else 0\nrun(value mut is in 0 to 255):\n    small: value < 16\n    grow(value)\n    return consume(value, small)\n",
    );
    assert!(
        support::flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-proposition-argument-unproven"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn constant_propositions_keep_their_predicate_type() {
    let typed = check_parsed("small is 1 < 16 := 1 < 16\nrun: when small then 1 else 0\n");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn every_return_path_must_describe_the_same_proposition() {
    for signature in ["", " is left < 16"] {
        let typed = check_parsed(&format!(
            "choose(flag is in 0 to 1, left is in 0 to 255, right is in 0 to 255){signature}:\n    when flag = 1 then\n        return left < 16\n    return right < 16\n"
        ));
        assert!(
            support::flaw_codes(&typed)
                .iter()
                .any(|code| code == "type-proposition-mismatch"),
            "{:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn a_mutating_comparison_cannot_prove_its_incoming_argument() {
    let typed = check_parsed(
        "reset_and_check(value mut is in 0 to 255):\n    value :: 0\n    return value < 16\naccept(value is in 0 to 15) is in 0 to 15: return value\nrun(value mut is in 0 to 255):\n    before is in 0 to 255: value\n    small: reset_and_check(value)\n    when small then\n        return accept(before)\n    return 0\n",
    );
    assert!(
        support::flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-parameter-refinement-unproven"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn local_callable_proposition_parameters_refine_and_check_arguments() {
    let source = "accept(value is in 0 to 15) is in 0 to 15: return value\nrun(value is in 0 to 255):\n    consume(item is in 0 to 255, small is item < 16):\n        when small then\n            return accept(item)\n        return 0\n    small: value < 16\n    return consume(value, small)\n";
    let typed = check_parsed(source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let stale = check_parsed(&source.replace(
        "    small: value < 16",
        "    small: value < 16\n    value :: 200",
    ));
    assert!(
        support::flaw_codes(&stale)
            .iter()
            .any(|code| code == "type-proposition-argument-unproven"),
        "{:?}",
        stale.all_flaws()
    );
}
