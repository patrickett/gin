//! Bare nullary source forms preserve their written shape and evaluate coherently.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

fn check(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid bare nullary syntax");
    let before = source.clone();
    let result = check_module("example.nullary", &source, &[], Limits::default());
    assert_eq!(source, before);
    result
}

#[test]
fn no_list_definitions_bare_calls_and_bare_empty_patterns_work_together() {
    let source = "Token has
make_token Token: return Token
finish_token(token Token):
    Token := token
    return
run:
    finish_token(make_token)
    return
";
    let checked = check(source).unwrap();
    assert_eq!(checked.checked().functions.len(), 3);
}

#[test]
fn a_forward_bare_call_is_scheduled_before_its_caller() {
    let source = "Token has
forward Token: return later
later Token: return Token
";
    let checked = check(source).unwrap();
    assert_eq!(checked.checked().functions[0].name, "forward");
}

#[test]
fn bare_local_names_do_not_create_false_nullary_dependency_cycles() {
    let source = "Token has
builder Token: return shadow(builder_value)
builder_value Token: return Token
shadow(builder Token) Token: return builder
";
    check(source).unwrap();
    check("Token has\nidentity(identity Token) Token: return identity\n").unwrap();
}

#[test]
fn a_case_binder_can_shadow_a_bare_nullary_computation() {
    let source = "Token has
Wrapper has value Token
empty_token Token: return Token
wrapper Wrapper: return helper(Wrapper(value: empty_token))
helper(value Wrapper) Wrapper:
    return when value is
        Wrapper(wrapper) then Wrapper(value: wrapper)
";
    check(source).unwrap();
}

#[test]
fn genuine_bare_call_cycles_still_reject() {
    let source = "Token has
first Token: return second
second Token: return first
";
    assert_eq!(check(source).unwrap_err(), SourceCheckError::Unsupported);
}

#[test]
fn a_bare_nonempty_constructor_cannot_fabricate_its_fields() {
    let source = "Nonempty has value in 0 to 16
bad Nonempty: return Nonempty
";
    assert_eq!(check(source).unwrap_err(), SourceCheckError::Type);
}

#[test]
fn each_bare_nullary_factory_evaluation_returns_a_distinct_owner() {
    let source = "Token has
Pair has first Token, second Token
make_token Token: return Token
make_pair Pair: return Pair(first: make_token, second: make_token)
";
    check(source).unwrap();
    let duplicate = source.replace(
        "make_pair Pair: return Pair(first: make_token, second: make_token)",
        "make_pair Pair:\n    token: make_token\n    return Pair(first: token, second: token)",
    );
    assert_ne!(duplicate, source);
    assert_eq!(
        check(&duplicate).unwrap_err(),
        SourceCheckError::UseAfterMove
    );
}

#[test]
fn pure_contracts_do_not_start_executing_bare_computations() {
    let source = "one in 0 to 16: return 1
claim(value in 0 to 0) in 0 to 16 and <= one: return value
";
    let error = check(source).unwrap_err();
    assert!(matches!(
        error,
        SourceCheckError::UnknownName | SourceCheckError::Unsupported
    ));
}
