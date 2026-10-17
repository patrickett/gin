use diagnostic::code::{SymbolCode, TypeCode};
use std::path::Path;

use diagnostic::{Category, Diagnostic, DiagnosticCode, DiagnosticPathExt, Span};

#[test]
fn diagnostic_report_path_is_relative_to_normalized_base() {
    let cwd = std::env::current_dir().unwrap();
    let path = cwd.join("src").join("..").join("src").join("lib.rs");

    assert_eq!(path.diagnostic_report_path(&cwd), "src/lib.rs");
}

#[test]
fn normalize_diagnostic_path_makes_relative_paths_absolute() {
    let normalized = Path::new("src/../src/lib.rs").normalize_diagnostic_path();

    assert!(normalized.is_absolute());
    assert!(normalized.ends_with(Path::new("src/lib.rs")));
}

#[test]
fn builder_basics() {
    let diag = Diagnostic::new(SymbolCode::Unknown, "test message")
        .with_arg("key", "val")
        .with_help("helpful text")
        .at_span(Span::new(5, 10));

    assert_eq!(
        diag.code,
        DiagnosticCode::Type(TypeCode::Symbol(SymbolCode::Unknown))
    );
    assert_eq!(diag.error_code(), "type-unknown-symbol");
    assert_eq!(diag.code.to_string(), "type-unknown-symbol");
    assert_eq!(diag.message, "test message");
    assert_eq!(diag.arg("key"), Some("val"));
    assert_eq!(diag.help.as_deref(), Some("helpful text"));
    assert_eq!(diag.span, Span::new(5, 10));
}

#[test]
fn with_category_sets_category() {
    let diag = Diagnostic::new(TypeCode::Mismatch, "y").with_category(Category::Warn);
    assert_eq!(diag.category, Category::Warn);
}

#[test]
fn diagnostic_category_names_use_gin_terms() {
    assert_eq!(Category::Flaw.as_str(), "flaw");
    assert_eq!(Category::Warn.as_str(), "warn");
    assert_eq!(Category::Help.as_str(), "help");
}

#[test]
fn diagnostic_codes_have_unique_kebab_case_slugs() {
    let mut slugs = std::collections::HashSet::new();
    for code in DiagnosticCode::all() {
        let slug = code.slug();
        assert!(
            slugs.insert(slug.clone()),
            "duplicate diagnostic code: {slug}"
        );
        assert!(
            slug.split('-').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            }),
            "invalid diagnostic code: {slug}"
        );
        assert_eq!(code.to_string(), slug);
    }
}

#[test]
fn nested_codes_keep_their_public_strings() {
    use diagnostic::code::{ExpectedSyntaxCode, ImportCode, IntegerCode};

    for (code, slug) in [
        (
            DiagnosticCode::from(IntegerCode::LiteralOutOfRange),
            "type-integer-literal-out-of-range",
        ),
        (
            DiagnosticCode::from(ExpectedSyntaxCode::Token),
            "parse-expected-token",
        ),
        (
            DiagnosticCode::from(ImportCode::ModuleNotFound),
            "use-module-not-found",
        ),
    ] {
        assert_eq!(code.slug(), slug);
        assert_eq!(code.to_string(), slug);
    }
    assert_eq!(
        IntegerCode::LiteralOutOfRange.to_string(),
        "type-integer-literal-out-of-range"
    );
}

#[test]
fn prefixes_and_names_compose_through_parent_enums() {
    use diagnostic::code::{Code, MemberCode, Prefix};

    let member = MemberCode::ArgumentsRequired;
    let parent = TypeCode::from(member);
    let flaw = DiagnosticCode::from(parent);
    assert_eq!(member.prefix(), "type");
    assert_eq!(member.name(), "member-requires-arguments");
    assert_eq!(flaw.prefix(), member.prefix());
    assert_eq!(flaw.name(), member.name());
    assert_eq!(flaw.to_string(), "type-member-requires-arguments");
}

#[test]
fn legacy_codes_without_prefixes_have_no_leading_separator() {
    use diagnostic::code::{CodegenCode, Prefix};

    let code = DiagnosticCode::from(CodegenCode::SumPayloadArityMismatch);
    assert_eq!(code.prefix(), "");
    assert_eq!(code.to_string(), "sum-payload-arity-mismatch");
    assert_eq!(DiagnosticCode::UnfilledHole.to_string(), "unfilled-hole");
}
