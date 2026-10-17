mod support;

use parser::parse_from_str;
use support::check_lexed;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{FileId, TypedExprKind};

#[test]
fn template_method_names_require_ordinary_declarations() {
    for call in [
        "Template.render()",
        "Template.release()",
        "TemplateSink.write()",
    ] {
        let typed = check_lexed(&format!("main: {call}\n"));
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
    let typed = check_lexed(&format!(
        "{}Template has\n    value Word\n    finish(self):\n        return\nTemplateSink has\n    value Word\n    finish(self):\n        return\nTemplate.render(self ref) Word: return self.value\nTemplate.release(self ref) Word: return self.value\nTemplateSink.write(self ref) Word: return self.value\nrender(value ref Template) Word: return Template.render(value)\nrelease(value ref Template) Word: return Template.release(value)\nwrite(value ref TemplateSink) Word: return TemplateSink.write(value)\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
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
