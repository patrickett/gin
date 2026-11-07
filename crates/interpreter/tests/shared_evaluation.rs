//! Where the checker's compile-time evaluator (`typecheck::evaluate_body`, which also does
//! the fold of a call with known arguments at check time) and the interpreter agree, and every
//! place they differ, pinned. A test failing here means evaluator behavior changed.
use interpreter::{Error, Interpreter, Limits, Program, Value, with_big_stack};
use typecheck::{
    BodyResolver, CheckedSourceBody, CheckedSourceBodyKind as Kind, CheckedSourceCallee, CheckedSourceFile, Limits as CheckLimits,
    check_module, evaluate_body, evaluate_value,
};

struct Table<'a>(&'a CheckedSourceFile);

impl BodyResolver for Table<'_> {
    fn body(&self, callee: CheckedSourceCallee) -> Option<&CheckedSourceBody> {
        match callee {
            CheckedSourceCallee::Local { function } => self.0.bodies.get(function as usize)?.as_ref(),
            CheckedSourceCallee::Dependency { .. } => None,
        }
    }
}

fn checked(source: &str) -> typecheck::CheckedSourceModule {
    let file = parser::parse_strict(source).expect("parses");
    check_module("main", &file, &[], CheckLimits::default()).expect("checks")
}

fn index(file: &CheckedSourceFile, name: &str) -> usize {
    file.functions.iter().position(|f| f.name == name).unwrap_or_else(|| panic!("no function {name}"))
}

/// The checker's evaluator on `name(arguments)`.
fn by_evaluator(source: &str, name: &str, arguments: &[i64], fuel: u64) -> Option<i128> {
    let module = checked(source);
    let file = module.checked();
    let body = file.bodies[index(file, name)].as_ref()?;
    let arguments: Vec<_> = arguments.iter().map(|a| i256::I256::from(*a)).collect();
    evaluate_body(body, &arguments, &Table(file), &mut { fuel }).map(|v| v.as_i128())
}

/// The interpreter on `name(arguments)`.
fn by_interpreter(source: &str, name: &str, arguments: &[i64], fuel: u64) -> Result<Value, Error> {
    let (source, name) = (source.to_owned(), name.to_owned());
    let arguments: Vec<Value> = arguments.iter().map(|a| Value::integer(i128::from(*a))).collect();
    with_big_stack(move || {
        let program = Program::from_source(&source).expect("checks");
        let limits = Limits { fuel, ..Limits::default() };
        Interpreter::with_limits(&program, limits).call(&name, &[], &arguments).map(|o| o.value)
    })
}

/// What the checker's fold did with `start() = <call>`: the integer it folded to, or `None`
/// when the call stayed a call.
fn folded(source: &str) -> Option<i128> {
    let module = checked(source);
    let file = module.checked();
    let body = file.bodies[index(file, "start")].as_ref()?;
    body.expressions.iter().find_map(|e| match &e.kind {
        Kind::Integer(v) => Some(v.as_i128()),
        _ => None,
    })
    .filter(|_| !body.expressions.iter().any(|e| matches!(e.kind, Kind::Call { .. })))
}

fn has_call(source: &str) -> bool {
    let module = checked(source);
    let file = module.checked();
    file.bodies[index(file, "start")].as_ref().is_some_and(|b| b.expressions.iter().any(|e| matches!(e.kind, Kind::Call { .. })))
}

const FUEL: u64 = 10_000;

// ---- agreement ---------------------------------------------------------------------------

const AGREE: &[(&str, &str)] = &[
    ("clamp", "clamp(n in 0 to 9) in 0 to 7:\n    when n < 2 then return 0\n    when n > 8 then return 7\n    return n - 2\n"),
    ("grade", "grade(n in 0 to 9) in 0 to 3:\n    return when\n        n < 3 then 0\n        n < 6 then 1\n        n < 8 then 2\n        else 3\n"),
    ("total", "total(n in 0 to 9) in 0 to 99:\n  s in 0 to 99: 0\n  for i in 1 to n do\n    s +: i\n  return s\n"),
    ("nested", "nested(n in 0 to 4) in 0 to 999:\n  a in 0 to 400: 0\n  b in 0 to 400: 0\n  for i in 1 to n do\n    for j in 1 to i do\n      a +: 1\n    b +: i\n  return a + b\n"),
    ("spread", "spread(n in 0 to 8) in 0 to 9:\n    m in 0 to 9: 0\n    when n < 5 then\n        m :: n + 1\n    else\n        m :: n\n    return m\n"),
];

