use ast::{BindValue, Expr};
use diagnostic::{Category, Span};
use parser::{cursor::TokenCursor, query::SourceParseExt};

#[test]
fn compiler_holes_preserve_their_spans_and_warn_in_both_parse_apis() {
    let source = "unfinished Word: #todo\n#todo\n";
    let full = source.parse_source_full();
    let direct = TokenCursor::parse_source(source);
    let expected = vec![Span::new(17, 22), Span::new(23, 28)];
    assert_eq!(full.ast.todo_holes, expected);
    assert_eq!(direct.todo_holes, expected);
    for diagnostics in [&full.symptoms, &direct.parse_warnings] {
        let holes = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.slug() == "unfilled-hole")
            .collect::<Vec<_>>();
        assert_eq!(holes.len(), 2, "{diagnostics:?}");
        for (hole, span) in holes.iter().zip(&expected) {
            assert_eq!(hole.category, Category::Warn);
            assert_eq!(hole.span, *span);
            assert_eq!(&source[hole.span.to_range()], "#todo");
        }
        assert!(
            !diagnostics
                .iter()
                .any(|diagnostic| diagnostic.category == Category::Flaw)
        );
    }
    let BindValue::Expr(body) = &full.ast.defs.values().next().unwrap().value else {
        panic!("expression body");
    };
    assert_eq!(body.value, Expr::Todo);
    assert_eq!(full.ast.exprs[0].0, Expr::Todo);
}

#[test]
fn statement_and_method_holes_are_recorded_without_duplicate_warnings() {
    let source = "outer:\n    #todo\n    return 42\nThing has\n    unfinished(self) Word: return #todo\n";
    let parsed = source.parse_source_full();
    assert_eq!(parsed.ast.todo_holes.len(), 2, "{:?}", parsed.symptoms);
    assert_eq!(
        parsed
            .symptoms
            .iter()
            .filter(|diagnostic| diagnostic.code.slug() == "unfilled-hole")
            .count(),
        2
    );
    assert!(
        !parsed
            .symptoms
            .iter()
            .any(|diagnostic| diagnostic.category == Category::Flaw),
        "{:?}",
        parsed.symptoms
    );
}

#[test]
fn bare_todo_names_comments_and_strings_do_not_create_holes() {
    let source = "todo := 42\ntext := '#todo'\n-- #todo\n";
    let parsed = source.parse_source_full();
    assert!(parsed.ast.todo_holes.is_empty());
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    assert!(
        parsed
            .ast
            .defs
            .values()
            .any(|bind| bind.name.as_str() == "todo")
    );
}

#[test]
fn holes_do_not_hide_parse_errors() {
    let parsed = "unfinished: #todo\nbroken := @\n".parse_source_full();
    assert_eq!(parsed.ast.todo_holes.len(), 1);
    assert!(
        parsed
            .symptoms
            .iter()
            .any(|diagnostic| diagnostic.category == Category::Flaw)
    );
}
