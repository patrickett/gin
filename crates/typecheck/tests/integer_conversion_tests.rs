use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{BindBody, DefId, FileId, TypedExprKind};

fn check(source: &str) -> typecheck::TypedFileAst {
    transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    )
}

fn flaw_codes(typed: &typecheck::TypedFileAst) -> Vec<String> {
    typed
        .all_flaws()
        .into_iter()
        .map(|(_, flaw)| flaw.code.slug().to_string())
        .collect()
}

#[test]
fn exact_anonymous_value_proves_nominal_conversion_containment() {
    let typed = check("Tiny is in 0...15\nconverted: 10 as Tiny");
    let codes = flaw_codes(&typed);
    assert!(
        !codes
            .iter()
            .any(|code| code.starts_with("type-integer-narrowing"))
    );
    let BindBody::Expr(converted) = typed.defs[&DefId(Intern::from_ref("converted"))].body else {
        panic!("expected cast body");
    };
    assert!(matches!(
        &typed.exprs.kind[converted.as_usize()],
        TypedExprKind::Cast { ty: ast::Ty::Named { name, .. }, .. } if name.as_str() == "Tiny"
    ));
}

#[test]
fn exact_anonymous_value_outside_nominal_validity_fails_conversion() {
    let typed = check("Tiny is in 0...15\nconverted: 20 as Tiny");
    let codes = flaw_codes(&typed);
    assert!(
        codes
            .iter()
            .any(|code| code == "type-integer-narrowing-failed"),
        "{codes:?}"
    );
}

#[test]
fn symbolic_anonymous_domain_leaves_nominal_conversion_unproven() {
    let typed = check("Tiny is in 0...15\nsource() is < limit: 0\nconverted: source() as Tiny");
    let codes = flaw_codes(&typed);
    assert!(
        codes
            .iter()
            .any(|code| code == "type-integer-narrowing-unproven"),
        "{codes:?}"
    );
}
