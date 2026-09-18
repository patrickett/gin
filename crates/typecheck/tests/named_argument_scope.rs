use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{FileId, TypedExprKind};

#[test]
fn named_arguments_do_not_replace_caller_local_types_or_places() {
    let source = "Word is in 0 to 18446744073709551615\nunrelated(offset Word) Word: offset\nmain Word:\n    offset is in 0 to 18446744073709551615: 0\n    position is in 0 to 18446744073709551615: offset + 0\n    result: unrelated(offset: position as Word)\n    next is in 0 to 18446744073709551615: offset + 1\nreturn next as Word\n";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let flaws = typed.all_flaws();
    assert!(flaws.is_empty(), "{flaws:?}");
    let bindings = typed.exprs.kind.iter().filter(|kind| {
        matches!(kind, TypedExprKind::Bind { name, .. } if name.as_str() == "offset")
    });
    assert_eq!(bindings.count(), 1);
}