#[test]
fn the_evaluator_the_interpreter_and_the_fold_agree_on_what_they_all_run() {
    for (name, source) in AGREE {
        let limit = if *name == "nested" { 4 } else if *name == "spread" { 8 } else { 9 };
        for n in 0..=limit {
            let evaluated = by_evaluator(source, name, &[n], FUEL).unwrap_or_else(|| panic!("{name}({n}): the evaluator declined"));
            let interpreted = by_interpreter(source, name, &[n], 1_000_000).unwrap_or_else(|e| panic!("{name}({n}): {e}")).as_integer().unwrap().as_i128();
            assert_eq!(evaluated, interpreted, "{name}({n})");
            // The same call as the checker folds it at check time.
            let with_start = format!("{source}\nstart() in 0 to 255: return {name}({n})\n");
            if let Some(value) = folded(&with_start) {
                assert_eq!(value, interpreted, "{name}({n}): fold");
            }
        }
    }
}

#[test]
fn a_call_the_evaluator_can_run_is_folded_by_the_checker() {
    // Negative control for the test above: the fold does happen for these, so `folded` is not
    // vacuously `None`.
    for (name, source) in AGREE {
        let with_start = format!("{source}\nstart() in 0 to 255: return {name}(3)\n");
        assert!(folded(&with_start).is_some(), "{name}(3) was not folded; has call: {}", has_call(&with_start));
    }
}

#[test]
fn records_and_sums_agree_too() {
    let source = "#unrestricted\nMode is Quiet or Loud\n\n#unrestricted\nPoint has\n    x in 0 to 9\n    y in 0 to 9\n\nmake(a in 0 to 9) Point: return Point(x: a, y: 4)\npick(n in 0 to 9) Mode: return when n < 5 then Mode.Quiet else Mode.Loud\nlevel(n in 0 to 9) in 0 to 9: return when pick(n) is Loud then 5 else 1\n";
    for n in 0..=9 {
        let evaluated = by_evaluator(source, "level", &[n], FUEL).unwrap();
        assert_eq!(evaluated, by_interpreter(source, "level", &[n], 1_000_000).unwrap().as_integer().unwrap().as_i128(), "level({n})");
    }
    // A record result: the evaluator's `Record` has the same fields as the interpreter's.
    let module = checked(source);
    let file = module.checked();
    let body = file.bodies[index(file, "make")].as_ref().unwrap();
    let record = evaluate_value(body, &[typecheck::Evaluated::Integer(i256::I256::from(2))], &Table(file), &mut { FUEL }).unwrap();
    let typecheck::Evaluated::Record { fields, .. } = record else { panic!("{record:?}") };
    assert_eq!(fields.len(), 2);
    let Value::Record { fields: theirs, .. } = by_interpreter(source, "make", &[2], 1_000_000).unwrap() else { panic!() };
    assert_eq!(theirs.len(), fields.len());
    assert_eq!((theirs[0].as_integer().unwrap().as_i128(), theirs[1].as_integer().unwrap().as_i128()), (2, 4));
}

// ---- pinned differences ------------------------------------------------------------------
// Each is deliberate unless the doc says otherwise: the fold may decline; it must not disagree.

const WHILE: &str = "count(n in 0 to 9) in 0 to 10:\n    i in 0 to 10: 0\n    while i < n do\n        i :: i + 1\n    return i\n";

