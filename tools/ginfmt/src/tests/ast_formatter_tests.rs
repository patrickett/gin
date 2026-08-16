use super::*;
use parser::query::SourceParseExt;

#[test]
fn test_basic_declare() {
    let source = "Maybe is Some or None\nResult is Ok or Error\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let r = f.format_file(&out.ast);
    assert_eq!(r, "Maybe is Some or None\nResult is Ok or Error\n");
}

#[test]
fn test_has_members_use_canonical_no_paren_syntax() {
    let source = "Range(x) has start x, contains(self, value x) Bool\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let r = f.format_file(&out.ast);
    assert_eq!(r, "Range(x) has start x, contains(self, value x) Bool\n");
}

#[test]
fn composed_interface_preserves_qualified_methods() {
    let source = "TraitA has run(ref self) Int\nTraitB has run(ref self) Int\nCombined has TraitA and TraitB\n    TraitA.run(ref self) Int := 1\n    TraitB.run(ref self) Int := 2\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut formatter = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let formatted = formatter.format_file(&out.ast);
    assert!(
        formatted.contains("TraitA has run(ref self) Int")
            || formatted.contains("TraitA has run(self) Int"),
        "formatted output: {formatted:?}",
    );
    assert_eq!(formatted.matches("TraitA.run").count(), 1);
    assert_eq!(formatted.matches("TraitB.run").count(), 1);
}

#[test]
fn test_simple_bind() {
    let source = "main:\n    print('hello')\nreturn\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let r = f.format_file(&out.ast);
    assert!(r.contains("main:"));
    assert!(r.contains("print('hello')"));
    assert!(r.contains("return"));
}

#[test]
fn test_empty() {
    let source = "";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let r = f.format_file(&out.ast);
    assert_eq!(r, "");
}

#[test]
fn test_import_sort_current_module() {
    let source = "use ToString, Copy\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let r = f.format_file(&out.ast);
    assert_eq!(r, "use Copy, ToString\n");
}

#[test]
fn test_import_sort_package() {
    let source = "use http.web, crypto.hash\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let r = f.format_file(&out.ast);
    assert_eq!(r, "use crypto.hash, http.web\n");
}

#[test]
fn test_import_sort_single() {
    let source = "use http\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let r = f.format_file(&out.ast);
    assert_eq!(r, "use http\n");
}

#[test]
fn test_bundle_member_sort() {
    let source = "use core.(Int, Byte, Area)\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let r = f.format_file(&out.ast);
    assert_eq!(r, "use core.(Area, Byte, Int)\n");
}

#[test]
fn test_import_sort_idempotent() {
    let source = "use Copy, ToString\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let r = f.format_file(&out.ast);
    assert_eq!(r, "use Copy, ToString\n");
}

#[test]
fn test_formatter_retains_bind_rebind_and_projection_write_forms() {
    let source = "\
counter Int\n\
counter:: 0\n\
result: counter\n\
pair.0:: counter\n\
pair.0 +: counter + 1\n\
buffer.(0):: 1\n\
buffer.(0) +: 2\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut formatter = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let formatted = formatter.format_file(&out.ast);
    assert!(formatted.contains("counter:: 0"));
    assert!(formatted.contains("result: counter"));
    assert!(formatted.contains("pair.0:: counter"), "{formatted}");
    assert!(formatted.contains("pair.0 +: counter + 1"));
    assert!(formatted.contains("buffer.(0):: 1"));
    assert!(formatted.contains("buffer.(0) +: 2"));
}
