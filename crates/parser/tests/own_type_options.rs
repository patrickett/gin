use parser::{ParseFailure, ParseLimit, ParseLimits, ParseOptions};

#[test]
fn own_prefixes_obey_expression_and_diagnostic_limits() {
    for (source, limits, expected) in [
        (
            "Storage is own own Item",
            ParseLimits {
                expression_depth_max: 2,
                ..Default::default()
            },
            ParseLimit::ExpressionDepth,
        ),
        (
            "Storage is own{}",
            ParseLimits {
                diagnostics_max: 1,
                ..Default::default()
            },
            ParseLimit::Diagnostics,
        ),
    ] {
        let options = ParseOptions {
            limits,
            ..Default::default()
        };
        let ParseFailure::LimitExceeded { limit, span, .. } =
            parser::parse_with_options(source, options).unwrap_err()
        else {
            panic!("bounded refusal");
        };
        assert_eq!(limit, expected);
        assert!(source.get(span.to_range()).is_some());
    }
}

#[test]
fn invalid_own_groups_still_check_cancellation_while_retaining_the_path() {
    use std::cell::Cell;

    let source = format!("Storage is own{{root{}}} Item\n", ".item".repeat(2_000));
    let count = Cell::new(0usize);
    let baseline = || {
        count.set(count.get() + 1);
        false
    };
    let output = parser::parse_with_options(
        &source,
        ParseOptions {
            is_cancelled: Some(&baseline),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(output.has_errors());
    assert!(count.get() > 1_000);
    let cutoff = count.get() - 1_000;
    let visits = Cell::new(0usize);
    let cancel = || {
        visits.set(visits.get() + 1);
        visits.get() == cutoff
    };
    let failure = parser::parse_with_options(
        &source,
        ParseOptions {
            is_cancelled: Some(&cancel),
            ..Default::default()
        },
    )
    .unwrap_err();
    let ParseFailure::Cancelled { span } = failure else {
        panic!("cancellation");
    };
    assert!(span.start() > source.len() / 2);
    assert!(source.get(span.to_range()).is_some());
}
