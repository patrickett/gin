#![allow(dead_code)]
use diagnostic::{Category, Diagnostic};
use ginfmt::{Config, format_with_config};
use parser::query::SourceParseExt;

pub fn assert_roundtrip(source: &str) {
    assert_eq!(ginfmt::format(source), source);
}

pub fn assert_formatted(source: &str, expected: &str, config: Config) {
    assert_formatted_with(source, expected, config, |symptoms| symptoms.is_empty());
}

pub fn assert_formatted_at_width(source: &str, expected: &str, max_line_width: usize) {
    assert_formatted(
        source,
        expected,
        Config {
            max_line_width,
            ..Config::default()
        },
    );
}

pub fn assert_formatted_allowing_warnings(source: &str, expected: &str, config: Config) {
    assert_formatted_with(source, expected, config, |symptoms| {
        symptoms
            .iter()
            .all(|diagnostic| diagnostic.category != Category::Flaw)
    });
}

fn assert_formatted_with(
    source: &str,
    expected: &str,
    config: Config,
    clean: impl Fn(&[Diagnostic]) -> bool,
) {
    let parsed = source.parse_source_full();
    assert!(clean(&parsed.symptoms), "{source}\n{:#?}", parsed.symptoms);
    let formatted = format_with_config(source, config.clone());
    assert_eq!(formatted, expected);
    let reparsed = formatted.parse_source_full();
    assert!(
        clean(&reparsed.symptoms),
        "{formatted}\n{:#?}",
        reparsed.symptoms
    );
    assert_eq!(format_with_config(&formatted, config), formatted);
}
