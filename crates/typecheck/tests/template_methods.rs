use parser::parse_from_str;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{FileId, TypedExprKind, TypedFileAst};

fn check(source: &str) -> TypedFileAst {
    let parsed = parse_from_str(source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    transform(&parsed, FileId(0), &TransformCtx::new())
}

#[test]
fn template_method_names_require_ordinary_declarations() {
    for call in [
        "Template.render()",
        "Template.release()",
        "TemplateSink.write()",
    ] {
        let typed = check(&format!("main: {call}\n"));
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.code.slug() == "type-unknown-symbol"),
            "{call}: {:?}",
            typed.all_flaws()
        );
        assert!(
            !typed
                .exprs
                .kind
                .iter()
                .any(|kind| matches!(kind, TypedExprKind::IntrinsicCall { .. }))
        );
    }
}

#[test]
fn user_defined_template_names_and_methods_have_ordinary_types() {
    let typed = check(
        r#"Word is in 0 to 255
Template has value Word
TemplateSink has value Word
Template.render(self ref) Word: self.value
Template.release(self ref) Word: self.value
TemplateSink.write(self ref) Word: self.value
render(value ref Template) Word: Template.render(ref value)
release(value ref Template) Word: Template.release(ref value)
write(value ref TemplateSink) Word: TemplateSink.write(ref value)
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    for name in ["Template.render", "Template.release", "TemplateSink.write"] {
        assert!(typed.exprs.kind.iter().any(|kind| matches!(kind, TypedExprKind::FnCall { target, .. } if target.0.as_str() == name)), "missing ordinary call to {name}");
    }
    assert!(
        !typed
            .exprs
            .kind
            .iter()
            .any(|kind| matches!(kind, TypedExprKind::IntrinsicCall { .. }))
    );
}

#[test]
fn recovered_template_syntax_cannot_reach_typed_runtime_operations() {
    let parsed = parse_from_str("main: \"value: \\(1)\"\n");
    let typed = transform(&parsed, FileId(0), &TransformCtx::new());
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-template-strings-not-supported"),
        "{:?}",
        typed.all_flaws()
    );
    assert!(
        typed
            .exprs
            .kind
            .iter()
            .any(|kind| matches!(kind, TypedExprKind::Invalid))
    );
}
