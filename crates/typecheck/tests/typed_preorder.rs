use ast::{Literal, SpanId, SpanTable, Ty};
use typecheck::FileId;
use typecheck::typed::{
    Availability, ConstFacts, ExprFacts, ExprId, OwnershipFacts, SemanticFacts, TypedExpr,
    TypedExprKind, TypedFileAst,
};

fn push(typed: &mut TypedFileAst, kind: TypedExprKind) -> ExprId {
    let id = ExprId(u32::try_from(typed.exprs.kind.len()).unwrap());
    typed.exprs.push(TypedExpr {
        kind,
        ty: Ty::bounded_int(0.into(), 0.into()),
        span: SpanId::INVALID,
        facts: ExprFacts {
            const_facts: ConstFacts {
                const_value: None,
                const_object: None,
                sequence_literal: None,
                literal_materialization: None,
            },
            semantic_facts: SemanticFacts {
                integer_knowledge: None,
                result_evidence: None,
                projected_result_evidence: Default::default(),
                availability: Availability::Unknown,
                flaws: vec![],
            },
            ownership_facts: OwnershipFacts {
                references: Default::default(),
                reference_reads: Default::default(),
                target_group: None,
                result_target_groups: vec![],
                place: None,
                place_version: None,
            },
        },
    });
    id
}

#[test]
fn preorder_preserves_order_cycles_and_missing_nodes() {
    use std::ops::ControlFlow;
    let mut typed = TypedFileAst::new(FileId(0), SpanTable::new());
    let leaf = push(&mut typed, TypedExprKind::Lit(Literal::Int(0.into())));
    let left = push(&mut typed, TypedExprKind::Negate(leaf));
    let right = push(&mut typed, TypedExprKind::Negate(leaf));
    let missing = ExprId(99);
    let root = push(
        &mut typed,
        TypedExprKind::ArrayLit(vec![left, right, left, missing]),
    );
    typed.exprs.kind[leaf.index()] = TypedExprKind::Negate(root);
    let mut visited = Vec::new();
    assert_eq!(
        typecheck::typed::walk_expr_preorder(&typed, root, &mut |id| {
            visited.push(id);
            ControlFlow::Continue(())
        }),
        ControlFlow::Continue(())
    );
    assert_eq!(visited, vec![root, left, leaf, right]);
    visited.clear();
    assert_eq!(
        typecheck::typed::walk_expr_preorder(&typed, missing, &mut |id| {
            visited.push(id);
            ControlFlow::Continue(())
        }),
        ControlFlow::Continue(())
    );
    assert!(visited.is_empty());
    assert_eq!(
        typecheck::typed::walk_expr_preorder(&typed, root, &mut |id| {
            visited.push(id);
            if id == left {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }),
        ControlFlow::Break(())
    );
    assert_eq!(visited, vec![root, left]);
}

#[test]
fn preorder_handles_deep_chains_on_small_stack() {
    const CHILD: &str = "GIN_TYPED_PREORDER_SMALL_STACK";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "preorder_handles_deep_chains_on_small_stack",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let mut typed = TypedFileAst::new(FileId(0), SpanTable::new());
    let count = 2000;
    let mut root = push(&mut typed, TypedExprKind::Lit(Literal::Int(0.into())));
    for _ in 1..count {
        root = push(&mut typed, TypedExprKind::Negate(root));
    }
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(move || {
            let mut next = count;
            let result = typecheck::typed::walk_expr_preorder(&typed, root, &mut |id| {
                next -= 1;
                assert_eq!(id.index(), next);
                std::ops::ControlFlow::Continue(())
            });
            assert_eq!(result, std::ops::ControlFlow::Continue(()));
            assert_eq!(next, 0);
        })
        .unwrap()
        .join()
        .unwrap();
}
