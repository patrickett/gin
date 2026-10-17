use diagnostic::Category;
use parser::query::SourceParseExt;

#[test]
fn calls_reject_ownership_prefixes() {
    for argument in [
        "ref value",
        "mut value",
        "own value",
        "value: ref item",
        "value: mut item",
        "value: own item",
        "(ref value)",
        "(mut value)",
    ] {
        for callee in ["read", "value.read", "Box"] {
            let source = format!("run: {callee}({argument})\n");
            let parsed = source.parse_source_full();
            assert!(
                parsed
                    .symptoms
                    .iter()
                    .any(|diagnostic| diagnostic.category == Category::Flaw),
                "{source}: {:?}",
                parsed.symptoms
            );
        }
    }
}

#[test]
fn declarations_and_reference_values_keep_their_conventions() {
    let source = "read(value ref Item): return value\nwrite(value mut Item): return value\ntake(value own Item): return value\nrun(value mut Item):\n    borrowed: ref value\n    writable: mut value\n    read(borrowed)\n    write(writable)\n    return\n";
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
}
