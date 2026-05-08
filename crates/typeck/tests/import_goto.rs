//! Goto-definition helpers for `use`-introduced names.

use parser::expr::parse_source;
use typeck::find_import_definition_span;

#[test]
fn import_span_use_package_root_only() {
    let src = "use core\n\nmain:\n    core\n";
    let ast = parse_source(src);
    let span = find_import_definition_span(&ast, "core", src).expect("import span");
    assert_eq!(&src[span.start..span.end], "core");
}

#[test]
fn import_span_use_package_with_segment() {
    let src = "use core.io\n\nmain:\n    io\n";
    let ast = parse_source(src);
    let span = find_import_definition_span(&ast, "io", src).expect("import span");
    assert_eq!(&src[span.start..span.end], "io");
}

#[test]
fn import_span_none_when_not_imported() {
    let src = "main:\n    xyzzy\n";
    let ast = parse_source(src);
    assert!(find_import_definition_span(&ast, "xyzzy", src).is_none());
}

#[test]
fn import_span_use_package_multi_segment_returns_last() {
    let src = "use a.b.c\n\nmain:\n    c\n";
    let ast = parse_source(src);
    let span = find_import_definition_span(&ast, "c", src).expect("import span");
    assert_eq!(&src[span.start..span.end], "c");
}
