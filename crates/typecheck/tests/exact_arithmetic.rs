//! One definition of exact integer arithmetic: `eval::exact_binary`. The recorder and the
//! interpreter use `apply` for `+ - *`; the constants evaluator uses `exact_binary` for all of
//! them, so these tests pin that the two cannot drift and what each operator means.
use i256::I256;
use ast::BinaryOperator as B;
use typecheck::{CheckedArithmetic, Exact, Limits, SourceCheckError, apply, check_module, exact_binary};

fn n(value: i64) -> I256 {
    I256::from(value)
}

#[test]
fn add_subtract_multiply_are_apply() {
    let samples = [-1_000_003, -7, -1, 0, 1, 2, 9, 1 << 40];
    for &l in &samples {
        for &r in &samples {
            for (op, checked) in [(B::Add, CheckedArithmetic::Add), (B::Subtract, CheckedArithmetic::Subtract), (B::Multiply, CheckedArithmetic::Multiply)] {
                assert_eq!(exact_binary(op, n(l), n(r)), apply(checked, n(l), n(r)).map_or(Exact::Overflow, Exact::Value), "{l} {op:?} {r}");
            }
        }
    }
    // Beyond the checker's width there is no value, never a wrap.
    let big = I256::MAX;
    assert_eq!(exact_binary(B::Add, big, I256::ONE), Exact::Overflow);
    assert_eq!(apply(CheckedArithmetic::Add, big, I256::ONE), None);
}

#[test]
fn division_truncates_toward_zero_and_a_remainder_follows_its_dividend() {
    assert_eq!(exact_binary(B::Divide, n(7), n(2)), Exact::Value(n(3)));
    assert_eq!(exact_binary(B::Divide, n(-7), n(2)), Exact::Value(n(-3)));
    assert_eq!(exact_binary(B::Remainder, n(-7), n(2)), Exact::Value(n(-1)));
    assert_eq!(exact_binary(B::Remainder, n(7), n(-2)), Exact::Value(n(1)));
    // Negative controls: a zero divisor has no value.
    assert_eq!(exact_binary(B::Divide, n(1), n(0)), Exact::Overflow);
    assert_eq!(exact_binary(B::Remainder, n(1), n(0)), Exact::Overflow);
}

#[test]
fn shifts_are_powers_of_two_and_right_keeps_the_sign() {
    assert_eq!(exact_binary(B::ShiftLeft, n(3), n(4)), Exact::Value(n(48)));
    assert_eq!(exact_binary(B::ShiftRight, n(-9), n(1)), Exact::Value(n(-5)));
    assert_eq!(exact_binary(B::ShiftRight, n(9), n(1)), Exact::Value(n(4)));
    assert_eq!(exact_binary(B::ShiftLeft, n(1), n(-1)), Exact::ShiftAmount(n(-1)));
    assert_eq!(exact_binary(B::ShiftLeft, n(1), n(255)), Exact::ShiftAmount(n(255)));
    assert_eq!(exact_binary(B::ShiftLeft, n(1), n(254)), Exact::Value(I256::ONE << 254_u32));
    assert_eq!(exact_binary(B::ShiftLeft, I256::MAX, n(1)), Exact::Overflow);
}

#[test]
fn operators_that_are_not_exact_arithmetic_are_said_so() {
    for op in [B::Less, B::Equal, B::And, B::Or, B::GreaterOrEqual] {
        assert_eq!(exact_binary(op, n(1), n(2)), Exact::NotArithmetic, "{op:?}");
    }
}

fn constants(text: &str) -> Result<Vec<(String, I256)>, SourceCheckError> {
    let file = parser::parse_strict(text).expect("parses");
    check_module("m", &file, &[], Limits::default()).map(|m| m.constants().to_vec())
}

#[test]
fn the_constants_evaluator_agrees_with_exact_binary() {
    let found = constants("a := -7 / 2\nb := -7 % 2\nc := 3 << 4\nd := -9 >> 1\ne := 6 * 7 - 2 + 1\n").unwrap();
    let value = |name: &str| found.iter().find(|(n, _)| n == name).unwrap().1;
    assert_eq!(Exact::Value(value("a")), exact_binary(B::Divide, n(-7), n(2)));
    assert_eq!(Exact::Value(value("b")), exact_binary(B::Remainder, n(-7), n(2)));
    assert_eq!(Exact::Value(value("c")), exact_binary(B::ShiftLeft, n(3), n(4)));
    assert_eq!(Exact::Value(value("d")), exact_binary(B::ShiftRight, n(-9), n(1)));
    assert_eq!(value("e"), n(41));
}

