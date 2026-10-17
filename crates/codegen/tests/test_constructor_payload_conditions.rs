#![cfg(any(
    all(target_arch = "aarch64", target_os = "macos"),
    all(target_arch = "x86_64", target_os = "linux")
))]

mod support;

const SOURCE: &str = "Word is in -9223372036854775808 to 9223372036854775807\nWord.finish(self):\n    return\nChoice is Some(value Word) or None\nChoice.finish(self): return when self is Some(value) then value.finish else idle\nidle:\n    return\nselect(value Word) Word:\n    wrapped Choice: Some(value)\n    result Word: when wrapped is Some(0) then 1 else 2\n    wrapped.finish\n    return result\n";

#[test]
#[allow(unsafe_code)]
fn constructor_payload_literal_controls_runtime_selection() {
    let engine = support::native_engine_for_source(SOURCE, "payload_conditions.gin", "select");
    for (mut input, expected) in [(0i64, 1i64), (7, 2), (-1, 2)] {
        let mut result = -1i64;
        unsafe {
            engine.invoke_packed(&mut [
                (&mut input as *mut i64).cast(),
                (&mut result as *mut i64).cast(),
            ])
        }
        .unwrap();
        assert_eq!(result, expected, "payload={input}");
    }
}

#[test]
#[allow(unsafe_code)]
fn repeated_literal_arguments_check_every_payload_position() {
    let source = "Word is in -9223372036854775808 to 9223372036854775807\nWord.finish(self):\n    return\nChoice is Pair(first Word, second Word) or None\nChoice.finish(self): return when self is Pair(first, second) then finish_words(first, second) else idle\nfinish_words(first Word, second Word):\n    first.finish\n    second.finish\n    return\nidle:\n    return\nselect(value Word) Word:\n    wrapped Choice: Pair(0, value)\n    result Word: when wrapped is Pair(0, 0) then 1 else 2\n    wrapped.finish\n    return result\n";
    let engine = support::native_engine_for_source(source, "payload_conditions.gin", "select");
    for (mut input, expected) in [(0i64, 1i64), (7, 2), (-1, 2)] {
        let mut result = -1i64;
        unsafe {
            engine.invoke_packed(&mut [
                (&mut input as *mut i64).cast(),
                (&mut result as *mut i64).cast(),
            ])
        }
        .unwrap();
        assert_eq!(result, expected, "second payload={input}");
    }
}

#[test]
#[allow(unsafe_code)]
fn binding_after_repeated_literals_uses_its_argument_position() {
    let source = "Word is in -9223372036854775808 to 9223372036854775807\nWord.finish(self):\n    return\nChoice is Many(first Word, second Word, third Word) or None\nChoice.finish(self): return when self is Many(first, second, third) then finish_many(first, second, third) else idle\nfinish_many(first Word, second Word, third Word):\n    first.finish\n    second.finish\n    third.finish\n    return\nidle:\n    return\nread_value(value ref Word) is in Word:\n    observed is in Word: value\n    return observed\nselect(value Word) Word:\n    wrapped Choice: Many(0, 0, value)\n    result Word: when ref wrapped is Many(0, 0, last) then read_value(last) else 0\n    wrapped.finish\n    return result\n";
    let engine = support::native_engine_for_source(source, "payload_conditions.gin", "select");
    for mut input in [0i64, 7, -1] {
        let mut result = 99i64;
        unsafe {
            engine.invoke_packed(&mut [
                (&mut input as *mut i64).cast(),
                (&mut result as *mut i64).cast(),
            ])
        }
        .unwrap();
        assert_eq!(result, input);
    }
}

#[test]
#[allow(unsafe_code)]
fn payload_conditions_do_not_match_an_inactive_variant() {
    let source = "Word is in -9223372036854775808 to 9223372036854775807\nWord.finish(self):\n    return\nChoice is Some(value Word) or None\nChoice.finish(self): return when self is Some(value) then value.finish else idle\nidle:\n    return\ndiscard(value Word) Choice:\n    value.finish\n    return None\nselect(value Word, enabled Word) Word:\n    wrapped Choice: when enabled is 0 then discard(value) else Some(value)\n    enabled.finish\n    result Word: when wrapped is Some(0) then 1 else 2\n    wrapped.finish\n    return result\n";
    let engine = support::native_engine_for_source(source, "payload_conditions.gin", "select");
    for (mut input, mut enabled, expected) in [(0i64, 0i64, 2i64), (0, 1, 1), (7, 1, 2), (7, 0, 2)]
    {
        let mut result = -1i64;
        unsafe {
            engine.invoke_packed(&mut [
                (&mut input as *mut i64).cast(),
                (&mut enabled as *mut i64).cast(),
                (&mut result as *mut i64).cast(),
            ])
        }
        .unwrap();
        assert_eq!(result, expected, "payload={input}, enabled={enabled}");
    }
}
