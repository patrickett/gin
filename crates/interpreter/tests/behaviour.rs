//! What the interpreter does at the edge of its reach, and the negative controls: it never
//! runs a program the checker refused, never guesses at a body the recorder did not produce, and
//! never evaluates an arm that is not taken.
use interpreter::{Error, Interpreter, Limits, LoadError, Program, Value, with_big_stack};
use typecheck::SourceCheckError;

fn int(value: i128) -> Value {
    Value::integer(value)
}

fn call(source: &str, function: &str, arguments: &[Value]) -> Result<Value, Error> {
    let source = source.to_owned();
    let function = function.to_owned();
    let arguments = arguments.to_vec();
    with_big_stack(move || {
        let program = Program::from_source(&source).expect("the program checks");
        Interpreter::new(&program).call(&function, &[], &arguments).map(|outcome| outcome.value)
    })
}

const TRAP: &str = "#unrestricted\nNothing has\n\n#instruction('brk #0')\ntrap(input Type, output Type, inputs input) output\n\nrisky() in 0 to 9:\n    trap(Nothing, Nothing, Nothing())\n    return 2\n\n";

#[test]
fn an_arm_that_is_not_taken_is_never_run() {
    let source = format!("{TRAP}choose(n in 0 to 9) in 0 to 9: return when n < 5 then 1 else risky()\n");
    assert_eq!(call(&source, "choose", &[int(3)]), Ok(int(1)));
    // Taken, the arm reaches a machine instruction, which this interpreter does not run.
    assert!(matches!(call(&source, "choose", &[int(7)]), Err(Error::Unsupported { .. })));
}

#[test]
fn the_rest_after_a_returning_guard_is_not_run_when_the_guard_holds() {
    let source = format!("{TRAP}guard(n in 0 to 9) in 0 to 9:\n    when n < 5 then return 1\n    trap(Nothing, Nothing, Nothing())\n    return 2\n");
    assert_eq!(call(&source, "guard", &[int(3)]), Ok(int(1)));
}

#[test]
fn a_chain_arm_runs_only_when_it_is_the_first_that_holds() {
    let source = format!("{TRAP}choose(n in 0 to 9) in 0 to 9:\n    return when\n        n < 5 then risky()\n        n < 8 then 1\n        else 3\n");
    assert_eq!(call(&source, "choose", &[int(9)]), Ok(int(3)));
    assert_eq!(call(&source, "choose", &[int(6)]), Ok(int(1)));
    assert!(matches!(call(&source, "choose", &[int(2)]), Err(Error::Unsupported { .. })));
}

#[test]
fn an_instruction_function_is_unsupported_not_guessed() {
    let source = format!("{TRAP}start() in 0 to 9: return risky()\n");
    let error = call(&source, "start", &[]).unwrap_err();
    assert!(matches!(error, Error::Unsupported { what, .. } if what.contains("instruction")), "{error:?}");
}

#[test]
fn an_infinite_loop_runs_out_of_fuel_instead_of_hanging() {
    let source = "spin(n in 0 to 9) in 0 to 9:\n    i in 0 to 9: 0\n    while i < 5 do\n        i :: 0\n    return i\n";
    let source = source.to_owned();
    let error = with_big_stack(move || {
        let program = Program::from_source(&source).expect("the program checks");
        let limits = Limits { fuel: 10_000, ..Limits::default() };
        Interpreter::with_limits(&program, limits).call("spin", &[], &[int(1)]).unwrap_err()
    });
    assert!(matches!(error, Error::OutOfFuel(10_000)), "{error:?}");
}

#[test]
fn a_function_the_recorder_declined_has_no_body_and_is_not_run() {
    // A loop whose condition joins two comparisons is not recorded yet.
    let source = "answer(n in 0 to 3) in 0 to 9:\n  i in 0 to 9: 0\n  while i < n and i < 2 do\n    i :: i + 1\n  return i\n";
    assert!(matches!(call(source, "answer", &[int(2)]), Err(Error::NoBody(name)) if name == "answer"));
}

