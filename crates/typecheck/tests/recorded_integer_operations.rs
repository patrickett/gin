//! Division, remainder, the bit operations and shifts are recorded (they were checked but left a
//! body unrecorded), with the exact arithmetic of `eval::exact_binary`, so every consumer of a
//! body can run them. Negative controls: what the checker cannot prove defined is still refused.
use typecheck::{
    CheckedArithmetic as Op, CheckedSourceBodyKind as Kind, CheckedSourceFile, Limits, SourceCheckError, SourceType, check_file,
};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    check_file(&parser::parse_strict(text).expect("parses"), Limits::default())
}

fn binary(file: &CheckedSourceFile, name: &str) -> Option<(Op, SourceType)> {
    let at = file.functions.iter().position(|f| f.name == name)?;
    let body = file.bodies[at].as_ref()?;
    body.expressions.iter().find_map(|e| match e.kind {
        Kind::Binary { operator, .. } => Some((operator, e.ty)),
        _ => None,
    })
}

fn root_integer(file: &CheckedSourceFile, name: &str) -> Option<i128> {
    let at = file.functions.iter().position(|f| f.name == name)?;
    let body = file.bodies[at].as_ref()?;
    let Kind::Block { result, .. } = &body.expressions[body.root as usize].kind else { return None };
    let Kind::Return { value } = body.expressions[*result as usize].kind else { return None };
    match &body.expressions[value as usize].kind {
        Kind::Integer(n) => Some(n.as_i128()),
        _ => None,
    }
}

fn domain(ty: SourceType) -> (i128, i128) {
    let SourceType::Integer { lower, upper } = ty else { panic!("{ty:?}") };
    (lower.as_i128(), upper.as_i128())
}

#[test]
fn each_operation_is_recorded_with_its_domain() {
    let file = check(concat!(
        "div(a in 0 to 9, b in 1 to 3) in 0 to 9: return a / b\n",
        "rem(a in 0 to 9, b in 1 to 3) in 0 to 2: return a % b\n",
        "bitand(a in 0 to 12, b in 0 to 10) in 0 to 15: return a & b\n",
        "bitor(a in 0 to 12, b in 0 to 10) in 0 to 15: return a | b\n",
        "bitxor(a in 0 to 12, b in 0 to 10) in 0 to 15: return a ^ b\n",
        "left(a in 0 to 9, b in 0 to 3) in 0 to 72: return a << b\n",
        "right(a in 0 to 99, b in 0 to 3) in 0 to 99: return a >> b\n",
    ))
    .unwrap();
    assert_eq!(binary(&file, "div").map(|(o, t)| (o, domain(t))), Some((Op::Divide, (0, 9))));
    assert_eq!(binary(&file, "rem").map(|(o, _)| o), Some(Op::Remainder));
    assert_eq!(binary(&file, "bitand").map(|(o, _)| o), Some(Op::BitAnd));
    assert_eq!(binary(&file, "bitor").map(|(o, _)| o), Some(Op::BitOr));
    assert_eq!(binary(&file, "bitxor").map(|(o, _)| o), Some(Op::BitXor));
    assert_eq!(binary(&file, "left").map(|(o, t)| (o, domain(t))), Some((Op::ShiftLeft, (0, 72))));
    assert_eq!(binary(&file, "right").map(|(o, _)| o), Some(Op::ShiftRight));
}

#[test]
fn constants_fold_with_the_shared_arithmetic() {
    let file = check(concat!(
        "q() in 0 to 255: return 7 / 2\n",
        "r() in 0 to 255: return 7 % 4\n",
        "m() in 0 to 255: return 12 & 10\n",
        "o() in 0 to 255: return 12 | 3\n",
        "x() in 0 to 255: return 6 ^ 3\n",
        "s() in 0 to 255: return 3 << 4\n",
        "t() in 0 to 255: return 200 >> 2\n",
        "n() in 0 to 255: return (0 - 7) / 2 + 10\n",
    ))
    .unwrap();
    for (name, value) in [("q", 3), ("r", 3), ("m", 8), ("o", 15), ("x", 5), ("s", 48), ("t", 50), ("n", 7)] {
        assert_eq!(root_integer(&file, name), Some(value), "{name}");
    }
}

#[test]
fn a_call_with_known_arguments_folds_through_the_evaluator() {
    let file = check("div(a in 0 to 9, b in 1 to 3) in 0 to 9: return a / b\nknown() in 0 to 9: return div(8, 3)\nunknown(n in 0 to 9) in 0 to 9: return div(n, 2)\n").unwrap();
    assert_eq!(root_integer(&file, "known"), Some(2));
    // Negative control: with an unknown argument the call stays a call.
    assert_eq!(root_integer(&file, "unknown"), None);
    let at = file.functions.iter().position(|f| f.name == "unknown").unwrap();
    assert!(file.bodies[at].as_ref().unwrap().expressions.iter().any(|e| matches!(e.kind, Kind::Call { .. })));
}

#[test]
fn a_guard_that_excludes_zero_lets_the_division_be_recorded() {
    let file = check("safe(a in 0 to 9, b in 0 to 3) in 0 to 9:\n    when b = 0 then return 0\n    return a / b\n").unwrap();
    let (operator, ty) = binary(&file, "safe").expect("recorded");
    assert_eq!(operator, Op::Divide);
    // The type is the domain with the zero divisor excluded, not an error.
    assert_eq!(domain(ty), (0, 9));
}

#[test]
fn what_cannot_be_proved_defined_is_still_refused() {
    // A divisor that may be zero, a constant zero divisor, an oversize shift: the checker refuses.
    assert_eq!(check("f(a in 0 to 9, b in 0 to 3) in 0 to 9: return a / b\n").unwrap_err(), SourceCheckError::Unproved);
    assert_eq!(check("f() in 0 to 9: return 1 / 0\n").unwrap_err(), SourceCheckError::Unproved);
    assert_eq!(check("f(a in 0 to 9, b in 0 to 300) in 0 to 9: return a << b\n").unwrap_err(), SourceCheckError::Unproved);
    assert_eq!(check("f(a in 0 to 9, b in 0 to 3) in 0 to 9: return a % b\n").unwrap_err(), SourceCheckError::Unproved);
}

#[test]
fn an_oversize_constant_shift_is_recorded_as_the_exact_number() {
    // `1 << 100` is exact at the checker's width; it is the lowering that refuses it (no i64).
    let file = check("big() in 0 to 1267650600228229401496703205376: return 1 << 100\n").unwrap();
    assert_eq!(root_integer(&file, "big"), Some(1_i128 << 100));
}

#[test]
fn an_assignment_records_these_too() {
    let file = check("half(a in 0 to 9) in 0 to 9:\n    x in 0 to 9: a\n    x :: x / 2\n    return x\n").unwrap_or_else(|e| panic!("{e:?}"));
    assert!(binary(&file, "half").is_some_and(|(o, _)| o == Op::Divide));
}
