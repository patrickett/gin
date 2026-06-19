//! `package_semantics` merges type flaws with non-fatal parse symptoms on the typed span table.

use analysis::{CacheEngine, FileSemanticOutput, QueryEngine};
use crossbeam_channel::unbounded;
use diagnostic::{Category, Diagnostic};
use std::path::PathBuf;

fn is_unexpected_equals_token(diag: &Diagnostic) -> bool {
    diag.code.slug() == "parse-unexpected-token"
        && diag.message.contains("unexpected token")
        && diag.message.contains("`=`")
}

const STRING_GIN_SOURCE: &str = "\
String has (bytes List(Byte))\n\
\n\
ToString has (to_string String)\n\
";

#[test]
fn package_semantics_spans_match_unknown_symbols() {
    let path = PathBuf::from("/tmp/test_package_semantics.gin");
    let source = STRING_GIN_SOURCE;

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    let _ = engine.add_file(path.clone());
    engine.set_contents(&path, source.to_string());

    let outputs = engine.package_semantics(&[path]);
    let FileSemanticOutput { symptoms } = &outputs[0];

    for diag in symptoms {
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

#[test]
fn package_semantics_includes_non_flaw_parse_symptoms() {
    let path = PathBuf::from("/tmp/test_parse_merge.gin");
    let source = "x := 1\n";

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    let _ = engine.add_file(path.clone());
    engine.set_contents(&path, source.to_string());

    let po = engine.parse_output(&path).expect("parse");
    let parse_non_flaw: Vec<_> = po
        .symptoms
        .iter()
        .filter(|d| !matches!(d.category, Category::Flaw))
        .collect();

    if parse_non_flaw.is_empty() {
        return;
    }

    let outputs = engine.package_semantics(&[path]);
    let sem = &outputs[0];
    for expected in parse_non_flaw {
        assert!(
            sem.symptoms.iter().any(|d| d.message == expected.message),
            "package_semantics should include parse symptom: {}",
            expected.message
        );
    }
}

#[test]
fn package_semantics_includes_deprecated_token_flaw() {
    let path = PathBuf::from("/tmp/test_deprecated_token.gin");
    // A standalone `=` should produce a ParseSymptom::UnexpectedToken flaw.
    let source = "=\n";

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    let _ = engine.add_file(path.clone());
    engine.set_contents(&path, source.to_string());

    // Verify the parse output contains the deprecated token symptom
    let po = engine.parse_output(&path).expect("parse");
    let has_token_flaw = po.symptoms.iter().any(is_unexpected_equals_token);
    assert!(
        has_token_flaw,
        "parse_output should contain deprecation error for `=`, got symptoms: {:?}",
        po.symptoms
            .iter()
            .map(|d| (d.code.slug().to_string(), d.message.clone()))
            .collect::<Vec<_>>()
    );

    // Verify package_semantics also has it (proving it flows through the pipeline)
    let outputs = engine.package_semantics(std::slice::from_ref(&path));
    assert!(
        !outputs.is_empty(),
        "package_semantics should produce output"
    );
    let has_in_semantics = outputs[0].symptoms.iter().any(is_unexpected_equals_token);
    assert!(
        has_in_semantics,
        "package_semantics should include deprecation error for `=`, got symptoms: {:?}",
        outputs[0]
            .symptoms
            .iter()
            .map(|d| (d.code.slug().to_string(), d.message.clone()))
            .collect::<Vec<_>>()
    );

    // Verify all_diagnostics has it (the API the LSP calls)
    let all_diags = engine.all_diagnostics(
        std::slice::from_ref(&path),
        &std::collections::HashMap::new(),
    );
    let file_diags = all_diags.get(&path);
    assert!(
        file_diags.is_some(),
        "all_diagnostics should have entry for the test file"
    );
    let has_in_all = file_diags.unwrap().iter().any(is_unexpected_equals_token);
    assert!(
        has_in_all,
        "all_diagnostics should include deprecation error for `=`, got: {:?}",
        file_diags
            .unwrap()
            .iter()
            .map(|d| (d.code.slug().to_string(), d.message.clone()))
            .collect::<Vec<_>>()
    );
}
