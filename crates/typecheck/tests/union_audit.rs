//! Nested-pattern and union regressions with exact diagnostic-code negative controls.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_file_diagnostic, check_module};

/// The checker's answer: `Ok`, a parse failure (as `Malformed` plus the parser's message), or
/// the error with its detail.
fn run(text: &str) -> Result<(), (SourceCheckError, String)> {
    let file = parser::parse_strict(text)
        .map_err(|d| (SourceCheckError::Malformed, format!("{d:?}")))?;
    check_file_diagnostic(&file, Limits::default())
        .map(|_| ())
        .map_err(|f| (f.error, f.detail.unwrap_or_default()))
}

fn code(text: &str) -> SourceCheckError {
    run(text).expect_err(text).0
}

const HEAD: &str = "\
#unrestricted
Code is in 0 to 9

#unrestricted
AllocError is OutOfMemory or Fragmented

#unrestricted
Outcome is Value(Code) or Flaw(AllocError) or Other(AllocError)

";

fn outcome(arms: &str) -> String {
    format!("{HEAD}f(r Outcome, n in 0 to 9) in 0 to 9: return when r is\n{arms}")
}

const FULL: &str = "    Value(_) then 0\n    Flaw(OutOfMemory) then 1\n    Flaw(Fragmented) then 2\n    Other(_) then 3\n";

#[test]
fn nested_negative_controls_name_their_reason() {
    assert_eq!(run(&outcome(FULL)), Ok(()));
    // A wrong inner variant name is an unknown name, not a missing arm.
    assert_eq!(code(&outcome(&FULL.replace("Flaw(Fragmented)", "Flaw(Nonexistent)"))), SourceCheckError::UnknownName);
    // A whole-payload arm beside an arm inside the same payload names a variant twice.
    assert_eq!(code(&outcome(&format!("{FULL}    Flaw(other) then 4\n"))), SourceCheckError::Duplicate);
    // The same leaf twice.
    assert_eq!(code(&outcome(&format!("{FULL}    Flaw(Fragmented) then 4\n"))), SourceCheckError::Duplicate);
    // A missing leaf.
    assert_eq!(code(&outcome(&FULL.replace("    Flaw(Fragmented) then 2\n", ""))), SourceCheckError::Type);
    // Another outer variant holding the same inner union is covered separately.
    assert_eq!(code(&outcome(&FULL.replace("    Other(_) then 3\n", "    Other(OutOfMemory) then 3\n"))), SourceCheckError::Type);
}

#[test]
fn a_depth_three_pattern_is_covered_leaf_by_leaf() {
    let types = "#unrestricted\nC is in 0 to 9\n#unrestricted\nI is Low or High(C)\n#unrestricted\nS is Full(I) or Empty\n#unrestricted\nH is Hit(S) or Miss\n";
    let arms = "    Miss then 0\n    Hit(Empty) then 1\n    Hit(Full(Low)) then 2\n    Hit(Full(High(v))) then v\n";
    let f = |arms: &str| format!("{types}f(h H) in 0 to 9: return when h is\n{arms}");
    assert_eq!(run(&f(arms)), Ok(()));
    assert_eq!(code(&f(&arms.replace("    Hit(Full(High(v))) then v\n", ""))), SourceCheckError::Type);
    assert_eq!(code(&f(&arms.replace("Hit(Full(High(v)))", "Hit(Full(Nope(v)))"))), SourceCheckError::UnknownName);
    assert_eq!(code(&f(&format!("{arms}    Hit(Full(Low)) then 4\n"))), SourceCheckError::Duplicate);
}

#[test]
fn nested_patterns_stay_out_of_destructures_and_one_line_tests() {
    let destructure = format!("{HEAD}h(r Outcome):\n    Flaw(OutOfMemory) := r\n    return\n");
    let (error, message) = run(&destructure).unwrap_err();
    assert_eq!(error, SourceCheckError::Malformed);
    assert!(message.contains("nested patterns are unsupported"), "{message}");
    let one_line = format!("{HEAD}h(r Outcome) in 0 to 1: return when r is Flaw(OutOfMemory) then 1 else 0\n");
    let (error, message) = run(&one_line).unwrap_err();
    assert_eq!(error, SourceCheckError::Malformed);
    assert!(message.contains("nested patterns are unsupported"), "{message}");
    // Negative control: the flat spelling of the same one-line test parses and checks.
    assert_eq!(run(&format!("{HEAD}h(r Outcome) in 0 to 1: return when r is Flaw(_) then 1 else 0\n")), Ok(()));
}

