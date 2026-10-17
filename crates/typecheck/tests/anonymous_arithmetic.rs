mod support;

use ast::ConstValue;
use internment::Intern;
use parser::query::SourceParseExt;
use typecheck::eval::{CompileTimeHost, Evaluator};
use typecheck::staging::{NormalizeResult, normalize};
use typecheck::subst::DepSubst;
use typecheck::{DefId, TypedFileAst};

fn compute(typed: &TypedFileAst, left: i128, right: i128) -> ConstValue {
    let target = flask::CompileTarget::Library;
    Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target))
        .call(
            DefId::named(Intern::from_ref("compute")),
            &[ConstValue::Int(left.into()), ConstValue::Int(right.into())],
        )
        .unwrap()
}

#[test]
fn unequal_operand_widths_preserve_mathematical_products() {
    for expression in ["left * right", "right * left"] {
        let typed = support::check_no_flaws(&format!(
            "compute(left in 1 to 10, right in 0 to 10000): return {expression}\n"
        ));
        for (left, right, expected) in [(3, 125, 375), (10, 10000, 100000), (1, 0, 0)] {
            assert_eq!(
                compute(&typed, left, right),
                ConstValue::Int(expected.into())
            );
        }
        let result = &typed.defs[&DefId::named(Intern::from_ref("compute"))].return_type;
        let hull = result
            .anonymous_integer_validity()
            .unwrap()
            .domain()
            .storage_hull()
            .unwrap();
        assert_eq!((hull.min(), hull.max()), (0.into(), 100000.into()));
        assert_eq!(result.anonymous_integer_width(), Some(17));
    }
}

#[test]
fn signed_arithmetic_preserves_values_and_result_bounds() {
    for (operator, left, right, expected, min, max) in [
        ("+", -3, -125, -128, -10010, 10010),
        ("-", 3, 125, -122, -10010, 10010),
        ("*", -3, -125, 375, -100000, 100000),
        ("*", -3, 125, -375, -100000, 100000),
        ("*", 3, -125, -375, -100000, 100000),
    ] {
        let typed = support::check_no_flaws(&format!(
            "compute(left in -10 to 10, right in -10000 to 10000): return left {operator} right\n"
        ));
        assert_eq!(
            compute(&typed, left, right),
            ConstValue::Int(expected.into())
        );
        let result = &typed.defs[&DefId::named(Intern::from_ref("compute"))].return_type;
        let hull = result
            .anonymous_integer_validity()
            .unwrap()
            .domain()
            .storage_hull()
            .unwrap();
        assert_eq!((hull.min(), hull.max()), (min.into(), max.into()));
    }
}

#[test]
fn subtraction_and_addition_expand_beyond_both_operand_representations() {
    for (operator, left, right, expected, min, max) in
        [("+", 250, 10, 260, 0, 510), ("-", 0, 255, -255, -255, 255)]
    {
        let typed = support::check_no_flaws(&format!(
            "compute(left in 0 to 255, right in 0 to 255): return left {operator} right\n"
        ));
        assert_eq!(
            compute(&typed, left, right),
            ConstValue::Int(expected.into())
        );
        let result = &typed.defs[&DefId::named(Intern::from_ref("compute"))].return_type;
        let hull = result
            .anonymous_integer_validity()
            .unwrap()
            .domain()
            .storage_hull()
            .unwrap();
        assert_eq!((hull.min(), hull.max()), (min.into(), max.into()));
        assert_eq!(result.anonymous_integer_width(), Some(9));
    }
}

#[test]
fn known_operands_keep_precise_bounds_inside_runtime_functions() {
    for (expression, min, max) in [
        ("left + 1", 1, 11),
        ("1 + left", 1, 11),
        ("left * 2", 0, 20),
        ("left - 1", -1, 9),
        ("50 + 50", 100, 100),
    ] {
        let typed =
            support::check_no_flaws(&format!("compute(left in 0 to 10): return {expression}\n"));
        let result = &typed.defs[&DefId::named(Intern::from_ref("compute"))].return_type;
        let hull = result
            .anonymous_integer_validity()
            .unwrap()
            .domain()
            .storage_hull()
            .unwrap();
        assert_eq!(
            (hull.min(), hull.max()),
            (min.into(), max.into()),
            "{expression}"
        );
    }
}

