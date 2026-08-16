use diagnostic::Category;
use parser::query::SourceParseExt;

const DOCUMENTS: [(&str, &str); 4] = [
    (
        "docs/wayfinder/inferred-integer-refinements.md",
        include_str!("../../../docs/wayfinder/inferred-integer-refinements.md"),
    ),
    (
        "docs/adr/0011-comparison-result-families-pattern-conditions-and-proof-contracts.md",
        include_str!(
            "../../../docs/adr/0011-comparison-result-families-pattern-conditions-and-proof-contracts.md"
        ),
    ),
    (
        "docs/adr/0012-explicit-linear-ownership-consumption-and-structural-value-capabilities.md",
        include_str!(
            "../../../docs/adr/0012-explicit-linear-ownership-consumption-and-structural-value-capabilities.md"
        ),
    ),
    (
        "docs/adr/0013-fresh-binding-identities-and-explicit-place-rebinding.md",
        include_str!(
            "../../../docs/adr/0013-fresh-binding-identities-and-explicit-place-rebinding.md"
        ),
    ),
];

fn gin_fences(document: &str) -> Vec<(usize, String)> {
    let mut fences = Vec::new();
    let mut current = None;

    for (index, line) in document.lines().enumerate() {
        match (&mut current, line) {
            (None, "```gin") => current = Some((index + 2, String::new())),
            (Some(_), "```") => fences.push(current.take().unwrap()),
            (Some((_, source)), _) => {
                source.push_str(line);
                source.push('\n');
            }
            (None, _) => {}
        }
    }

    assert!(current.is_none(), "unclosed Gin fence");
    fences
}

#[test]
fn every_normative_gin_fence_parses_without_flaws() {
    let mut failures = Vec::new();
    let mut count = 0;

    for (path, document) in DOCUMENTS {
        for (line, source) in gin_fences(document) {
            count += 1;
            let output = source.as_str().parse_source_full();
            let flaws = output
                .symptoms
                .iter()
                .filter(|diagnostic| diagnostic.category == Category::Flaw)
                .map(|diagnostic| format!("{}: {}", diagnostic.code.slug(), diagnostic.message))
                .collect::<Vec<_>>();
            if !flaws.is_empty() {
                failures.push(format!("{path}:{line}: {}", flaws.join("; ")));
            }
        }
    }

    assert_eq!(count, 39, "normative Gin fence inventory changed");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
