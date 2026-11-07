use std::cell::Cell;

use parser::{ParseFailure, ParseLimit, ParseLimits, ParseOptions, parse_with_options};

fn options(limits: ParseLimits) -> ParseOptions<'static> {
    ParseOptions {
        limits,
        is_cancelled: None,
    }
}

fn refused(source: &str, limits: ParseLimits, expected: ParseLimit) {
    let failure = parse_with_options(source, options(limits)).unwrap_err();
    let ParseFailure::LimitExceeded { limit, span, .. } = failure else {
        panic!("expected a resource refusal, got {failure:?}");
    };
    assert_eq!(limit, expected);
    assert!(span.end() <= source.len());
}

#[test]
fn source_and_token_budgets_are_configurable_at_the_exact_boundary() {
    let limits = ParseLimits {
        source_bytes_max: 2,
        tokens_max: 2,
        ..Default::default()
    };
    assert!(parse_with_options("x\n", options(limits)).is_ok());
    refused(
        "x\n",
        ParseLimits {
            source_bytes_max: 1,
            ..limits
        },
        ParseLimit::SourceBytes,
    );
    refused(
        "x\n",
        ParseLimits {
            tokens_max: 1,
            ..limits
        },
        ParseLimit::Tokens,
    );
    let zero = ParseLimits {
        source_bytes_max: 0,
        tokens_max: 0,
        ..Default::default()
    };
    assert!(parse_with_options("", options(zero)).is_ok());
}

#[test]
fn parenthesis_and_tree_depth_budgets_are_distinct() {
    let limits = ParseLimits {
        parentheses_max: 1,
        expression_depth_max: 2,
        ..Default::default()
    };
    assert!(parse_with_options("(x)", options(limits)).is_ok());
    refused("((x))", limits, ParseLimit::Parentheses);
    assert!(parse_with_options("x + y", options(limits)).is_ok());
    refused("x + y + z", limits, ParseLimit::ExpressionDepth);
}

#[test]
fn body_budget_no_longer_inherits_the_old_lexer_sixteen_level_limit() {
    let mut source = String::new();
    for depth in 0..24 {
        source.push_str(&format!("{}f{depth}:\n", "    ".repeat(depth)));
    }
    source.push_str(&format!("{}return 1\n", "    ".repeat(24)));
    let limits = ParseLimits {
        body_depth_max: 32,
        ..Default::default()
    };
    let output = parse_with_options(&source, options(limits)).unwrap();
    assert!(!output.has_errors());
    refused(
        &source,
        ParseLimits {
            body_depth_max: 4,
            ..limits
        },
        ParseLimit::BodyDepth,
    );
}

#[test]
fn diagnostic_budget_bounds_recovery_without_pretending_it_completed() {
    let source = "first := [\nsecond := [\nthird := [\n";
    let output = parser::parse(source).unwrap();
    assert_eq!(output.diagnostics.len(), 3);
    let limits = ParseLimits {
        diagnostics_max: 2,
        ..Default::default()
    };
    refused(source, limits, ParseLimit::Diagnostics);
    refused(&"\r".repeat(10_000), limits, ParseLimit::Diagnostics);
}

#[test]
fn cancellation_is_distinct_from_source_errors() {
    let cancelled = || true;
    let options = ParseOptions {
        is_cancelled: Some(&cancelled),
        ..Default::default()
    };
    assert!(matches!(
        parse_with_options("valid := 1\n", options),
        Err(ParseFailure::Cancelled { .. })
    ));
    let checks = Cell::new(0);
    let cancel_later = || {
        checks.set(checks.get() + 1);
        checks.get() > 8
    };
    let options = ParseOptions {
        is_cancelled: Some(&cancel_later),
        ..Default::default()
    };
    assert!(matches!(
        parse_with_options(&"x\n".repeat(100), options),
        Err(ParseFailure::Cancelled { .. })
    ));
    assert!(checks.get() > 8);
    assert!(parser::parse("broken := [\n").unwrap().has_errors());
}

#[test]
fn invalid_limits_cannot_disable_recursive_ownership_guards() {
    for limits in [
        ParseLimits {
            diagnostics_max: 0,
            ..Default::default()
        },
        ParseLimits {
            expression_depth_max: 0,
            ..Default::default()
        },
        ParseLimits {
            expression_depth_max: 513,
            body_depth_max: 0,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            parse_with_options("x", options(limits)),
            Err(ParseFailure::InvalidLimits(_))
        ));
    }
}

#[test]
fn defaults_accept_generated_sources_beyond_the_original_prototype_budgets() {
    let large_comment = format!("--{}\n", "x".repeat(1024 * 1024));
    assert!(!parser::parse(&large_comment).unwrap().has_errors());
    let many_tokens = "x\n".repeat(50_001);
    let output = parser::parse(&many_tokens).unwrap();
    assert!(!output.has_errors());
    assert_eq!(output.file.items.len(), 50_001);
}

#[test]
fn configured_ownership_boundary_supports_clone_debug_and_drop() {
    let limits = ParseLimits {
        expression_depth_max: 512,
        body_depth_max: 0,
        ..Default::default()
    };
    let source = "x + ".repeat(511) + "x";
    let output = parse_with_options(&source, options(limits)).unwrap();
    assert!(!output.has_errors());
    let snapshot = output.clone();
    assert_eq!(snapshot, output);
    assert!(!format!("{snapshot:?}").is_empty());
    drop(snapshot);
    drop(output);
}

#[test]
fn cancellation_is_checked_during_parsing_not_only_lexing() {
    let source = "main:\n    broken := [\n    return 2\nnext := 3\n";
    let baseline_checks = Cell::new(0);
    let count_only = || {
        baseline_checks.set(baseline_checks.get() + 1);
        false
    };
    let baseline = ParseOptions {
        is_cancelled: Some(&count_only),
        ..Default::default()
    };
    assert!(parse_with_options(source, baseline).unwrap().has_errors());
    let checks = Cell::new(0);
    let cancel_near_completion = || {
        checks.set(checks.get() + 1);
        checks.get() >= baseline_checks.get() - 1
    };
    let options = ParseOptions {
        is_cancelled: Some(&cancel_near_completion),
        ..Default::default()
    };
    assert!(matches!(
        parse_with_options(source, options),
        Err(ParseFailure::Cancelled { .. })
    ));
}

#[test]
fn a_format_string_with_many_holes_is_one_token_and_costs_one() {
    // The text of a format string is read from the source, so 50,000 holes are one token, not
    // 200,000: nothing is expanded, and the hole is reported once.
    let source = format!("\"{}\"", "\\(x)".repeat(50_000));
    let limits = ParseLimits {
        tokens_max: 1,
        ..Default::default()
    };
    let output = parse_with_options(&source, options(limits)).expect("within the token budget");
    assert!(output.has_errors());
    // The budget still counts the tokens that are there: two strings are two tokens.
    refused("\"a\" \"b\"", limits, ParseLimit::Tokens);
}
