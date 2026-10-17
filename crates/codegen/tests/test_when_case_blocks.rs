mod support;

use codegen::NativeEngine;

fn engine(body: &str) -> NativeEngine {
    let source = format!("Byte is in 0 to 255\nByte.finish(self): return\n{body}");
    support::native_engine_for_source(&source, "when_blocks.gin", "choose")
}

#[allow(unsafe_code)]
fn check(body: &str, cases: &[(u8, u8)]) {
    let engine = engine(body);
    for (input, expected) in cases {
        let mut input = *input;
        let mut result = 0u8;
        unsafe {
            engine.invoke_packed(&mut [
                (&mut input as *mut u8).cast(),
                (&mut result as *mut u8).cast(),
            ])
        }
        .unwrap();
        assert_eq!(result, *expected);
    }
}

#[test]
fn value_blocks_and_function_returns_have_distinct_boundaries() {
    check(
        "choose(captured Byte) Byte:\n    value: when\n        captured < 4 then\n            adjusted Byte: 7\n            adjusted\n        else\n            captured\n    return value\n",
        &[(2, 7), (8, 8)],
    );
    check(
        "choose(captured Byte) Byte:\n    value: when captured < 4 then\n        return 9\n    else\n        captured\n    return 5\n",
        &[(2, 9), (8, 5)],
    );
    check(
        "choose(captured Byte) Byte:\n    value Byte:\n        return 9\n    return 5\n",
        &[(2, 9), (8, 9)],
    );
}

#[test]
fn effect_cases_can_fall_through_and_exhaustive_cases_can_return() {
    check(
        "choose(captured Byte) Byte:\n    when captured < 4 then return 9\n    return 5\n",
        &[(2, 9), (8, 5)],
    );
    check(
        "choose(captured Byte) Byte:\n    when captured\n        < 4 then return 1\n        = 4 then return 2\n        else return 3\n",
        &[(2, 1), (4, 2), (8, 3)],
    );
}

#[test]
fn guarded_alternatives_and_chained_patterns_bind_values() {
    let body = "Choice is First(Byte) or Second(Byte) or Neither\nChoice.finish(self): return\nchoose(captured Byte) Byte:\n    return when Choice.First(captured) is\n        First(value) or Second(value) and value < 4 then value\n        First(value) then 7\n        Second(value) then 8\n        Neither then 0\n";
    check(body, &[(2, 2), (8, 7)]);
    check(
        &body.replace("Choice.First(captured)", "Choice.Second(captured)"),
        &[(2, 2), (8, 8)],
    );
    check(
        "Choice is First(Byte) or Second(Byte) or Neither\nChoice.finish(self): return\nchoose(captured Byte) Byte: return when\n    Choice.First(captured) is First(value)\n        and Choice.Second(value) is Second(final)\n        and final < 4\n    then final\n    else 7\n",
        &[(2, 2), (8, 7)],
    );
}

#[test]
fn local_callables_have_their_own_return_boundary() {
    check(
        "choose(captured Byte) Byte:\n    local(value Byte) Byte:\n        when value < 4 then return 9\n        return value\n    value: local(captured)\n    return 5\n",
        &[(2, 5), (8, 5)],
    );
    check(
        "choose(captured Byte) Byte:\n    local(value Byte) Byte:\n        when value < 4 then return 9\n        return value\n    return local(captured)\n",
        &[(2, 9), (8, 8)],
    );
}

#[test]
fn a_return_in_a_call_argument_skips_the_call() {
    check(
        "discard(value Byte) Byte: return 5\nchoose(captured Byte) Byte: return discard(when captured < 4 then\n    return 9\nelse\n    captured\n)\n",
        &[(2, 9), (8, 5)],
    );
}

#[test]
fn nested_is_guards_and_constructor_patterns_fall_through() {
    let declarations = "Inner is Left(Byte) or Right(Byte)\nInner.finish(self): return\nOuter is Some(Inner) or None\nOuter.finish(self):\n    when self is\n        Some(value) then\n            value.finish\n            return\n        None then return\n";
    let grouped = "choose(captured Byte) Byte: return when Outer.Some(Inner.Left(captured)) is\n    Some(x) and x is\n        Left(y) and y < 4 then\n            adjusted: y\n            adjusted\n        Right(y) and y < 4 then y\n    Some(_) then 7\n    None then 9\n";
    check(&format!("{declarations}{grouped}"), &[(2, 2), (8, 7)]);
    check(
        &format!(
            "{declarations}{}",
            grouped.replace("Inner.Left(captured)", "Inner.Right(captured)")
        ),
        &[(2, 2), (8, 7)],
    );
    let flat = "choose(captured Byte) Byte: return when Outer.Some(Inner.Left(captured)) is\n    Some(Left(y)) and y < 4 then y\n    Some(_) then 7\n    None then 9\n";
    check(&format!("{declarations}{flat}"), &[(2, 2), (8, 7)]);
}

#[test]
fn nested_coverage_and_committed_zero_results_reach_native_code() {
    check(
        "Inner is Left(Byte) or Right(Byte)\nInner.finish(self): return\nOuter is Some(Inner) or None\nOuter.finish(self):\n    when self is\n        Some(value) then\n            value.finish\n            return\n        None then return\nchoose(captured Byte) Byte: return when Outer.Some(Inner.Left(captured)) is\n    Some(x) and x is\n        Left(y) then\n            y.finish\n            0\n        Right(y) then y\n    None then 9\n",
        &[(2, 0), (8, 0)],
    );
}
