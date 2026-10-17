use parser::query::SourceParseExt;

mod common;

use common::assert_roundtrip;

#[test]
fn domain_queries_and_destructuring_parameters_round_trip() {
    let source = "Range(lb to ub) is lb to ub\n\nRange(lb to ub) has\n    min: lb\n    max: ub\n\nWord is in Range(-128 to 127)\n\nWord has\n    minimum(): return Range(domain of Self).min\n";
    let formatted = ginfmt::format(source);
    assert!(formatted.contains("Range(lb to ub)"), "{formatted}");
    assert!(
        formatted.contains("Word is in Range(-128 to 127)"),
        "{formatted}"
    );
    assert!(formatted.contains("min: lb"), "{formatted}");
    assert!(formatted.contains("max: ub"), "{formatted}");
    assert!(
        formatted.contains("Range(domain of Self).min"),
        "{formatted}"
    );
    let parsed = formatted.parse_source_full();
    assert!(
        parsed.symptoms.is_empty(),
        "{:?}\n{formatted}",
        parsed.symptoms
    );
    assert_roundtrip(&formatted);
}
