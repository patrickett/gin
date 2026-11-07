use typecheck::{Limits, SourceCheckError, SourceType, check_file};

#[test]
fn primary_entry_consumes_new_ast_and_rejects_a_body_only_contract_violation() {
    let positive =
        parser::parse_strict(include_str!("../examples/source-echo.gin")).unwrap();
    let checked = check_file(&positive, Limits::default()).unwrap();
    assert_eq!(checked.functions.len(), 1);
    assert_eq!(checked.functions[0].name, "echo");
    assert_eq!(
        checked.functions[0].result,
        SourceType::Integer {
            lower: 0.into(),
            upper: 16.into(),
        }
    );

    let negative =
        parser::parse_strict(include_str!("../examples/source-echo-invalid.gin")).unwrap();
    assert_eq!(
        check_file(&negative, Limits::default()),
        Err(SourceCheckError::Unproved)
    );
}

#[test]
fn complete_window_source_is_checked_through_the_primary_entry() {
    let source =
        parser::parse_strict(include_str!("../examples/window-source.gin")).unwrap();
    assert_eq!(source.items.len(), 4);
    let checked = check_file(&source, Limits::default()).unwrap();
    assert_eq!(checked.functions.len(), 3);
}

#[test]
fn invalid_window_body_fails_the_constructor_relation() {
    let source =
        parser::parse_strict(include_str!("../examples/window-invalid.gin")).unwrap();
    assert_eq!(
        check_file(&source, Limits::default()),
        Err(SourceCheckError::Unproved)
    );
}
