mod support;

use support::check_parsed;

#[test]
fn numeric_guards_do_not_construct_result_families() {
    for condition in [
        "value = 3",
        "value < 16",
        "value <= 15",
        "value > 239",
        "value >= 240",
        "value in 0 to 15",
        "value in 0 to 15 and value < 16",
        "value < 16 or value in 240 to 255",
        "not (value >= 16)",
        "value >= 0 and value < 16",
        "value < 16 or value > 239",
        "(value + 0) < 16",
        "(value < 16)",
    ] {
        let typed = check_parsed(&format!(
            "choose(value is in 0 to 255): return when {condition} then 1 else 0\n"
        ));
        assert!(
            typed.all_flaws().is_empty(),
            "{condition}: {:?}",
            typed.all_flaws()
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
fn successful_and_fallthrough_predicates_prove_refinements() {
    for guard in [
        "when value < 16 then\n        return accept(value)\n    return 0",
        "when value >= 16 then\n        return 0\n    return accept(value)",
        "when not (value >= 16) then\n        return accept(value)\n    return 0",
        "when value in 0 to 15 then\n        return accept(value)\n    return 0",
    ] {
        let typed = check_parsed(&format!(
            "accept(value is in 0 to 255 and < 16) is in 0 to 255: return value\nrun(value is in 0 to 255) is in 0 to 255:\n    {guard}\n"
        ));
        assert!(
            typed.all_flaws().is_empty(),
            "{guard}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn named_comparison_contracts_use_numeric_predicates() {
    let typed = check_parsed(
        "less(left ref is in 0 to 255, right is in 0 to 255) True thus left < right or False thus left >= right:\n    when left < right then\n        return True\n    return False\naccept(value is in 0 to 255 and < 16) is in 0 to 255: return value\nrun(value is in 0 to 255) is in 0 to 255:\n    when less(value, 16) is True then\n        return accept(value)\n    return 0\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let invalid = check_parsed(
        "less(left ref is in 0 to 255, right is in 0 to 255) True thus left < right or False thus left >= right: return True\n",
    );
    assert!(!invalid.all_flaws().is_empty());
}

#[test]
fn numeric_operators_cannot_be_overloaded() {
    for operator in ["=", "<", "<=", ">", ">="] {
        let typed = check_parsed(&format!(
            "#infix('{operator}')\ncustom(left is in 0 to 255, right is in 0 to 255) Before or After: return Before\n"
        ));
        assert!(
            support::flaw_codes(&typed)
                .iter()
                .any(|code| code == "type-fixed-numeric-comparison")
        );
    }
}

#[test]
fn named_custom_orderings_remain_ordinary_pattern_matches() {
    let typed = check_parsed(
        "comes_before(left, right) Before or After: return Before\nrun: when comes_before(200, 16) is Before then 1 else 0\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn non_numeric_values_cannot_supply_numeric_predicates() {
    let typed = check_parsed("run: when 'a' < 'b' then 1 else 0\n");
    assert!(
        support::flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-numeric-predicate-operands")
    );
}

#[test]
fn nominal_domains_compare_without_operator_implementations() {
    let typed = check_parsed(
        "Count is in 0 to 255\nCount has release(self): return\nchoose(value ref Count): return when value < 256 then 1 else 0\ncompare(value ref Count, other is in -128 to 127): return when value > other then 1 else 0\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn a_numeric_guard_proves_a_dependent_parameter_refinement() {
    let declarations = include_str!("../../test-fixtures/fixtures/runtime_bounded_lookup.gin");
    let typed = check_parsed(&format!(
        "{declarations}\nread(view ref View(Byte), index Count) Byte:\n    when index < view.length then\n        return at(view, index)\n    return 0\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}
