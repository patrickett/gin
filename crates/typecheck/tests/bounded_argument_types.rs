mod support;

use support::expect_flaw_code_named;

const CASES: &[(&str, &str, &str)] = &[
    (
        "local_integer_parameters_reject_boolean_values",
        "Bool is True or False\nouter(flag Bool):\n    local(value is in 0 to 1): return value\n    return local(flag)\n",
        "type-incompatible-union-value",
    ),
    (
        "declared_integer_parameters_reject_boolean_values",
        "Bool is True or False\nbounded(value is in 0 to 1): return value\nouter(flag Bool): return bounded(flag)\n",
        "type-incompatible-union-value",
    ),
    (
        "integer_parameters_reject_float_values",
        "outer():\n    local(value is in 0 to 1): return value\n    return local(0.5)\n",
        "type-mismatch",
    ),
    (
        "integer_parameters_reject_float_values",
        "bounded(value is in 0 to 1): return value\nouter(): return bounded(0.5)\n",
        "type-mismatch",
    ),
];

#[test]
fn bounded_argument_type_cases() {
    for &(name, source, code) in CASES {
        expect_flaw_code_named(source, code, name);
    }
}