#[test]
fn the_documented_limits_of_a_nested_pattern_are_refused_or_unsupported() {
    let pair = format!("{HEAD}Pair is Both(a AllocError, b AllocError) or None\n");
    // Arms of one variant must nest at the same field.
    let (error, message) = run(&format!("{pair}g(p Pair) in 0 to 3: return when p is\n    None then 0\n    Both(OutOfMemory: a, _: b) then 1\n    Both(_: a, Fragmented: b) then 2\n")).unwrap_err();
    assert_eq!(error, SourceCheckError::Unsupported);
    assert!(message.contains("same place"), "{message}");
    // One nested position per pattern: two inner variants in one arm are refused. Today this is
    // reported as `Duplicate` (the second position repeats the first variant name), not as
    // `Unsupported`, which says less than the documented limit; pinned as it is.
    assert_eq!(code(&format!("{pair}g(p Pair) in 0 to 3: return when p is\n    None then 0\n    Both(OutOfMemory: a, OutOfMemory: b) then 1\n")), SourceCheckError::Duplicate);
    // Negative control: nesting in one position only is accepted.
    assert_eq!(run(&format!("{pair}g(p Pair) in 0 to 3: return when p is\n    None then 0\n    Both(OutOfMemory: a, _: b) then 1\n    Both(Fragmented: a, _: b) then 2\n")), Ok(()));
}

const LINEAR: &str = "Token has n in 0 to 3\n\nHeld is Some(Token) or None\n\nWrapped is Inside(Held) or Outside\n\n";

#[test]
fn a_linear_payload_in_a_nested_pattern_is_consumed_exactly_once_by_every_arm() {
    let take = |arms: &str| format!("{LINEAR}take(w Wrapped, k in 0 to 3) in 0 to 3: return when w is\n    Outside then 0\n{arms}    Inside(None) then 1\n");
    let consume = "        Token(n) := token\n        return n\n";
    let guarded_both = format!("    Inside(Some(token)) and k > 1 then\n{consume}    Inside(Some(token)) then\n{consume}");
    assert_eq!(run(&take(&guarded_both)), Ok(()));
    // Consumed twice (the exact reason: a second use of a moved value).
    let twice = format!("    Inside(Some(token)) then\n        Token(n) := token\n        Token(m) := token\n        return n\n");
    assert_eq!(code(&take(&twice)), SourceCheckError::UseAfterMove);
    // The guarded arm drops the payload.
    let guarded_leaks = format!("    Inside(Some(token)) and k > 1 then 2\n    Inside(Some(token)) then\n{consume}");
    assert_eq!(code(&take(&guarded_leaks)), SourceCheckError::Unconsumed);
    // The fallback arm drops it.
    let fallback_leaks = format!("    Inside(Some(token)) and k > 1 then\n{consume}    Inside(Some(token)) then 1\n");
    assert_eq!(code(&take(&fallback_leaks)), SourceCheckError::Unconsumed);
    // A wildcard does not consume the linear payload either.
    assert_eq!(code(&take("    Inside(Some(_)) then 2\n")), SourceCheckError::Unconsumed);
}