#[test]
fn normalization_agrees_with_evaluation_after_parameter_substitution() {
    let mut typed =
        support::check_no_flaws("compute(left in 1 to 10, right in 0 to 10000): left * right\n");
    typed.exprs.facts.const_facts.const_value.fill(None);
    let root = support::body(&typed, "compute");
    let mut substitutions = DepSubst::new();
    substitutions
        .consts
        .insert(Intern::from_ref("left"), 3.into());
    substitutions
        .consts
        .insert(Intern::from_ref("right"), 125.into());
    assert_eq!(
        normalize(&typed, root, &substitutions),
        NormalizeResult::Value(compute(&typed, 3, 125)),
    );
}

#[test]
fn arithmetic_domains_that_exceed_256_bits_are_rejected_instead_of_wrapping() {
    let maximum = u128::MAX;
    let typed = support::check(&format!(
        "compute(left in 0 to {maximum}, right in 0 to {maximum}): return left * right\n"
    ));
    assert!(
        support::flaw_codes(&typed).contains(&"type-integer-arithmetic-overflow".to_string()),
        "{:?}",
        typed.all_flaws(),
    );
}

#[test]
fn arithmetic_cannot_narrow_to_an_operand_width_without_a_proof() {
    let typed = support::check(concat!(
        "Byte is in 0 to 255\nByte has finish(self): return\n",
        "compute(left in 0 to 255, right in 0 to 255) Byte:\n",
        "    return (left + right) as Byte\n",
    ));
    assert!(
        support::flaw_codes(&typed).contains(&"type-integer-narrowing-failed".to_string()),
        "{:?}",
        typed.all_flaws(),
    );
}

#[test]
fn division_and_remainder_preserve_mathematical_values_and_bounds() {
    for (left_domain, right_domain, operator, left, right, expected, min, max) in [
        ("0 to 255", "-10 to -1", "/", 255, -3, -85, -255, 0),
        ("-128 to 127", "-1 to -1", "/", -128, -1, 128, -127, 128),
        (
            "-10000 to 10000",
            "1 to 10",
            "/",
            -10000,
            3,
            -3333,
            -10000,
            10000,
        ),
        ("-10000 to 0", "10 to 10", "%", -123, 10, -3, -9, 0),
        ("0 to 10000", "-10 to -1", "%", 123, -10, 3, 0, 9),
        ("-128 to 127", "-1 to -1", "%", -128, -1, 0, 0, 0),
    ] {
        let typed = support::check_no_flaws(&format!(
            "compute(left in {left_domain}, right in {right_domain}): return left {operator} right\n"
        ));
        assert_eq!(
            compute(&typed, left, right),
            ConstValue::Int(expected.into())
        );
        let result = &typed.defs[&DefId::named(Intern::from_ref("compute"))].return_type;
        let hull = result
            .anonymous_integer_validity()
            .unwrap()
            .domain()
            .storage_hull()
            .unwrap();
        assert_eq!(
            (hull.min(), hull.max()),
            (min.into(), max.into()),
            "{left_domain} {operator} {right_domain}"
        );
    }
}

#[test]
fn division_and_remainder_require_nonzero_divisors() {
    for operator in ["/", "%"] {
        for right in ["right", "0"] {
            let typed = support::check(&format!(
                "compute(left in -10 to 10, right in -10 to 10): return left {operator} {right}\n"
            ));
            assert!(
                support::flaw_codes(&typed)
                    .contains(&"type-integer-divisor-may-be-zero".to_string()),
                "{:?}",
                typed.all_flaws()
            );
        }
        let typed = support::check_no_flaws(&format!(
            "Nonzero is in -10 to 10 and /= 0\nNonzero has finish(self): return\ncompute(left in -100 to 100, right is in Nonzero): return left {operator} right\n"
        ));
        assert_eq!(
            compute(&typed, -100, -3),
            ConstValue::Int(if operator == "/" { 33 } else { -1 }.into())
        );
    }
}

