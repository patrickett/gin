use parser::{ParseFailure, ParseLimit, ParseLimits, ParseOptions};

#[test]
fn borrowed_prefix_depth_and_group_diagnostics_obey_existing_budgets() {
    for (source, limits, expected) in [
        (
            "T is ref mut ref Item",
            ParseLimits {
                expression_depth_max: 3,
                ..Default::default()
            },
            ParseLimit::ExpressionDepth,
        ),
        (
            "T is ref{}",
            ParseLimits {
                diagnostics_max: 1,
                ..Default::default()
            },
            ParseLimit::Diagnostics,
        ),
        (
            "T is ref{g.item} Item",
            ParseLimits {
                tokens_max: 3,
                ..Default::default()
            },
            ParseLimit::Tokens,
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
    let options = ParseOptions {
        limits: ParseLimits {
            parentheses_max: 0,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(
        !parser::parse_with_options("T is ref{g.items} Item", options)
            .unwrap()
            .has_errors()
    );
}

#[test]
fn cancellation_is_checked_late_while_consuming_long_group_paths() {
    use std::cell::Cell;

    let source = format!("T is ref{{root{}}} Item\n", ".item".repeat(2_000));
    let total = Cell::new(0usize);
    let count = || {
        total.set(total.get() + 1);
        false
    };
    assert!(
        !parser::parse_with_options(
            &source,
            ParseOptions {
                is_cancelled: Some(&count),
                ..Default::default()
            }
        )
        .unwrap()
        .has_errors()
    );
    let cutoff = total.get() - 1_000;
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
