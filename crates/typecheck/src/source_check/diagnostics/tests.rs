use super::*;

fn check(text: &str) -> (SourceFile, SourceCheckFailure) {
    let source = parser::parse_strict(text).expect("strict source syntax");
    let before = source.clone();
    let failure = check_file_diagnostic(&source, crate::Limits::default()).unwrap_err();
    assert_eq!(
        check_file(&source, crate::Limits::default()),
        Err(failure.error),
    );
    assert_eq!(source, before);
    (source, failure)
}

fn function_span(source: &SourceFile, name: &str) -> Span {
    let item = source
        .items
        .iter()
        .find(|item| matches!(&item.value, Item::Binding(binding) if binding.name.value == name))
        .unwrap();
    source.span_table.get(item.span_id)
}

#[test]
fn function_context_follows_the_failure_under_declaration_permutations() {
    let good = "good() in 0 to 9: return 1\n";
    for (bad, error) in [
        (
            "bad() in 0 to 9: return absent\n",
            SourceCheckError::UnknownName,
        ),
        ("bad() Int: return 0\n", SourceCheckError::Unsupported),
        (
            "bad(a in 0 to 9) in 0 to 9 and < a: return a\n",
            SourceCheckError::Unproved,
        ),
    ] {
        for text in [format!("{good}{bad}"), format!("{bad}{good}")] {
            let (source, failure) = check(&format!("-- π 🦀 byte offsets\n{text}"));
            assert_eq!(failure.error, error);
            assert_eq!(failure.span, Some(function_span(&source, "bad")));
            assert_ne!(failure.span, Some(function_span(&source, "good")));
            assert!(source.text.get(failure.span.unwrap().to_range()).is_some());
        }
    }
}

#[test]
fn scheduling_does_not_attribute_failure_to_the_last_scanned_or_verified_function() {
    for text in [
        "bad() in 0 to 9: return good()\ngood() in 0 to 9: return absent\n",
        "good() in 0 to 9: return 1\nbad() in 0 to 9 and > 1: return good()\n",
    ] {
        let (source, failure) = check(text);
        let name = if failure.error == SourceCheckError::UnknownName {
            "good"
        } else {
            "bad"
        };
        assert_eq!(failure.span, Some(function_span(&source, name)));
    }
}

#[test]
fn actual_echo_and_window_fixtures_have_semantic_context() {
    for valid in [
        include_str!("../../../examples/source-echo.gin"),
        include_str!("../../../examples/window-source.gin"),
    ] {
        let source = parser::parse_strict(valid).unwrap();
        assert_eq!(
            check_file_diagnostic(&source, crate::Limits::default()).unwrap(),
            check_file(&source, crate::Limits::default()).unwrap(),
        );
    }
    let invalid_window = include_str!("../../../examples/window-source.gin").replacen(
        "return Window(payload: payload, lower: lower, upper: upper)",
        "return Window(payload: payload, lower: lower, upper: 0)",
        1,
    );
    for invalid in [
        include_str!("../../../examples/source-echo-invalid.gin"),
        &invalid_window,
    ] {
        let (source, failure) = check(invalid);
        assert_eq!(failure.error, SourceCheckError::Unproved);
        assert!(source.items.iter().any(|item| {
            matches!(item.value, Item::Binding(_))
                && failure.span == Some(source.span_table.get(item.span_id))
        }));
    }
}

