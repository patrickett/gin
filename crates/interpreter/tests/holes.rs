//! `#todo`: the program runs until the hole is reached, and a hole that is not reached never fires.
use interpreter::{Error, Interpreter, Program, Value, with_big_stack};

fn call(source: &str, function: &str, argument: i128) -> Result<Value, Error> {
    let (source, function) = (source.to_owned(), function.to_owned());
    with_big_stack(move || {
        let program = Program::from_source(&source).expect("a program with a hole checks");
        Interpreter::new(&program).call(&function, &[], &[Value::integer(argument)]).map(|o| o.value)
    })
}

const GUARDED: &str = "f(a in 0 to 9) in 0 to 9:\n    when a < 5 then return 1\n    return #todo\n";

#[test]
fn the_function_runs_until_the_hole_is_reached() {
    assert_eq!(call(GUARDED, "f", 2), Ok(Value::integer(1)));
    let error = call(GUARDED, "f", 7).unwrap_err();
    assert!(matches!(&error, Error::HoleReached { function, stack, .. } if function == "f" && stack == &["f"]), "{error:?}");
}

#[test]
fn a_hole_in_a_callee_reports_the_whole_call_chain() {
    let source = "inner(a in 0 to 9) in 0 to 9: return #todo\nouter(a in 0 to 9) in 0 to 9: return inner(a)\n";
    let error = call(source, "outer", 1).unwrap_err();
    assert!(
        matches!(&error, Error::HoleReached { function, stack, .. } if function == "inner" && stack == &["outer", "inner"]),
        "{error:?}"
    );
    assert!(error.to_string().contains("outer -> inner"), "{error}");
}

#[test]
fn an_untaken_hole_in_a_callee_never_fires() {
    let source = "inner(a in 0 to 9) in 0 to 9: return #todo\nouter(a in 0 to 9) in 0 to 9: return when a < 5 then 4 else inner(a)\n";
    assert_eq!(call(source, "outer", 3), Ok(Value::integer(4)));
    assert!(matches!(call(source, "outer", 8), Err(Error::HoleReached { .. })));
}