#[test]
fn a_guarded_nested_arm_over_an_imported_union_needs_its_fallback() {
    let library_text = format!("{HEAD}classify(n in 0 to 9) Outcome:\n    when n < 3 then\n        return Value(n as Code)\n    return Flaw(OutOfMemory)\n");
    let library_file = parser::parse_strict(&library_text).unwrap();
    let library: CheckedSourceModule = check_module("lib.outcome", &library_file, &[], Limits::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let client = |arms: &str| format!("use lib.outcome Outcome, classify\n\nrank(n in 0 to 9) in 0 to 9: return when classify(n) is\n    Value(v) then v\n{arms}    Other(_) then 3\n");
    let check = |text: &str| {
        let file = parser::parse_strict(text).unwrap();
        check_module("app.client", &file, &[&library], Limits::default()).map(|_| ())
    };
    let ok = "    Flaw(OutOfMemory) and n > 7 then 9\n    Flaw(OutOfMemory) then 4\n    Flaw(Fragmented) then 2\n";
    assert_eq!(check(&client(ok)), Ok(()));
    // Without the unguarded arm the guarded one covers nothing.
    let no_fallback = "    Flaw(OutOfMemory) and n > 7 then 9\n    Flaw(Fragmented) then 2\n";
    assert_eq!(check(&client(no_fallback)), Err(SourceCheckError::Type));
    // The imported binder keeps its range: `v` is a `Code`, so `v + 1` can reach 10.
    assert!(check(&client(ok).replace("Value(v) then v", "Value(v) then v + 1")).is_err());
}

#[test]
fn there_is_no_reverse_constructor_for_a_discriminant() {
    let text = "LogLevel is Debug = 'debug' or Info = 'info'\n\n";
    // The one derived direction works.
    assert_eq!(run(&format!("{text}f() : return LogLevel.Debug.value\n")), Ok(()));
    // Neither a derived `from_value`, nor calling the union on a value, nor a value on a numeric union.
    assert_eq!(code(&format!("{text}f() LogLevel: return LogLevel.from_value('debug')\n")), SourceCheckError::UnknownName);
    assert_eq!(code(&format!("{text}f() LogLevel: return LogLevel('debug')\n")), SourceCheckError::Type);
    assert_eq!(code("Exit is Success = 0 or Failure = 1\nf() Exit: return Exit(1)\n"), SourceCheckError::Type);
    // A pattern over the literal is a parse error, not a reverse match.
    let (error, message) = run(&format!("{text}f(l LogLevel): return when 'debug' is\n    LogLevel.Debug then ()\n")).unwrap_err();
    assert_eq!(error, SourceCheckError::Malformed);
    assert!(message.contains("computed patterns are unsupported"), "{message}");
}

#[test]
fn unrestricted_is_refused_on_a_union_with_a_linear_payload_and_names_it() {
    let (error, message) = run("Res has v in 0 to 9\n#unrestricted\nU is A(Res) or B\n").unwrap_err();
    assert_eq!(error, SourceCheckError::UnrestrictedField);
    assert!(message.contains("`U`") && message.contains("Res"), "{message}");
    // Negative control: the same union is accepted once the payload is unrestricted too.
    assert_eq!(run("#unrestricted\nRes has v in 0 to 9\n#unrestricted\nU is A(Res) or B\n"), Ok(()));
    // And a payload-less union is linear unless marked, for a parameter used twice.
    let base = "Register is Low or High\n\nshow(r Register):\n    code in 0 to 1: when r is\n        Low then 0\n        High then 1\n    return\n\n";
    assert_eq!(code(&format!("{base}twice(r Register):\n    show(r)\n    show(r)\n    return\n")), SourceCheckError::UseAfterMove);
    assert_eq!(run(&format!("#unrestricted\n{base}twice(r Register):\n    show(r)\n    show(r)\n    return\n")), Ok(()));
}

#[test]
fn a_local_bound_from_a_known_variant_of_a_linear_union_is_linear() {
    let base = "Register is Low or High\n\nshow(r Register):\n    code in 0 to 1: when r is\n        Low then 0\n        High then 1\n    return\n\n";
    assert_eq!(code(&format!("{base}twice():\n    r Register: Register.High\n    show(r)\n    show(r)\n    return\n")), SourceCheckError::UseAfterMove);
    assert_eq!(code(&format!("{base}dropped():\n    r Register: Register.High\n    return\n")), SourceCheckError::Unconsumed);
}

/// Controls for the test above: `#unrestricted` stays copyable and droppable, and a `=`
/// (Discriminants) union follows the same rule as a plain one.
#[test]
fn a_known_variant_of_an_unrestricted_union_stays_free_and_discriminants_follow_the_rule() {
    let base = "Register is Low or High\n\nshow(r Register):\n    code in 0 to 1: when r is\n        Low then 0\n        High then 1\n    return\n\n";
    let free = format!("#unrestricted\n{base}twice():\n    r Register: Register.High\n    show(r)\n    show(r)\n    return\n");
    assert_eq!(run(&free), Ok(()));
    let dropped = format!("#unrestricted\n{base}dropped():\n    r Register: Register.High\n    return\n");
    assert_eq!(run(&dropped), Ok(()));
    let coded = "ExitCode is Success = 0 or Failure = 1\n\nshow(c ExitCode):\n    code in 0 to 1: when c is\n        Success then 0\n        Failure then 1\n    return\n\n";
    assert_eq!(code(&format!("{coded}twice():\n    c ExitCode: ExitCode.Failure\n    show(c)\n    show(c)\n    return\n")), SourceCheckError::UseAfterMove);
    assert_eq!(code(&format!("{coded}dropped():\n    c ExitCode: ExitCode.Failure\n    return\n")), SourceCheckError::Unconsumed);
    assert_eq!(run(&format!("#unrestricted\n{coded}twice():\n    c ExitCode: ExitCode.Failure\n    show(c)\n    show(c)\n    return\n")), Ok(()));
}

/// BUG (found 2026-11-05, not fixed): a flat guarded arm after the unguarded arm of its member is
/// refused (`Duplicate`), but the same order for a nested arm is accepted by the checker, and
/// the native recorder then declines the function (see `nested_patterns_runtime`,
/// `guarded_arms_out_of_order_are_known_lowering_gaps`).
#[test]
fn a_guarded_nested_arm_after_its_unguarded_arm_is_refused_like_a_flat_one() {
    let flat = "#unrestricted\nCode is in 0 to 9\n\n#unrestricted\nO is Done or Failure(Code)\n\nf(o O) in 0 to 2: when o is\n    Done then 0\n    Failure(c) then 2\n    Failure(c) and c < 5 then 1\n";
    assert_eq!(code(flat), SourceCheckError::Duplicate);
    assert_eq!(code(&outcome("    Value(_) then 0\n    Flaw(OutOfMemory) then 4\n    Flaw(OutOfMemory) and n > 3 then 1\n    Flaw(Fragmented) then 2\n    Other(_) then 3\n")), SourceCheckError::Duplicate);
}

#[test]
fn a_guarded_arm_not_next_to_its_fallback_is_refused_flat_and_nested() {
    // Decided 2026-11-05 (B2): a guarded arm sits directly before the unguarded fallback of its
    // member. The checker refuses every other order, so lowering never sees one.
    let apart = "    Value(_) then 0\n    Flaw(OutOfMemory) and n > 3 then 1\n    Other(_) then 3\n    Flaw(OutOfMemory) then 4\n    Flaw(Fragmented) then 2\n";
    assert_eq!(code(&outcome(apart)), SourceCheckError::Duplicate);
    // Negative control: the adjacent order checks.
    let adjacent = "    Value(_) then 0\n    Flaw(OutOfMemory) and n > 3 then 1\n    Flaw(OutOfMemory) then 4\n    Flaw(Fragmented) then 2\n    Other(_) then 3\n";
    assert_eq!(run(&outcome(adjacent)), Ok(()));
    // Flat: a guarded arm separated from its fallback by another variant.
    let flat = "#unrestricted\nCode is in 0 to 9\n\n#unrestricted\nO is Done or Failure(Code)\n\nf(o O) in 0 to 2: when o is\n    Failure(c) and c < 5 then 1\n    Done then 0\n    Failure(c) then 2\n";
    assert_eq!(code(flat), SourceCheckError::Duplicate);
    let flat_ok = "#unrestricted\nCode is in 0 to 9\n\n#unrestricted\nO is Done or Failure(Code)\n\nf(o O) in 0 to 2: when o is\n    Done then 0\n    Failure(c) and c < 5 then 1\n    Failure(c) then 2\n";
    assert_eq!(run(flat_ok), Ok(()));
}

#[test]
fn variant_claims_name_the_reason_they_fail() {
    let claimed = "#unrestricted\nCode is in 0 to 9\n\ncheck(n in 0 to 9) Fine where (n < 5) or Failed(Code) where (n >= 5):\n    when n < 5 then\n        return Fine\n    return Failed(n as Code)\n";
    assert_eq!(run(claimed), Ok(()));
    assert_eq!(code(&claimed.replace("Fine where (n < 5)", "Fine where (n < 3)")), SourceCheckError::Unproved);
    assert_eq!(code(&claimed.replace("Failed(Code) where (n >= 5)", "Failed(Code) where (n >= 7)")), SourceCheckError::Unproved);
    assert_eq!(code(&claimed.replace("(n >= 5)", "(zz >= 5)")), SourceCheckError::UnknownName);
}

const COUNTER: &str = "#unrestricted\nCounter has\n    n in 0 to 100\n\nbump(c mut Counter and c.n < 100) Grown where (c.n :: c.n + 1) or Kept where (c.n :: c.n):\n    when c.n < 50 then\n        c.n :: c.n + 1\n        return Grown\n    return Kept\n";

#[test]
fn a_transition_claim_is_integer_places_only_and_a_lie_is_unproved() {
    assert_eq!(run(COUNTER), Ok(()));
    assert_eq!(code(&COUNTER.replace("        c.n :: c.n + 1\n", "        c.n :: c.n + 2\n")), SourceCheckError::Unproved);
    // Whole-record `c :: c` is Unsupported (documented), not silently accepted.
    let whole = COUNTER.replace("Grown where (c.n :: c.n + 1) or Kept where (c.n :: c.n)", "Grown where (c.n :: c.n + 1) or Kept where (c :: c)");
    assert_eq!(code(&whole), SourceCheckError::Unsupported);
}

#[test]
fn a_provider_that_drops_the_traits_parameter_is_refused_for_that_reason() {
    // The older tests also changed the call site; here only the provider differs.
    let generic = "#unrestricted\nCode is in 0 to 9\n\nMaker has\n    make(self, element, value element) Made(element) or Empty\n\n#unrestricted\nPlain has Maker\n    k in 0 to 9\n\n    make(self, element, value element) Made(element) or Empty:\n        return Made(value)\n";
    assert_eq!(run(generic), Ok(()));
    let wrong = generic.replace("    make(self, element, value element) Made(element) or Empty:\n        return Made(value)", "    make(self, value Code) Made(Code) or Empty:\n        return Empty");
    let (error, message) = run(&wrong).unwrap_err();
    assert_eq!(error, SourceCheckError::Type);
    assert!(message.contains("type parameters"), "{message}");
    let params = "#unrestricted\nCode is in 0 to 9\n\n#unrestricted\nWide is in 0 to 99\n\nMaker(payload) has\n    make(self, value payload) Made(payload) or Empty\n\n#unrestricted\nPlain has Maker(Code)\n    k in 0 to 9\n\n    make(self, value Code) Made(Code) or Empty:\n        return Made(value)\n";
    assert_eq!(run(params), Ok(()));
    let (error, message) = run(&params.replace("Made(Code) or Empty:", "Made(Wide) or Empty:")).unwrap_err();
    assert_eq!(error, SourceCheckError::Type);
    assert!(message.contains("payload type of `Made` differs"), "{message}");
    // A provision that does not give the trait its parameter.
    assert_eq!(code(&params.replace("Plain has Maker(Code)", "Plain has Maker")), SourceCheckError::Type);
}

#[test]
fn the_unit_value_in_a_returning_function_is_not_a_return() {
    // `()` as an arm value is a value; `then return` is the early exit and is only needed
    // where the function leaves before its end.
    let t = "#unrestricted\nT is A or B\n\n";
    assert_eq!(run(&format!("{t}f(t T): return when t is\n    A then ()\n    B then ()\n")), Ok(()));
    assert_eq!(run(&format!("{t}f(t T):\n    when t is\n        A then return\n        B then ()\n    return\n")), Ok(()));
    // Negative control: a unit arm beside a numeric one in a numeric result.
    assert_eq!(code(&format!("{t}f(t T) in 0 to 1: return when t is\n    A then ()\n    B then 1\n")), SourceCheckError::Type);
}