#[test]
fn schema_context_is_the_failing_body_not_an_earlier_valid_field_or_schema() {
    for bad in [
        "Bad has value in 0 to 9 and >= absent\n",
        "Bad has value Unknown\n",
        "Bad has lower in 8 to 10, upper in 0 to 3 and >= lower\n",
    ] {
        for text in [
            format!("Good has value in 0 to 9\n{bad}"),
            format!("{bad}Good has value in 0 to 9\n"),
        ] {
            let (source, failure) = check(&text);
            let declaration = source
                .items
                .iter()
                .find_map(|item| match &item.value {
                    Item::Type(declaration) if declaration.name.value == "Bad" => Some(declaration),
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                failure.span,
                Some(source.span_table.get(declaration.body.span_id))
            );
        }
    }
}

#[test]
fn annotation_phase_precedes_constraint_phase_in_diagnostic_mode_too() {
    let (source, failure) = check(concat!(
        "First has value in 0 to 9 and >= absent\n",
        "Second has value Unknown\n",
    ));
    assert_eq!(failure.error, SourceCheckError::Unsupported);
    let Item::Type(second) = &source.items[1].value else {
        panic!("schema")
    };
    assert_eq!(
        failure.span,
        Some(source.span_table.get(second.body.span_id))
    );
}

#[test]
fn import_failure_uses_the_import_not_a_preceding_declaration() {
    for (import, error) in [
        (
            "use absent.module selected\n",
            SourceCheckError::UnknownName,
        ),
        ("use absent.module\n", SourceCheckError::Unsupported),
    ] {
        let (source, failure) = check(&format!("good(): return\n{import}"));
        assert_eq!(failure.error, error);
        assert_eq!(
            failure.span,
            Some(source.span_table.get(source.items[1].span_id))
        );
    }
}

#[test]
fn invalid_context_ranges_are_never_published() {
    let mut source = parser::parse_strict("-- π\nbad(): return\n").unwrap();
    let invalid = [
        SpanId::INVALID,
        SpanId::new(u32::MAX - 1),
        source.span_table.insert(Span::new(4, 3)),
        source
            .span_table
            .insert(Span::new(0, source.text.len() + 1)),
        source.span_table.insert(Span::new(4, 5)), // Inside π.
        source.span_table.insert(Span::new(0, 0)),
    ];
    for context in invalid {
        let checker = Checker::new(&source, crate::Limits::default());
        assert_eq!(
            checker.failure(SourceCheckError::Malformed, context).span,
            None
        );
        let mut malformed = source.clone();
        malformed.items[0].span_id = context;
        assert_eq!(
            check_file_diagnostic(&malformed, crate::Limits::default()),
            Err(SourceCheckFailure {
                error: SourceCheckError::Malformed,
                span: None,
                file: None,
                detail: None,
                culprit: None,
                facts: Vec::new(),
            }),
        );
    }
}

#[test]
fn caught_errors_cannot_leave_context_for_a_later_failure() {
    let source = parser::parse_strict("first(): return\nsecond(): return\n").unwrap();
    let mut checker = Checker::new(&source, crate::Limits::default());
    let ignored = checker
        .span(SpanId::INVALID)
        .map_err(|error| checker.failure(error, source.items[0].span_id));
    assert!(ignored.is_err());
    let later = checker
        .local("absent", "absent")
        .map_err(|error| checker.failure(error, source.items[1].span_id));
    let failure = later.err().unwrap();
    assert_eq!(failure.error, SourceCheckError::UnknownName);
    assert_eq!(
        failure.span,
        Some(source.span_table.get(source.items[1].span_id))
    );
    assert_eq!(SourceCheckFailure::from(SourceCheckError::Type).span, None);
}

#[test]
fn budgets_and_non_source_module_failures_are_unlocated() {
    let source = parser::parse_strict("good() in 0 to 9: return 1\n").unwrap();
    for limits in [
        crate::Limits {
            work_max: 0,
            ..crate::Limits::default()
        },
        crate::Limits {
            definitions_max: 0,
            ..crate::Limits::default()
        },
        crate::Limits {
            flow_work_max: 0,
            ..crate::Limits::default()
        },
    ] {
        let failure = check_file_diagnostic(&source, limits).unwrap_err();
        assert_eq!(failure, SourceCheckError::Budget.into());
        assert_eq!(check_file(&source, limits), Err(failure.error));
    }
    for name in ["", "not a module"] {
        let failure =
            check_module_diagnostic(name, &source, &[], crate::Limits::default()).unwrap_err();
        assert_eq!(failure, SourceCheckError::Malformed.into());
    }
    let checker = Checker::new(&source, crate::Limits::default());
    assert_eq!(
        checker
            .failure(SourceCheckError::Budget, source.items[0].span_id)
            .span,
        None
    );
}
