use analysis::{CacheEngine, QueryEngine};
use crossbeam_channel::unbounded;
use std::path::PathBuf;

/// Record field types that reference undeclared tags (`List`, `Byte`) without imports.
const STRING_GIN_SOURCE: &str = "\
String has (bytes List(Byte))\n\
\n\
ToString has (to_string String)\n\
";

/// ginlsp maps typecheck diagnostics through the typed span table, not the parse table.
#[test]
fn string_gin_typecheck_spans_match_source() {
    let path = PathBuf::from("/tmp/test_string.gin");
    let source = STRING_GIN_SOURCE;

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    let _ = engine.add_file(path.clone());
    engine.set_contents(&path, source.to_string());

    let outputs = engine.package_semantics(&[path]);
    let diagnostics = &outputs[0].symptoms;

    for diag in diagnostics {
        if diag.code.slug() != "type-unknown-symbol" {
            continue;
        }
        let snippet = source[diag.span.start()..diag.span.end()].trim();
        let name = diag.message.split('`').nth(1).unwrap_or("(unknown)");
        assert_eq!(
            snippet, name,
            "diagnostic span should highlight `{name}`, got `{snippet:?}`"
        );
    }
}
