mod common;
use common::*;

#[test]
fn conditional_return_uses_the_value_on_the_return_line() {
    for (condition, expected) in [(0, 29), (1, 17)] {
        let source = format!(
            "{}choose(flag Word) Word:\n    when flag is 1 then\n        return 17\n    return 29\n#entry\nmain: choose({condition})\n",
            test_fixtures::preludes::WORD_PRELUDE
        );
        cra(&format!("conditional_return_{condition}"), &source)
            .assert_compiled()
            .assert_exit_code(expected);
    }
}

#[test]
fn conditional_return_preserves_statements_and_nested_returns() {
    for (outer, inner, expected) in [(0, 0, 29), (1, 0, 17), (1, 1, 11)] {
        let source = format!(
            "{}choose(outer Word, inner Word) Word:\n    when outer is 1 then\n        value Word: 17\n        when inner is 1 then\n            return 11\n        return value\n    return 29\n#entry\nmain: choose({outer}, {inner})\n",
            test_fixtures::preludes::WORD_PRELUDE
        );
        cra(&format!("nested_return_{outer}_{inner}"), &source)
            .assert_compiled()
            .assert_exit_code(expected);
    }
}

#[test]
fn bare_conditional_return_skips_the_function_continuation() {
    for (condition, expected) in [(0, 29), (1, 17)] {
        let source = format!(
            "{}replace(previous Word, next Word) Word: return next\nset(value mut Word, flag Word):\n    value:: replace(deref value, 17)\n    when flag is 1 then\n        return\n    value:: replace(deref value, 29)\n#entry\nmain:\n    value Word: 0\n    set(value, {condition})\n    return value\n",
            test_fixtures::preludes::WORD_PRELUDE
        );
        cra(&format!("bare_return_{condition}"), &source)
            .assert_compiled()
            .assert_exit_code(expected);
    }
}

#[test]
fn local_unit_functions_fall_through_and_return_early() {
    for (condition, expected) in [(0, 29), (1, 17)] {
        let source = format!(
            "Word is in 0 to 255\nWord has finish(self): return\nreplace(previous Word, next Word) Word: return next\nset(value mut Word, flag Word):\n    local(condition Word):\n        value:: replace(deref value, 17)\n        when condition is 1 then return\n        value:: replace(deref value, 29)\n    local(flag)\n#entry\nmain:\n    value Word: 0\n    set(value, {condition})\n    return value\n"
        );
        cra(&format!("local_bare_return_{condition}"), &source)
            .assert_compiled()
            .assert_exit_code(expected);
    }
}

#[test]
fn unit_functions_can_end_with_a_conditional_effect() {
    for (condition, expected) in [(0, 0), (1, 17)] {
        let source = format!(
            "Word is in 0 to 255\nWord has finish(self): return\nreplace(previous Word, next Word) Word: return next\nset(value mut Word, flag Word):\n    when flag is 1 then\n        value:: replace(deref value, 17)\n#entry\nmain:\n    value Word: 0\n    set(value, {condition})\n    return value\n"
        );
        cra(&format!("conditional_effect_{condition}"), &source)
            .assert_compiled()
            .assert_exit_code(expected);
    }
}

#[test]
fn conditional_return_after_reassignment_returns_the_new_value() {
    for (condition, expected) in [(0, 29), (1, 17)] {
        let source = format!(
            "{}replace(previous Word, next Word) Word: return next\nchoose(flag Word) Word:\n    value Word: 29\n    when flag is 1 then\n        value:: replace(value, 17)\n        return value\n    return value\n#entry\nmain: choose({condition})\n",
            test_fixtures::preludes::WORD_PRELUDE
        );
        cra(&format!("reassigned_return_{condition}"), &source)
            .assert_compiled()
            .assert_exit_code(expected);
    }
}

#[test]
fn return_from_a_loop_exits_the_function() {
    for (name, source, expected) in [
        (
            "while_return",
            format!(
                "{}choose(flag Word) Word:\n    while flag is 1 do\n        when flag is 1 then\n            return 17\n    return 29\n#entry\nmain: choose(1)\n",
                test_fixtures::preludes::WORD_PRELUDE
            ),
            17,
        ),
        (
            "while_fallthrough",
            format!(
                "{}choose(flag Word) Word:\n    while flag is 1 do\n        when flag is 1 then\n            return 17\n    return 29\n#entry\nmain: choose(0)\n",
                test_fixtures::preludes::WORD_PRELUDE
            ),
            29,
        ),
        (
            "for_return",
            format!(
                "{}choose(limit Word) Word:\n    for index in 0 to 3\n        when index is 1 then\n            return 17\n    return 29\n#entry\nmain: choose(3)\n",
                test_fixtures::preludes::WORD_PRELUDE
            ),
            17,
        ),
    ] {
        cra(name, &source)
            .assert_compiled()
            .assert_exit_code(expected);
    }
}

#[test]
fn bare_return_from_a_for_loop_skips_later_iterations() {
    let source = format!(
        "{}replace(previous Word, next Word) Word: return next\nvisit(value mut Word):\n    for index in 0 to 3\n        when index is 1 then\n            value:: replace(deref value, 17)\n            return\n        value:: replace(deref value, 29)\n    value:: replace(deref value, 29)\n    return\n#entry\nmain:\n    value Word: 0\n    visit(value)\n    return value\n",
        test_fixtures::preludes::WORD_PRELUDE
    );
    cra("bare_for_return", &source)
        .assert_compiled()
        .assert_exit_code(17);
}

#[test]
fn conditional_returns_preserve_all_named_results() {
    for (flag, expected) in [(0, 52), (1, 28)] {
        let source = format!(
            "{}choose(flag Word) first Word, second Word:\n    while flag is 1 do\n        when flag is 1 then\n            return 17, 11\n    return 29, 23\n#entry\nmain:\n    pair: choose({flag})\n    left is in 0 to 255: pair.first\n    right is in 0 to 255: pair.second\n    return left + right\n",
            test_fixtures::preludes::WORD_PRELUDE
        );
        cra(&format!("named_loop_return_{flag}"), &source)
            .assert_compiled()
            .assert_exit_code(expected);
    }
}
