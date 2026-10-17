use diagnostic::Category;
use parser::query::SourceParseExt;

mod common;

use common::assert_roundtrip;

#[test]
fn compiler_holes_round_trip_in_declarations_and_expressions() {
    let source = "unfinished Word:#todo\n\nouter:\n    #todo\n    return 42\n\n#todo\n";
    let formatted = ginfmt::format(source);
    let parsed = formatted.parse_source_full();
    assert_eq!(parsed.ast.todo_holes.len(), 3, "{formatted}");
    assert!(
        !parsed
            .symptoms
            .iter()
            .any(|diagnostic| diagnostic.category == Category::Flaw),
        "{:?}",
        parsed.symptoms
    );
    assert!(formatted.contains("unfinished Word: #todo"), "{formatted}");
    assert_roundtrip(&formatted);
}