#[test]
fn an_unknown_function_and_a_wrong_arity_are_refused() {
    let source = "f(n in 0 to 9) in 0 to 9: return n\n";
    assert!(matches!(call(source, "g", &[]), Err(Error::NoSuchFunction(_))));
    assert!(matches!(call(source, "f", &[]), Err(Error::Arity { expected: 1, given: 0, .. })));
}

#[test]
fn an_argument_outside_its_domain_is_refused() {
    let source = "f(n in 0 to 9) in 0 to 9: return n\n";
    assert!(matches!(call(source, "f", &[int(10)]), Err(Error::ArgumentOutOfDomain { .. })));
    assert!(matches!(call(source, "f", &[int(-1)]), Err(Error::ArgumentOutOfDomain { .. })));
    assert_eq!(call(source, "f", &[int(9)]), Ok(int(9)));
}

/// The checker, not the interpreter, enforces linearity: a program that uses a linear value
/// twice never reaches the interpreter, which has no run-time ownership check to fall back on.
#[test]
fn a_program_the_checker_rejects_is_never_run() {
    let source = "Token has value in 0 to 9\n\nspend(t Token) in 0 to 9:\n    Token(a) := t\n    Token(b) := t\n    return a\n";
    let error = Program::from_source(source).unwrap_err();
    assert!(
        matches!(&error, LoadError::Check(failure) if failure.error == SourceCheckError::UseAfterMove),
        "{error:?}"
    );
}

#[test]
fn a_domain_error_in_source_is_a_check_error_not_a_run_time_one() {
    // `n - 3` can be negative: the checker refuses it, so no run-time underflow check exists.
    let error = Program::from_source("f(n in 0 to 9) in 0 to 9: return n - 3\n").unwrap_err();
    assert!(matches!(error, LoadError::Check(_)), "{error:?}");
}

#[test]
fn syntax_errors_are_load_errors() {
    assert!(matches!(Program::from_source("f(: return\n").unwrap_err(), LoadError::Syntax(_)));
}

#[test]
fn records_and_sums_render_with_their_field_names() {
    let source = "#unrestricted\nPoint has\n    x in 0 to 9\n    y in 0 to 9\n\nmake(a in 0 to 9) Point: return Point(x: a, y: 4)\n";
    let source = source.to_owned();
    with_big_stack(move || {
        let program = Program::from_source(&source).unwrap();
        let outcome = Interpreter::new(&program).call("make", &[], &[int(2)]).unwrap();
        assert_eq!(program.render(&outcome.value), "Point(x: 2, y: 4)");
        assert_eq!(outcome.value.to_string(), "Point(2, 4)");
    });
}

#[test]
fn a_mut_parameter_comes_back_in_the_outcome() {
    let source = "bump(x mut in 0 to 9, by in 0 to 3) in 0 to 9:\n  x :: when x < 6 then x + by else x\n  return by\n";
    let source = source.to_owned();
    with_big_stack(move || {
        let program = Program::from_source(&source).unwrap();
        let outcome = Interpreter::new(&program).call("bump", &[], &[int(4), int(2)]).unwrap();
        assert_eq!((outcome.value, outcome.mutated), (int(2), vec![int(6)]));
        let outcome = Interpreter::new(&program).call("bump", &[], &[int(8), int(2)]).unwrap();
        assert_eq!(outcome.mutated, vec![int(8)]);
    });
}

#[test]
fn a_generic_function_runs_with_the_types_it_is_applied_to() {
    use interpreter::RtType;
    let source = "#unrestricted\nWord has value in 0 to 255\n\nid(element, x element) element: return x\n";
    let source = source.to_owned();
    with_big_stack(move || {
        let program = Program::from_source(&source).unwrap();
        let word = RtType::Nominal { home: "main".into(), name: "Word".into(), args: vec![] };
        let given = Interpreter::new(&program).call("id", &[], &[int(1)]);
        assert!(matches!(given, Err(Error::TypeArity { expected: 1, given: 0, .. })));
        let value = Interpreter::new(&program).call("id", &[word], &[int(1)]).unwrap().value;
        assert_eq!(value, int(1));
    });
}