#[test]
fn constants_reject_what_exact_binary_has_no_value_for() {
    assert_eq!(constants("a := 1 / 0\n").unwrap_err(), SourceCheckError::Overflow);
    assert_eq!(constants("a := 1 << 255\n").unwrap_err(), SourceCheckError::Overflow);
    assert_eq!(constants("a := 1 << (0 - 1)\n").unwrap_err(), SourceCheckError::Overflow);
}

/// The prover works with intervals and the evaluator with numbers; this is the link between the
/// two. For `+ - *` the interval of two ranges holds the concrete result of every pair of
/// members, and is exactly as wide as the results (the extremes are attained, which the corners
/// argument in `eval::interval` promises).
#[test]
fn the_interval_of_an_operation_is_sound_and_tight_for_the_concrete_one() {
    use typecheck::interval;
    let ranges = [(-3, 4), (0, 0), (2, 7), (-9, -1), (5, 5)];
    for (op, checked) in [(B::Add, CheckedArithmetic::Add), (B::Subtract, CheckedArithmetic::Subtract), (B::Multiply, CheckedArithmetic::Multiply)] {
        for &(ll, lu) in &ranges {
            for &(rl, ru) in &ranges {
                let (lower, upper) = interval(checked, (n(ll), n(lu)), (n(rl), n(ru))).unwrap();
                let mut seen = Vec::new();
                for l in ll..=lu {
                    for r in rl..=ru {
                        let Exact::Value(v) = exact_binary(op, n(l), n(r)) else { panic!() };
                        assert!(lower <= v && v <= upper, "{op:?} {l},{r}");
                        seen.push(v);
                    }
                }
                assert_eq!((seen.iter().min().copied(), seen.iter().max().copied()), (Some(lower), Some(upper)), "{op:?} {ll}..{lu} {rl}..{ru}");
            }
        }
    }
}

/// D19. For every operation the checker records, the domain the prover gives (`interval`) holds
/// the exact result of every pair of members, wherever the operation is defined. Where the
/// prover cannot establish that the operation is defined (a divisor that may be zero, a shift
/// outside 0 to 254) there is no domain.
#[test]
fn every_recorded_operations_domain_contains_its_exact_results() {
    use typecheck::interval;
    let ops = [
        (B::Divide, CheckedArithmetic::Divide),
        (B::Remainder, CheckedArithmetic::Remainder),
        (B::BitAnd, CheckedArithmetic::BitAnd),
        (B::BitOr, CheckedArithmetic::BitOr),
        (B::BitXor, CheckedArithmetic::BitXor),
        (B::ShiftLeft, CheckedArithmetic::ShiftLeft),
        (B::ShiftRight, CheckedArithmetic::ShiftRight),
    ];
    let lefts = [(-9, 9), (0, 0), (3, 12), (-8, -1), (7, 7), (0, 31)];
    let rights = [(1, 4), (-4, -1), (2, 2), (-3, 5), (0, 0), (0, 3), (1, 7), (0, 10)];
    let mut checked_pairs = 0;
    for (op, arithmetic) in ops {
        for &(ll, lu) in &lefts {
            for &(rl, ru) in &rights {
                let domain = interval(arithmetic, (n(ll), n(lu)), (n(rl), n(ru)));
                let shift = matches!(op, B::ShiftLeft | B::ShiftRight);
                let division = matches!(op, B::Divide | B::Remainder);
                if division && rl <= 0 && 0 <= ru {
                    assert_eq!(domain, None, "{op:?} a divisor that may be zero has no domain");
                    continue;
                }
                if shift && rl < 0 {
                    assert_eq!(domain, None, "{op:?} a negative amount has no domain");
                    continue;
                }
                let (lower, upper) = domain.unwrap_or_else(|| panic!("{op:?} {ll}..{lu} {rl}..{ru}: no domain"));
                for l in ll..=lu {
                    for r in rl..=ru {
                        let Exact::Value(v) = exact_binary(op, n(l), n(r)) else { panic!("{op:?} {l} {r}") };
                        assert!(lower <= v && v <= upper, "{op:?} {l},{r} = {v} outside {lower}..{upper}");
                        checked_pairs += 1;
                    }
                }
            }
        }
    }
    assert!(checked_pairs > 1_000, "{checked_pairs}");
    // A shift by 255 or more has no domain.
    assert_eq!(interval(CheckedArithmetic::ShiftLeft, (n(1), n(1)), (n(0), n(300))), None);
}

#[test]
fn bit_operations_are_twos_complement() {
    assert_eq!(exact_binary(B::BitAnd, n(-1), n(0xff)), Exact::Value(n(0xff)));
    assert_eq!(exact_binary(B::BitOr, n(-8), n(3)), Exact::Value(n(-5)));
    assert_eq!(exact_binary(B::BitXor, n(-1), n(5)), Exact::Value(n(-6)));
    assert_eq!(exact_binary(B::BitXor, n(6), n(3)), Exact::Value(n(5)));
}
