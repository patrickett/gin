mod support;

use support::check_lexed;

#[test]
fn outer_return_refinement_does_not_constrain_uncalled_local_exits() {
    for local in ["local(input is in 0 to 511)", "local()"] {
        let source = format!(
            "outer(value is in 0 to 511) is in 0 to 255:\n    {local}:\n        when value is in 0 to 255 then\n            return 300\n        return 400\n    return 0\n"
        );
        let typed = check_lexed(&source);
        assert!(
            typed.all_flaws().is_empty(),
            "{source}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn outer_return_refinement_still_checks_its_own_conditional_exits() {
    for (value, accepted) in [(255, true), (256, false)] {
        let source = format!(
            "outer(value is in 0 to 511) is in 0 to 255:\n    local(): return 400\n    when value is in 0 to 255 then\n        return {value}\n    return 0\n"
        );
        let typed = check_lexed(&source);
        if accepted {
            assert!(
                typed.all_flaws().is_empty(),
                "{source}: {:?}",
                typed.all_flaws()
            );
        } else {
            assert!(
                typed
                    .all_flaws()
                    .iter()
                    .any(|(_, flaw)| { flaw.code.slug() == "type-integer-narrowing-failed" }),
                "{source}: {:?}",
                typed.all_flaws()
            );
        }
    }
}

#[test]
fn outer_return_refinement_checks_nested_conditional_exits() {
    let source = "outer(value is in 0 to 511) is in 0 to 255:\n    when value is in 0 to 255 then\n        when value is in 0 to 127 then\n            return 256\n        return 0\n    return 0\n";
    let typed = check_lexed(source);
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| { flaw.code.slug() == "type-integer-narrowing-failed" }),
        "{:?}",
        typed.all_flaws()
    );
}
