use ast::ty::PackageInstanceKey;
use diagnostic::Category;
use parser::query::SourceParseExt;
use typecheck::transform::{
    PackageTransformOptions, transform_package_with_shared_context_and_package,
};
use typecheck::{TypedExprKind, TypedFileAst};

fn check(source: &str) -> TypedFileAst {
    let parsed = source.parse_source_full();
    assert!(
        parsed.symptoms.iter().all(|d| d.category != Category::Flaw),
        "{:?}",
        parsed.symptoms
    );
    transform_package_with_shared_context_and_package(
        vec![parsed.ast],
        PackageTransformOptions::IDE,
        PackageInstanceKey::workspace("fixture", "1"),
    )
    .typed_asts
    .pop()
    .unwrap()
}

#[test]
fn unresolved_method_chains_lower_each_call_once() {
    for depth in [1, 4, 12] {
        let source = format!("value: seed(0){}\n", ".missing(0)".repeat(depth));
        let typed = check(&source);
        let calls = typed
            .exprs
            .kind
            .iter()
            .filter(|kind| matches!(kind, TypedExprKind::FnCall { .. }))
            .count();
        assert_eq!(calls, depth + 1, "chain depth {depth}");
        let flaws = typed.all_flaws();
        let unknown = flaws
            .iter()
            .filter(|(_, flaw)| flaw.code.slug() == "type-unknown-symbol")
            .count();
        assert_eq!(unknown, depth + 1, "chain depth {depth}: {flaws:?}");
    }
}

#[test]
fn implicit_receiver_calls_lower_arguments_once() {
    for methods in [
        "Box.accept(value Int) Int: value\n",
        "Box.accept(value Int) Int: value\nBox.accept(value Int, extra Int) Int: extra\n",
    ] {
        for depth in [1, 4, 12] {
            let argument = format!("{}missing{}", "accept(".repeat(depth), ")".repeat(depth));
            let source = format!("Box has value Int\n{methods}Box.run Int: {argument}\n");
            let typed = check(&source);
            let missing = typed
                .exprs
                .kind
                .iter()
                .filter(|kind| {
                    matches!(kind, TypedExprKind::FnCall { target, .. } if target.0.as_str() == "missing")
                })
                .count();
            assert_eq!(missing, 1, "depth {depth}: {:?}", typed.all_flaws());
            assert_eq!(typed.all_flaws().len(), 1, "{:?}", typed.all_flaws());
        }
    }
}

#[test]
fn implicit_receiver_argument_receives_literal_context_on_first_lowering() {
    let typed = check(
        "Byte is in 0 to 255\nBox has value Byte\nBox.accept(value Byte) Byte: value\nBox.run Byte: accept(42)\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let literals: Vec<_> = typed
        .exprs
        .kind
        .iter()
        .enumerate()
        .filter(|(_, kind)| matches!(kind, TypedExprKind::Lit(_)))
        .collect();
    assert_eq!(literals.len(), 1);
    assert_eq!(
        typed.exprs.ty[literals[0].0].type_name().unwrap().as_str(),
        "Byte"
    );
}

#[test]
fn resolved_method_chains_lower_each_receiver_once() {
    for depth in [1, 4, 12] {
        let source = format!(
            "Box has value Int\nBox.new(value Int) Box: Box(value: value)\nBox.next(box Box, value Int) Box: box\nresult(value Int) Box: Box.new(value){}\n",
            ".next(0)".repeat(depth)
        );
        let typed = check(&source);
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let calls = typed
            .exprs
            .kind
            .iter()
            .filter(|kind| {
                matches!(kind, TypedExprKind::FnCall { target, .. } if target.0.as_str() == "Box.next")
            })
            .count();
        assert_eq!(calls, depth, "chain depth {depth}");
    }
}
