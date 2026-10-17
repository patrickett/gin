mod support;

use support::check_with;

const PRELUDE: &str = "Number is in 0 to 255\nNumber has finish(self): return\n";

fn reject(source: &str, name: &str) {
    let typed = check_with(PRELUDE, source);
    let flaws = typed.all_flaws();
    assert!(
        flaws.iter().any(|(_, flaw)| {
            flaw.code.slug() == "type-binding-shadows-visible-name"
                && flaw.arg("name") == Some(name)
        }),
        "{source}: {flaws:?}"
    );
}

fn accept(source: &str) {
    let typed = check_with(PRELUDE, source);
    assert!(
        typed.all_flaws().is_empty(),
        "{source}: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn fresh_declarations_reject_visible_mutable_immutable_and_unassigned_locals() {
    for first in ["value: 1", "value := 1", "value Number"] {
        for second in ["value: 2", "value := 2", "value Number"] {
            reject(
                &format!("main:\n    {first}\n    {second}\n    return 0\n"),
                "value",
            );
        }
    }
}

#[test]
fn nested_declarations_reject_visible_locals_and_parameters() {
    for source in [
        "main(flag is in 0 to 1):\n    value: 1\n    when flag = 1 then\n        value: 2\n        return value\n    return value\n",
        "main(value is in 0 to 255):\n    value: 2\n    return value\n",
        "main(value is in 0 to 255):\n    when value = 1 then\n        value := 2\n        return value\n    return value\n",
        "main:\n    value := 1\n    local(value is in 0 to 255): return value\n    return local(2)\n",
        "main(value is in 0 to 255):\n    local(other is in 0 to 255):\n        value: 2\n        return value\n    return local(0)\n",
        "main(value is in 0 to 255):\n    value(other is in 0 to 255): return other\n    return 0\n",
    ] {
        reject(source, "value");
    }
}

#[test]
fn loop_variables_and_bodies_cannot_hide_visible_bindings() {
    for iterable in ["0 to 3", "[1, 2]", "[]"] {
        reject(
            &format!(
                "main:\n    value: 42\n    for value in {iterable}\n        observed: value\n    return value\n"
            ),
            "value",
        );
        reject(
            &format!("main:\n    for value in {iterable}\n        value: 2\n    return 0\n"),
            "value",
        );
    }
    reject(
        "main(flag is in 0 to 1):\n    value: 42\n    while flag = 1 do\n        value: 2\n    return value\n",
        "value",
    );
    reject(
        "main(count is in 0 to 255, source ref [in 0 to 255; count]):\n    value: 42\n    for value in source\n        observed: value\n    return value\n",
        "value",
    );
}

#[test]
fn patterns_and_destructuring_cannot_hide_visible_bindings() {
    for source in [
        "Choice is Some(payload Number) or None\nChoice.finish(self): return when self is\n    Some(payload) then payload.finish\n    else return\nmain(choice Choice, value is in 0 to 255):\n    return when choice is\n        Some(value) then value\n        else 0\n",
        "Choice is Some(payload Number) or None\nChoice.finish(self): return when self is\n    Some(payload) then payload.finish\n    else return\nmain(choice Choice, value is in 0 to 255):\n    when choice is Some(value) then\n        return value\n    return 0\n",
        "Pair has\n    value in 0 to 255\nmain(pair Pair, value is in 0 to 255):\n    Pair(value) := pair\n    return value\n",
        "Choice is Some(payload Number) or None\nChoice.finish(self): return when self is\n    Some(payload) then payload.finish\n    else return\nmain(choice Choice) Number:\n    return when choice is\n        Some(value) then\n            value: 2\n            value\n        else 0\n",
    ] {
        reject(source, "value");
    }
}

#[test]
fn generic_bodies_are_checked_before_specialization() {
    reject(
        "identity(value):\n    value: 2\n    return value\n",
        "value",
    );
}

#[test]
fn method_bodies_cannot_hide_parameters_but_can_use_field_names() {
    reject(
        "Record has\n    inspect(self ref, value is in 0 to 255):\n        value: 2\n        return value\n    finish(self): return\n",
        "value",
    );
    accept(
        "Record has\n    value Number\n    inspect(self ref):\n        value is in Number: self.value\n        return value\n    finish(self):\n        self.value.finish\n        return\n",
    );
}

#[test]
fn disjoint_branches_loops_functions_and_later_bindings_can_reuse_names() {
    for source in [
        "main(flag is in 0 to 1):\n    when flag = 1 then\n        value := 7\n        return value\n    value := 0\n    return value\n",
        "main(flag is in 0 to 1): return when flag is\n    0 then\n        value := 7\n        value\n    else\n        value := 0\n        value\n",
        "main:\n    for value in [1, 2]\n        observed: value\n    for value in 0 to 3\n        observed: value\n    value: 42\n    return value\n",
        "first(value is in 0 to 255): return value\nsecond(value is in 0 to 255): return value\n",
        "main:\n    first(value is in 0 to 255): return value\n    second(value is in 0 to 255): return value\n    return first(1) + second(2)\n",
    ] {
        accept(source);
    }
}

#[test]
fn pattern_names_can_be_reused_in_disjoint_arms_and_alternatives() {
    for source in [
        "Choice is First(payload Number) or Second(payload Number)\nChoice.finish(self): return when self is\n    First(payload) then payload.finish\n    Second(payload) then payload.finish\nmain(choice Choice) Number: return when choice is\n    First(value) then value\n    Second(value) then value\n",
        "Choice is First(payload Number) or Second(payload Number)\nChoice.finish(self): return when self is\n    First(payload) then payload.finish\n    Second(payload) then payload.finish\nmain(choice Choice) Number:\n    when choice is First(value) or choice is Second(value) then\n        return value\n    return 0\n",
    ] {
        accept(source);
    }
}

#[test]
fn a_binding_is_not_visible_in_its_own_initializer() {
    accept(
        "Choice is Some(payload Number) or None\nChoice.finish(self): return when self is\n    Some(payload) then payload.finish\n    else return\nmain(choice Choice) Number:\n    value: when choice is\n        Some(value) then value\n        else 0\n    return value\n",
    );
}

#[test]
fn repeated_pattern_literals_do_not_declare_bindings() {
    accept(
        "Choice is Pair(first Number, second Number) or None\nChoice.finish(self): return when self is\n    Pair(first, second) then\n        first.finish\n        second.finish\n        return\n    else return\nmain(choice ref Choice) Number: return when choice is\n    Pair(0, 0) then 1\n    else 2\n",
    );
}

#[test]
fn rebind_and_compound_updates_preserve_the_existing_binding() {
    accept(
        "main:\n    value is in 0 to 255: 1\n    value :: 2\n    value +: 1\n    when value = 3 then\n        value :: 4\n    return value\n",
    );
    accept("set(value mut in 0 to 255):\n    value :: 2\n    return\n");
}

#[test]
fn argument_and_sequence_labels_do_not_declare_local_bindings() {
    for source in [
        "take(value is in 0 to 255): return value\nmain:\n    value: 7\n    result: take(value: value)\n    return result\n",
        "Record has\n    value Number\n    finish(self):\n        self.value.finish\n        return\nmake(value Number) Record: return Record(value: value)\n",
        "main:\n    value: 7\n    row: [value: value]\n    return row.0\n",
    ] {
        accept(source);
    }
}

#[test]
fn initializer_blocks_have_their_own_scope() {
    accept(
        "main:\n    result:\n        value := 7\n        return value\n    value := 9\n    return value\n",
    );
    reject(
        "main:\n    value: 7\n    result:\n        value := 9\n        return value\n    return result\n",
        "value",
    );
}

#[test]
fn diagnostics_point_to_both_declarations() {
    let source = "main:\n    value: 1\n    value: 2\n    return value\n";
    let prefix_length = PRELUDE.len();
    let typed = check_with(PRELUDE, source);
    let flaws = typed.all_flaws();
    let conflicts: Vec<_> = flaws
        .iter()
        .filter(|(_, flaw)| flaw.code.slug() == "type-binding-shadows-visible-name")
        .collect();
    assert_eq!(conflicts.len(), 1, "{flaws:?}");
    let diagnostic = conflicts[0].1;
    assert_eq!(
        diagnostic.span.start(),
        prefix_length + source.rfind("value: 2").unwrap()
    );
    assert_eq!(diagnostic.related().len(), 1);
    assert_eq!(
        diagnostic.related()[0].span.start(),
        prefix_length + source.find("value: 1").unwrap()
    );
    assert!(diagnostic.help.as_ref().unwrap().contains("::"));
}