#[test]
fn decimal_digit_remainder_proves_byte_conversion_without_wrapping() {
    let typed = support::check_no_flaws(concat!(
        "Byte is in 0 to 255\nByte has finish(self): return\n",
        "compute(left in -9223372036854775808 to 0, right in 10 to 10) Byte:\n",
        "    return (48 - (left % right)) as Byte\n",
    ));
    for (input, expected) in [(0, 48), (-123, 51), (i64::MIN, 56)] {
        assert_eq!(
            compute(&typed, input.into(), 10),
            ConstValue::Int(expected.into())
        );
    }
}

#[test]
fn guarded_divisors_are_proven_nonzero_until_mutated() {
    for operator in ["/", "%"] {
        let typed = support::check_no_flaws(&format!(
            "compute(left in -100 to 100, right in -10 to 10):\n    when right = 0 then return 0\n    return left {operator} right\n"
        ));
        assert_eq!(compute(&typed, -100, 0), ConstValue::Int(0.into()));
        assert_eq!(
            compute(&typed, -100, -3),
            ConstValue::Int(if operator == "/" { 33 } else { -1 }.into())
        );

        let typed = support::check(&format!(
            "compute(left in -100 to 100, right in -10 to 10):\n    divisor is in -10 to 10: right\n    when divisor = 0 then return 0\n    divisor :: 0\n    return left {operator} divisor\n"
        ));
        assert!(
            support::flaw_codes(&typed).contains(&"type-integer-divisor-may-be-zero".to_string()),
            "{:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn guarded_minimum_overflow_proves_the_remaining_signed_division() {
    let source = format!(
        "{}\n{}",
        include_str!("../../../modules/core/range/range.gin"),
        concat!(
            "Signed is in -128 to 127\nSigned has finish(self): return\n",
            "compute(left is in Signed, right is in Signed) Signed:\n",
            "    when right = 0 then return 0\n",
            "    when right = -1 then\n",
            "        when left > Range(domain of Signed).min then return (0 - left) as Signed\n",
            "        return 0\n",
            "    return (left / right) as Signed\n",
        ),
    );
    let typed = support::check_no_flaws(&source);
    for (left, right, expected) in [
        (-128, -1, 0),
        (-127, -1, 127),
        (-128, 1, -128),
        (-128, -2, 64),
        (127, 2, 63),
    ] {
        assert_eq!(
            compute(&typed, left, right),
            ConstValue::Int(expected.into())
        );
    }
}

#[test]
fn excluded_divisors_tighten_bounds_at_both_sides_of_zero() {
    let source = concat!(
        "negative_one := -1\nDivisor is in -10 to 10 and /= negative_one and /= 0 and /= 1\nDivisor has finish(self): return\n",
        "compute(left in -128 to 127, right is in Divisor): return left / right\n",
    );
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut ast = parsed.ast;
    assert!(typecheck::prepare_parse_ast(&mut ast, &flask::CompileTarget::Library).is_empty());
    let typed = typecheck::transform::transform(
        &ast,
        typecheck::FileId(0),
        &typecheck::transform::TransformCtx::new(),
    );
    support::expect_no_flaws(&typed);
    let result = &typed.defs[&DefId::named(Intern::from_ref("compute"))].return_type;
    let hull = result
        .anonymous_integer_validity()
        .unwrap()
        .domain()
        .storage_hull()
        .unwrap();
    assert_eq!((hull.min(), hull.max()), ((-64).into(), 64.into()));
    assert_eq!(compute(&typed, -128, -2), ConstValue::Int(64.into()));
}

#[test]
fn allocator_product_guard_proves_narrowing_without_wrapping() {
    let typed = support::check_no_flaws(concat!(
        "MemorySize is in 0 to 18446744073709551615\nMemorySize has finish(self): return\n",
        "compute(left in 0 to 9223372036854775807, right in 0 to 9223372036854775807) MemorySize:\n",
        "    requested: left * right\n",
        "    when requested > 9223372036854775807 then return 0\n",
        "    return requested as MemorySize\n",
    ));
    for (left, right, expected) in [
        (i64::MAX, 1, i64::MAX),
        (i64::MAX, 2, 0),
        (0, i64::MAX, 0),
        (17, 8, 136),
    ] {
        assert_eq!(
            compute(&typed, left.into(), right.into()),
            ConstValue::Int(expected.into())
        );
    }
}

#[test]
fn left_shift_preserves_mathematical_values_and_rejects_unproven_counts() {
    for (left_domain, right_domain, left, right, expected) in [
        ("1 to 1", "0 to 63", 1_i128, 63_i128, 1_i128 << 63),
        ("-1 to 1", "0 to 63", -1, 63, i64::MIN.into()),
        ("0 to 0", "0 to 255", 0, 255, 0),
    ] {
        let typed = support::check_no_flaws(&format!(
            "compute(left in {left_domain}, right in {right_domain}): return left << right\n"
        ));
        assert_eq!(
            compute(&typed, left, right),
            ConstValue::Int(expected.into())
        );
    }
    for right_domain in ["-1 to 10", "0 to 256"] {
        let typed = support::check(&format!(
            "compute(left in 0 to 1, right in {right_domain}): return left << right\n"
        ));
        assert!(
            support::flaw_codes(&typed).contains(&"type-integer-shift-count-unproven".to_string()),
            "{:?}",
            typed.all_flaws()
        );
    }
    let typed =
        support::check("compute(left in 1 to 1, right in 255 to 255): return left << right\n");
    assert!(
        support::flaw_codes(&typed).contains(&"type-integer-arithmetic-overflow".to_string()),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn right_shift_preserves_source_bits_and_tightens_the_result_domain() {
    for (left_domain, right_domain, left, right, expected, min, max) in [
        (
            "0 to 255", "3 to 3", 255_i128, 3_i128, 31_i128, 0_i128, 31_i128,
        ),
        ("-128 to 127", "3 to 3", -128, 3, -16, -16, 15),
        ("0 to 0", "0 to 255", 0, 255, 0, 0, 0),
    ] {
        let typed = support::check_no_flaws(&format!(
            "compute(left in {left_domain}, right in {right_domain}): return left >> right\n"
        ));
        assert_eq!(
            compute(&typed, left, right),
            ConstValue::Int(expected.into())
        );
        let result = &typed.defs[&DefId::named(Intern::from_ref("compute"))].return_type;
        let hull = result
            .anonymous_integer_validity()
            .unwrap()
            .domain()
            .storage_hull()
            .unwrap();
        assert_eq!((hull.min(), hull.max()), (min.into(), max.into()));
    }
    for right_domain in ["-1 to 10", "0 to 256"] {
        let typed = support::check(&format!(
            "compute(left in 0 to 255, right in {right_domain}): return left >> right\n"
        ));
        assert!(
            support::flaw_codes(&typed).contains(&"type-integer-shift-count-unproven".to_string()),
            "{:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn shift_counts_use_proven_bindings_and_computed_singleton_bounds() {
    let typed = support::check_no_flaws(concat!(
        "Signed is in -128 to 127\nSigned has finish(self): return\n",
        "compute(left in 0 to 255, right Signed and right >= 0):\n",
        "    count is in 0 to 127: right\n",
        "    return left >> count\n",
    ));
    assert_eq!(compute(&typed, 255, 3), ConstValue::Int(31.into()));
    assert_eq!(compute(&typed, 255, 127), ConstValue::Int(0.into()));
    let typed = support::check_no_flaws(concat!(
        "Byte is in 0 to 255\nByte has finish(self): return\n",
        "compute(left in 0 to 255, right in 0 to 65535):\n",
        "    width: #size(Byte) * 8\n",
        "    when right >= width then return 0\n",
        "    return left << right\n",
    ));
    assert_eq!(compute(&typed, 3, 7), ConstValue::Int(384.into()));
    assert_eq!(compute(&typed, 3, 8), ConstValue::Int(0.into()));
}

#[test]
fn division_normalization_agrees_with_evaluation() {
    for operator in ["/", "%"] {
        let mut typed = support::check_no_flaws(&format!(
            "compute(left in -10000 to 10000, right in 1 to 10): left {operator} right\n"
        ));
        typed.exprs.facts.const_facts.const_value.fill(None);
        let root = support::body(&typed, "compute");
        let mut substitutions = DepSubst::new();
        substitutions
            .consts
            .insert(Intern::from_ref("left"), (-10000).into());
        substitutions
            .consts
            .insert(Intern::from_ref("right"), 3.into());
        assert_eq!(
            normalize(&typed, root, &substitutions),
            NormalizeResult::Value(compute(&typed, -10000, 3))
        );
    }
}

fn core_checked_integer_source() -> String {
    let operators = include_str!("../../../modules/core/ops/operators.gin")
        .split_once("BitOr has")
        .unwrap()
        .0;
    let integer = include_str!("../../../modules/core/primitive/int.gin");
    let mut source = format!(
        "{operators}SignedBigInt is in -9223372036854775808 to 9223372036854775807\n\
         IntegerResult is SignedBigInt or Flaw(DivideByZero or Overflow or InvalidShift)\n\
         SignedBigInt has Add(IntegerResult) and Sub(IntegerResult) and Mul(IntegerResult) and BitAnd\n    release(self): return\n"
    );
    for (start, end) in [
        ("    add(", "    divide("),
        ("    bit_and(", "    bit_or("),
        ("    decimal_width(", "    bit_width("),
    ] {
        let method = integer
            .split_once(start)
            .unwrap()
            .1
            .split_once(end)
            .unwrap()
            .0;
        source.push_str(start);
        source.push_str(method);
    }
    source
}

#[test]
fn core_signed_operators_report_overflow_without_wrapping() {
    let minimum = i128::from(i64::MIN);
    let maximum = i128::from(i64::MAX);
    for (operator, cases) in [
        (
            "+",
            [
                (minimum, 0, minimum),
                (maximum, 0, maximum),
                (minimum, -1, 999),
                (maximum, 1, 999),
                (100, -5, 95),
            ],
        ),
        (
            "-",
            [
                (minimum, 0, minimum),
                (maximum, 0, maximum),
                (minimum, 1, 999),
                (maximum, -1, 999),
                (minimum, minimum, 0),
            ],
        ),
        (
            "*",
            [
                (minimum, 1, minimum),
                (maximum, 1, maximum),
                (minimum, -1, 999),
                (maximum, 2, 999),
                (minimum, 0, 0),
            ],
        ),
    ] {
        let typed = support::check_no_flaws(&format!(
            "{}compute(left SignedBigInt, right SignedBigInt):\n\
                 \x20   outcome: left {operator} right\n\
                 \x20   return when outcome is\n\
                 \x20       Flaw(_) then 999\n\
                 \x20       else\n\
                 \x20           value is in SignedBigInt: outcome\n\
                 \x20           return value\n",
            core_checked_integer_source()
        ));
        for (left, right, expected) in cases {
            assert_eq!(
                compute(&typed, left, right),
                ConstValue::Int(expected.into()),
                "{left} {operator} {right}"
            );
        }
    }
}

#[test]
fn core_signed_decimal_width_preserves_its_bound_with_operator_traits() {
    let typed = support::check_no_flaws(&core_checked_integer_source());
    let target = flask::CompileTarget::Library;
    let function = DefId::named(Intern::from_ref("SignedBigInt.decimal_width"));
    for (value, expected) in [(0, 1), (123, 3), (-123, 4), (i64::MAX, 19), (i64::MIN, 20)] {
        let actual = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target))
            .call(function, &[ConstValue::Int(value.into())])
            .unwrap();
        assert_eq!(actual, ConstValue::Int(expected.into()));
    }
}

#[test]
fn core_signed_bit_width_is_target_invariant() {
    let integer = include_str!("../../../modules/core/primitive/int.gin");
    let method = integer.split_once("    bit_width()").unwrap().1;
    let typed = support::check_no_flaws(&format!(
        "SignedBigInt is in -9223372036854775808 to 9223372036854775807\nSignedBigInt has\n    release(self): return\n    bit_width(){method}"
    ));
    let target = flask::CompileTarget::Library;
    let actual = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target))
        .call(
            DefId::named(Intern::from_ref("SignedBigInt.bit_width")),
            &[],
        )
        .unwrap();
    assert_eq!(actual, ConstValue::Int(64.into()));
}