#[test]
fn a_sum_member_is_written_by_its_member_name() {
    let source = "#unrestricted\nMode is Quiet or Loud\n\nloud() Mode: return Mode.Loud\n";
    let source = source.to_owned();
    with_big_stack(move || {
        let program = Program::from_source(&source).unwrap();
        let outcome = Interpreter::new(&program).call("loud", &[], &[]).unwrap();
        assert_eq!(program.render(&outcome.value), "Loud");
        assert_eq!(outcome.value.to_string(), "Loud");
        let Value::Record { tag, .. } = &outcome.value else { panic!() };
        assert_eq!((tag.short(), tag.sum()), ("Loud", Some("Mode")));
    });
}

const MEMORY: &str = "#unrestricted\nByte is in 0 to 255\n\n#load\nload(element Type, pointer ref @element, index in 0 to 15) element\n\n#store\nstore(element Type, pointer ref @element, index in 0 to 15, value element) ()\n\n";

#[test]
fn raw_memory_is_refused_by_name_until_memory_is_byte_addressed() {
    // Not taken, the arm is never run; taken, it is refused rather than guessed.
    let source = format!(
        "{MEMORY}peek(address in 0 to 4095, n in 0 to 9) in 0 to 9:\n    return when n < 5 then 1 else 2\n"
    );
    assert_eq!(call(&source, "peek", &[int(8), int(3)]), Ok(int(1)));
    let source = format!(
        "{MEMORY}poke(pointer ref @Byte) Byte: return load(Byte, pointer, 0)\n"
    );
    match call(&source, "poke", &[int(0)]) {
        Err(Error::Unsupported { what, .. }) => assert!(what.contains("raw memory"), "{what}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_target_choice_records_the_arm_of_its_target_and_an_unknown_target_takes_else() {
    // A domain result chosen by the target: the arms for other targets are checked but are not the
    // body, so the conversions the body records are those of the arm that runs. On a target none
    // of the arms names, `else` runs.
    let source = "#unrestricted\nResult is in -100 to 100\npick() Result: return when #target.triple is\n    'no-such-a' then 1\n    'no-such-b' then 2\n    else 3\n";
    let answer = format!("{:?}", call(source, "pick", &[]));
    assert!(answer.contains("fields: [Integer(3)]"), "{answer}");
}

const NESTED: &str = "#unrestricted\nCode is in 0 to 9\n\n#unrestricted\nAllocError is OutOfMemory or Fragmented\n\n#unrestricted\nOutcome is Value(Code) or Flaw(AllocError)\n\nclassify(n in 0 to 9) Outcome:\n    when n < 3 then\n        return Value(n as Code)\n    when n < 6 then\n        return Flaw(OutOfMemory)\n    return Flaw(Fragmented)\n\nrank(n in 0 to 9) in 0 to 9: return when classify(n) is\n    Value(v) then v\n    Flaw(OutOfMemory) then 7\n    Flaw(Fragmented) then 8\n";

#[test]
fn a_nested_pattern_picks_the_arm_of_the_inner_variant() {
    // Audit 2026-11-05: the interpreter runs the recorder's match-over-the-payload.
    assert_eq!(call(NESTED, "rank", &[int(2)]), Ok(int(2)));
    assert_eq!(call(NESTED, "rank", &[int(4)]), Ok(int(7)));
    assert_eq!(call(NESTED, "rank", &[int(9)]), Ok(int(8)));
    // Negative control: swapping the inner arms' values swaps the answers, so the inner variant
    // (not only the outer one) decides.
    let swapped = NESTED.replace("Flaw(OutOfMemory) then 7", "Flaw(OutOfMemory) then 5").replace("Flaw(Fragmented) then 8", "Flaw(Fragmented) then 6");
    assert_eq!(call(&swapped, "rank", &[int(4)]), Ok(int(5)));
    assert_eq!(call(&swapped, "rank", &[int(9)]), Ok(int(6)));
}
