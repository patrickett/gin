use parser::cursor::TokenCursor;
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform};

#[test]
fn for_binder_is_available_only_in_loop_body() {
    let positive = transform(
        &TokenCursor::parse_source(
            r#"observe(value Int) Int: return value
main:
    for index in 0 to 3
        observe(index)
"#,
        ),
        FileId(0),
        &TransformCtx::new(),
    );
    assert!(
        positive.all_flaws().is_empty(),
        "unexpected flaws: {:?}",
        positive.all_flaws()
    );

    let typed = transform(
        &TokenCursor::parse_source(
            r#"observe(value Int) Int: return value
main:
    for index in 0 to 3
        observe(index)
    observe(index)
"#,
        ),
        FileId(0),
        &TransformCtx::new(),
    );
    let flaws = typed.all_flaws();
    assert_eq!(flaws.len(), 1, "unexpected flaws: {flaws:?}");
    assert!(
        flaws
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-unknown-symbol")
    );
    assert_eq!(
        flaws
            .iter()
            .filter(|(_, flaw)| flaw.code.slug() == "type-unknown-symbol")
            .count(),
        1
    );
}
