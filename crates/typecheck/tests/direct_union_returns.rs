mod support;

use support::package_flaws;

const TYPES: &str = "Word is in 0 to 255\nWord.finish(self):\n    return\nBlock(x) has value x\nBlock(x).finish(self):\n    return\nError is Empty or Full\nfinish_error(error Error):\n    return\n";

#[test]
fn generic_free_function_returns_each_direct_union_member() {
    let source = format!(
        "{TYPES}\nchoose(x, value x, flag Word) Block(x) or Error:\n    n is in Word: flag\n    when n = 0 then\n        return Empty\n    return Block(value)\nrun:\n    result: choose(Word, 7 as Word, 1 as Word)\n    when result is\n        Block(block) then block.finish\n        Error(error) then finish_error(error)\n    return\n"
    );
    let flaws = package_flaws("direct_results", &[("values", &source)]);
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn method_union_contract_is_preserved_in_implementations_and_forwarding() {
    let source = format!(
        "{TYPES}\nStore has\n    reserve(self mut, x, value x, flag Word) Block(x) or Error\nConcrete has Store\n    reserve(self mut, x, value x, flag Word): return choose(x, value, flag)\n    finish(self):\n        return\nchoose(x, value x, flag Word) Block(x) or Error:\n    n is in Word: flag\n    when n = 0 then\n        return Empty\n    return Block(value)\nrun:\n    store: Concrete()\n    result: store.reserve(Word, 7 as Word, 0 as Word)\n    when result is\n        Block(block) then block.finish\n        Error(error) then finish_error(error)\n    store.finish\n    return\n"
    );
    let flaws = package_flaws("direct_results", &[("values", &source)]);
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn bare_named_types_form_a_direct_union() {
    let source = format!(
        "{TYPES}\nchoose(value own Word) Word or Error: return value\nrun:\n    result: choose(7 as Word)\n    when result is\n        Word(number) then number.finish\n        Error(error) then finish_error(error)\n    return\n"
    );
    let flaws = package_flaws("direct_results", &[("values", &source)]);
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn anonymous_result_labels_remain_callable_owned() {
    let flaws = package_flaws(
        "direct_results",
        &[(
            "values",
            &format!(
                "{TYPES}\nchoose(value Word) True or False:\n    n is in Word: value\n    when n = 0 then\n        return False\n    return True\ninspect(value Word) Word: return when choose(value) is True then 1 else 0\n"
            ),
        )],
    );
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn named_value_and_payload_free_constructor_form_a_direct_union() {
    for body in ["value", "Done"] {
        let flaws = package_flaws(
            "direct_results",
            &[(
                "values",
                &format!(
                    "{TYPES}\nchoose(value Word) Word or Done: {return_prefix}{body}\n",
                    return_prefix = if body.starts_with('\n') {
                        ""
                    } else {
                        "return "
                    }
                ),
            )],
        );
        assert!(flaws.is_empty(), "{flaws:?}");
    }
}

#[test]
fn direct_union_record_members_need_no_terminal_exit() {
    for result in ["Resource or Error", "Error or Resource"] {
        let flaws = package_flaws(
            "direct_results",
            &[(
                "values",
                &format!(
                    "Resource has value @Resource\nError is Empty or Full\nchoose(value own Resource) {result}: return value\n"
                ),
            )],
        );
        assert!(flaws.is_empty(), "{result}: {flaws:?}");
    }
}

#[test]
fn direct_union_sum_members_need_no_terminal_exit() {
    for result in ["Resource or Error", "Error or Resource"] {
        let flaws = package_flaws(
            "direct_results",
            &[(
                "values",
                &format!(
                    "Resource is Value(@Resource) or Done\nError is Empty or Full\nchoose(value own Resource) {result}: return value\n"
                ),
            )],
        );
        assert!(flaws.is_empty(), "{result}: {flaws:?}");
    }
}

#[test]
fn direct_union_rejects_values_outside_its_members() {
    let flaws = package_flaws(
        "direct_results",
        &[(
            "values",
            &format!("{TYPES}\nchoose(value Word) Block(Word) or Error: return value\n"),
        )],
    );
    assert!(
        flaws
            .iter()
            .any(|code| code == "type-incompatible-union-value"),
        "{flaws:?}"
    );
}

#[test]
fn direct_union_handles_more_than_two_named_members() {
    let flaws = package_flaws(
        "direct_results",
        &[(
            "values",
            &format!(
                "{TYPES}\nOther is Missing or Invalid\nchoose(value Word) Word or Error or Other: return value\n"
            ),
        )],
    );
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn local_callable_preserves_a_direct_union_return_type() {
    let flaws = package_flaws(
        "direct_results",
        &[(
            "values",
            &format!(
                "{TYPES}\nouter(value Word) Word:\n    choose(number Word) Word or Error: return number\n    result: choose(value)\n    return when result is\n        Word(number) then number\n        Error(error) then 0\n"
            ),
        )],
    );
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn direct_union_result_cannot_be_silently_discarded() {
    let values = format!("{TYPES}\nchoose(x, value x) Block(x) or Error: return Block(value)\n");
    let flaws = package_flaws(
        "direct_results",
        &[
            ("values", &values),
            (
                "client",
                "run:\n    result: choose(Word, 7 as Word)\n    return\n",
            ),
        ],
    );
    assert!(
        flaws
            .iter()
            .any(|code| code == "type-terminal-value-not-consumed"),
        "{flaws:?}"
    );
}
