mod support;

use ast::ConstValue;
use support::{body, check};

#[test]
fn anonymous_comparison_literals_preserve_values_beyond_signed_64_bits() {
    for (domain, value) in [
        ("0 to 18446744073709551615", "18446744073709551615"),
        (
            "0 to 340282366920938463463374607431768211455",
            "340282366920938463463374607431768211455",
        ),
        (
            "-170141183460469231731687303715884105728 to 170141183460469231731687303715884105727",
            "-170141183460469231731687303715884105728",
        ),
    ] {
        for operator in ["<", "<=", "=", ">=", ">"] {
            for expression in [
                format!("input {operator} {value}"),
                format!("{value} {operator} input"),
            ] {
                let source = format!("compare(input is in {domain}): return {expression}\n");
                let typed = check(&source);
                assert!(
                    typed.all_flaws().is_empty(),
                    "{source}: {:?}",
                    typed.all_flaws()
                );
            }
        }
        let source = format!("input is in {domain} := {value}\nresult := input = {value}\n");
        let typed = check(&source);
        assert!(
            typed.all_flaws().is_empty(),
            "{source}: {:?}",
            typed.all_flaws()
        );
        let Some(ConstValue::Proposition(holds)) =
            &typed.exprs.facts.const_facts.const_value[body(&typed, "result").as_usize()]
        else {
            panic!("expected a constant comparison: {source}");
        };
        assert!(*holds);
    }
}

#[test]
fn constant_comparisons_preserve_both_operands_numeric_values() {
    for (left_type, right_type, left, right, operator, expected) in [
        ("0 to 255", "-128 to 127", "0", "-1", ">", 1),
        ("0 to 255", "-128 to 127", "255", "-1", "=", 0),
        ("-128 to 127", "0 to 255", "-1", "255", "<", 1),
        ("-128 to 127", "0 to 255", "127", "255", ">", 0),
        ("-128 to 127", "0 to 255", "127", "127", "=", 1),
        (
            "-9223372036854775808 to 9223372036854775807",
            "0 to 18446744073709551615",
            "-1",
            "18446744073709551615",
            "<",
            1,
        ),
        (
            "0 to 340282366920938463463374607431768211455",
            "-170141183460469231731687303715884105728 to 170141183460469231731687303715884105727",
            "340282366920938463463374607431768211455",
            "-1",
            ">",
            1,
        ),
    ] {
        let source = format!(
            "left is in {left_type} := {left}\nright is in {right_type} := {right}\nresult := left {operator} right\n"
        );
        let typed = check(&source);
        assert!(
            typed.all_flaws().is_empty(),
            "{source}: {:?}",
            typed.all_flaws()
        );
        let constant =
            &typed.exprs.facts.const_facts.const_value[body(&typed, "result").as_usize()];
        let Some(ConstValue::Proposition(holds)) = constant else {
            panic!("expected a comparison result: {constant:?}\n{source}");
        };
        assert_eq!(*holds, expected == 1, "{source}");
    }
}
