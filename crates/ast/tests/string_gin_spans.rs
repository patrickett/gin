mod support;
use ast::source::SourceExt;
use support::transform_source;

/// Record field types that reference undeclared tags (`List`, `Byte`) without imports.
const STRING_GIN_SOURCE: &str = "\
String has bytes List(Byte)\n\
\n\
ToString has to_string String\n\
";

#[test]
fn string_gin_unknown_tag_spans() {
    let source = STRING_GIN_SOURCE;
    let typed = transform_source(source);

    for (span_id, flaw) in typed.all_flaws() {
        if flaw.code.slug() != "type-unknown-symbol" {
            continue;
        }
        let name = flaw.arg("name").unwrap_or("");
        let span = typed.span_table.get(span_id);
        let snippet = &source[span.start()..span.end()];
        eprintln!(
            "UnknownSymbol `{name}`: bytes {:?} snippet={snippet:?}",
            span.start()..span.end()
        );
        let (line, col) = source.byte_offset_to_position(span.start());
        eprintln!("  LSP line={line} col={col}");
        assert_eq!(
            snippet.trim(),
            name,
            "highlight should be exactly `{name}`, got `{snippet:?}` at bytes {:?}",
            span.start()..span.end()
        );
    }
}