/// D3. `while`: the evaluator does not run it ("how many passes it takes is not worked out");
/// the interpreter does. A fold therefore leaves the call, and the interpreter's value is the
/// answer. Deliberate for the fold; closed by the shared evaluator.
#[test]
fn a_while_is_run_by_the_interpreter_and_declined_by_the_evaluator() {
    assert_eq!(by_evaluator(WHILE, "count", &[4], FUEL), None);
    assert_eq!(by_interpreter(WHILE, "count", &[4], 1_000_000).unwrap(), Value::integer(4));
    assert!(has_call(&format!("{WHILE}\nstart() in 0 to 255: return count(4)\n")));
}

fn pick_source() -> String {
    "heavy(n in 0 to 9) in 0 to 99:\n  s in 0 to 99: 0\n  for i in 1 to n do\n    s +: i\n  return s\npick(n in 0 to 9) in 0 to 99:\n  return when n < 5 then 1 else heavy(n)\n".to_owned()
}

/// The least fuel with which `run` succeeds.
fn least_fuel(run: impl Fn(u64) -> bool) -> u64 {
    (1..10_000).find(|fuel| run(*fuel)).expect("succeeds with some fuel")
}

/// D2. Eager arms: the evaluator runs every node, so the call in the arm that is not taken is run
/// too, and costs fuel (and when it costs more than the budget the whole evaluation is abandoned).
/// The interpreter evaluates only the arm taken. Deliberate (an over-approximation of cost, never
/// of value).
#[test]
fn an_arm_not_taken_costs_the_evaluator_fuel_and_the_interpreter_nothing() {
    let source = pick_source();
    let evaluator = least_fuel(|fuel| by_evaluator(&source, "pick", &[3], fuel) == Some(1));
    let interpreter = least_fuel(|fuel| by_interpreter(&source, "pick", &[3], fuel).is_ok());
    assert!(interpreter < evaluator, "interpreter {interpreter}, evaluator {evaluator}");
    // With a budget in between, one finishes and the other gives up.
    let between = interpreter;
    assert_eq!(by_evaluator(&source, "pick", &[3], between), None);
    assert_eq!(by_interpreter(&source, "pick", &[3], between).unwrap(), Value::integer(1));
    // Negative control: when the arm is taken the interpreter pays for it as well.
    let taken = least_fuel(|fuel| by_interpreter(&source, "pick", &[9], fuel).is_ok());
    assert!(taken > interpreter, "{taken} {interpreter}");
    assert!(matches!(by_interpreter(&source, "pick", &[9], between), Err(Error::OutOfFuel(_))));
    assert_eq!(by_interpreter(&source, "pick", &[9], 1_000_000).unwrap(), Value::integer(45));
}

/// D16. Call depth: the evaluator follows 32 nested calls; the interpreter 256. Deliberate.
#[test]
fn a_call_chain_deeper_than_the_evaluator_allows_is_declined() {
    let mut source = "f0(n in 0 to 9) in 0 to 9: return n\n".to_owned();
    for k in 1..40 {
        source.push_str(&format!("f{k}(n in 0 to 9) in 0 to 9: return f{}(n)\n", k - 1));
    }
    assert_eq!(by_evaluator(&source, "f20", &[3], FUEL), Some(3));
    assert_eq!(by_evaluator(&source, "f39", &[3], FUEL), None);
    // The depth is the caller's parameter (E4): the fold passes 32, anything else may pass more.
    let module = checked(&source);
    let file = module.checked();
    let body = file.bodies[index(file, "f39")].as_ref().unwrap();
    let argument = [typecheck::Evaluated::Integer(i256::I256::from(3))];
    let deeper = typecheck::evaluate_value_with(body, &argument, &Table(file), &mut { FUEL }, 64);
    assert_eq!(deeper, Some(typecheck::Evaluated::Integer(i256::I256::from(3))));
    assert_eq!(typecheck::evaluate_value_with(body, &argument, &Table(file), &mut { FUEL }, typecheck::FOLD_DEPTH), None);
    assert_eq!(by_interpreter(&source, "f39", &[3], 1_000_000).unwrap(), Value::integer(3));
    assert!(has_call(&format!("{source}\nstart() in 0 to 255: return f39(3)\n")));
    assert!(!has_call(&format!("{source}\nstart() in 0 to 255: return f20(3)\n")));
}

