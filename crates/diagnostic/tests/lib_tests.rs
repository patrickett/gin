use std::path::Path;

use diagnostic::{Category, Diagnostic, DiagnosticPathExt, Span};

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
    let diag = Diagnostic::new("test-code", "test message")
        .with_arg("key", "val")
        .with_help("helpful text")
        .at_span(Span::new(5, 10));

    assert_eq!(diag.code.slug(), "test-code");
    assert_eq!(diag.message, "test message");
    assert_eq!(diag.arg("key"), Some("val"));
    assert_eq!(diag.help.as_deref(), Some("helpful text"));
    assert_eq!(diag.span, Span::new(5, 10));
}

#[test]
fn with_category_sets_category() {
    let diag = Diagnostic::new("x", "y").with_category(Category::Help);
    assert_eq!(diag.category, Category::Help);
}
