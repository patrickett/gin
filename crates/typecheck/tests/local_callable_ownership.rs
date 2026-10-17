mod support;

use support::check_lexed_with;

const PRELUDE: &str = "Number is in 0 to 255\nNumber.finish(self): return\n";

#[test]
fn owned_local_arguments_cannot_be_used_after_the_call() {
    let typed = check_lexed_with(
        PRELUDE,
        "outer(value Number):\n    take(item Number): return 0\n    take(value)\n    return value\n",
    );
    assert!(
        typed.all_flaws().iter().any(|(_, flaw)| matches!(
            flaw.code.slug().as_str(),
            "type-use-of-moved-value" | "type-use-after-move"
        )),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn inspected_local_arguments_remain_available_after_the_call() {
    let typed = check_lexed_with(
        PRELUDE,
        "outer(value Number):\n    inspect(item ref Number): return 0\n    inspect(value)\n    return value\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn local_call_cannot_consume_the_same_runtime_value_twice() {
    let typed = check_lexed_with(
        PRELUDE,
        "outer(value Number):\n    take(first Number, second Number): return 0\n    take(value, value)\n    return 0\n",
    );
    assert!(
        typed.all_flaws().iter().any(|(_, flaw)| matches!(
            flaw.code.slug().as_str(),
            "type-use-of-moved-value" | "type-use-after-move"
        )),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn local_call_cannot_consume_storage_borrowed_by_another_argument() {
    let typed = check_lexed_with(
        PRELUDE,
        "outer(value Number):\n    take(first Number, second ref Number): return 0\n    take(value, value)\n    return 0\n",
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-overlapping-target-group-arguments"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn local_owned_calls_can_rematerialize_singleton_integers() {
    let typed = check_lexed_with(
        PRELUDE,
        "outer():\n    take(item x): return 0\n    value: 7\n    take(value)\n    take(value)\n    return 0\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}