/// D4. A call through a bound needs the type it was given: the evaluator has no type
/// environment and declines; the interpreter runs the template with one. Closed by the shared
/// evaluator (templates with a type environment).
#[test]
fn a_bounded_call_is_run_by_the_interpreter_and_declined_by_the_evaluator() {
    let source = "Measure has\n    measure(self) in 0 to 100\n\n#unrestricted\nSmall is in 0 to 9\nSmall has Measure\n    measure(self) in 0 to 100: return 5\n\nweigh(element has Measure, item element) in 0 to 100:\n    return element.measure(item)\n\n#entry\nstart() in 0 to 255:\n    s Small := 4\n    x in 0 to 100: weigh(Small, s)\n    return x\n";
    let module = checked(source);
    let file = module.checked();
    let weigh = file.bodies[index(file, "weigh")].as_ref().unwrap();
    assert!(weigh.expressions.iter().any(|e| matches!(e.kind, Kind::BoundCall { .. })));
    assert_eq!(evaluate_body(weigh, &[i256::I256::from(4)], &Table(file), &mut { FUEL }), None);
    let value = with_big_stack({
        let source = source.to_owned();
        move || {
            let program = Program::from_source(&source).unwrap();
            Interpreter::new(&program).run_entry().unwrap().value
        }
    });
    assert_eq!(value, Value::integer(5));
}

/// D6. A hole: the evaluator stops (the call stays a call, so a hole is never folded away); the
/// interpreter reports it with the call chain. They agree that nothing is computed.
#[test]
fn a_hole_is_never_folded_and_is_reported_by_the_interpreter() {
    let source = "pending(n in 0 to 9) in 0 to 9: return #todo\n";
    assert_eq!(by_evaluator(source, "pending", &[2], FUEL), None);
    assert!(matches!(by_interpreter(source, "pending", &[2], 1_000_000), Err(Error::HoleReached { .. })));
    assert!(has_call(&format!("{source}\nstart() in 0 to 255: return pending(2)\n")));
}

/// D11. Unit: the evaluator has no unit value, so a function that returns nothing has no
/// evaluated result; the interpreter has `Unit`. Deliberate for the fold (nothing to substitute).
#[test]
fn a_unit_result_exists_only_in_the_interpreter() {
    let source = "nothing(n in 0 to 9):\n    return\n";
    let module = checked(source);
    let file = module.checked();
    let body = file.bodies[index(file, "nothing")].as_ref().unwrap();
    assert!(evaluate_value(body, &[typecheck::Evaluated::Integer(i256::I256::from(1))], &Table(file), &mut { FUEL }).is_none());
    assert_eq!(by_interpreter(source, "nothing", &[1], 1_000_000).unwrap(), Value::Unit);
}

/// D8. An argument outside the declared domain: the evaluator declines (it filters the parameter
/// by its type); the interpreter names the error. They agree it is not a value.
#[test]
fn an_argument_outside_its_domain_is_declined_by_both() {
    let source = "f(n in 0 to 9) in 0 to 9: return n\n";
    assert_eq!(by_evaluator(source, "f", &[10], FUEL), None);
    assert!(matches!(by_interpreter(source, "f", &[10], 1_000_000), Err(Error::ArgumentOutOfDomain { .. })));
}

/// D2'. The fuel is per node in the evaluator (10 000 for a fold) and per node and pass in the
/// interpreter (50 000 000): the checker's work budget is not the interpreter's.
#[test]
fn the_fold_budget_is_the_checkers_and_not_the_interpreters() {
    let source = "total(n in 0 to 9) in 0 to 99:\n  s in 0 to 99: 0\n  for i in 1 to n do\n    s +: i\n  return s\n";
    assert_eq!(by_evaluator(source, "total", &[9], 5), None);
    assert_eq!(by_evaluator(source, "total", &[9], FUEL), Some(45));
}
