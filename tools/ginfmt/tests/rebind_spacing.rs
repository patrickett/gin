use ginfmt::Config;

mod common;

use common::{assert_formatted_allowing_warnings, assert_roundtrip};

#[test]
fn spaces_rebinds_on_locals_and_projections() {
    for lhs in [
        "value",
        "self.position",
        "pair.0",
        "buffer.[0]",
        "buffer.[index + 1]",
    ] {
        for spacing in ["::1", ":: 1", " ::1", "  \t::\t 1", " :: 1"] {
            assert_formatted_allowing_warnings(
                &format!("{lhs}{spacing}\n"),
                &format!("{lhs} :: 1\n"),
                Config::default(),
            );
        }
    }
}

#[test]
fn spaces_rebinds_in_preserved_method_and_control_flow_bodies() {
    assert_formatted_allowing_warnings(
        "Reader has\n    refill(self mut):\n        self.position:: 0\n        while ready is True do\n            buffer.[0]::1\n        return\n",
        "Reader has\n    refill(self mut):\n        self.position :: 0\n        while ready is True do\n            buffer.[0] :: 1\n        return\n",
        Config::default(),
    );
    assert_formatted_allowing_warnings(
        "run:\n    value: 0\n    value::1\n    return value\n",
        "run:\n    value: 0\n    value :: 1\n    return value\n",
        Config::default(),
    );
}

#[test]
fn literals_comments_and_other_bind_operators_keep_their_text() {
    for source in [
        "-- value::1\ntext: 'value::1'\n",
        "run:\n    immutable := 1\n    value :: '::'\n    buffer.[0] +: 2\n    return\n",
    ] {
        assert_formatted_allowing_warnings(source, source, Config::default());
    }
}

#[test]
fn malformed_source_is_unchanged() {
    let source = "value::\n";
    assert_roundtrip(source);
}

#[test]
fn multiline_rebinds_keep_their_line_break_and_have_no_trailing_space() {
    assert_formatted_allowing_warnings(
        "value::\n    compute()\n    return 1\n",
        "value ::\n    compute()\n    return 1\n",
        Config::default(),
    );
}

#[test]
fn rebinds_with_non_ascii_literal_values_preserve_utf8() {
    assert_formatted_allowing_warnings(
        "value::'café::茶'\n",
        "value :: 'café::茶'\n",
        Config::default(),
    );
}

#[test]
fn typed_reference_parameters_do_not_prevent_body_formatting() {
    assert_formatted_allowing_warnings(
        "update(left ref Vector(Byte), right mut Vector(Byte)):\n    right.len:: left.len\n    return\n",
        "update(left ref Vector(Byte), right mut Vector(Byte)):\n    right.len :: left.len\n    return\n",
        Config::default(),
    );
}

#[test]
fn numeric_domain_parameters_do_not_discard_body_formatting() {
    for parameters in [
        "value mut in 0 to 255, byte is in 0 to 255",
        "value mut in Byte, byte is in Byte",
    ] {
        assert_formatted_allowing_warnings(
            &format!("update({parameters}):\n    value:: byte\n    return\n"),
            &format!("update({parameters}):\n    value :: byte\n    return\n"),
            Config::default(),
        );
    }
}

#[test]
fn grouped_expressions_keep_all_parentheses_while_spacing_rebinds() {
    for source in [
        "calculate(value Int) Int:\n    value:: ((value + 1) * 2) as Int\n    return ((value + 1) * 2) as Int\n",
        "calculate(value Int) Int: return ((value + 1) * 2) as Int\n",
    ] {
        assert_formatted_allowing_warnings(
            source,
            &source.replace("value:: ", "value :: "),
            Config::default(),
        );
    }
    assert_formatted_allowing_warnings(
        "choose(value Int) Int: return (when value = 0 then 1 else 2) as Int\n",
        "choose(value Int) Int: return (when value = 0 then 1 else 2) as Int\n",
        Config::default(),
    );
}

#[test]
fn condition_after_a_loop_does_not_accumulate_blank_lines() {
    assert_formatted_allowing_warnings(
        "run:\n    while ready is True do\n        value::1\n    when done is True then\n        finish()\n        return\n    return\n",
        "run:\n    while ready is True do\n        value :: 1\n    when done is True then\n        finish()\n        return\n    return\n",
        Config::default(),
    );
}
