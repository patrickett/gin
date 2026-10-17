use super::*;
use crate::FileId;
use crate::test_support::push_byte_expr;
use crate::typed::{ExprId, TypedExprKind, TypedFileAst};
use ast::{Literal, SpanTable, Ty};

#[test]
fn projection_path_preserves_order_and_rejects_unsupported_paths() {
    let mut typed = TypedFileAst::new(FileId(0), SpanTable::new());
    let root = push_byte_expr(&mut typed, TypedExprKind::Lit(Literal::Int(0.into())));
    let inner = push_byte_expr(
        &mut typed,
        TypedExprKind::AggregateGet {
            base: root,
            index: 2,
        },
    );
    let outer = push_byte_expr(
        &mut typed,
        TypedExprKind::AggregateGet {
            base: inner,
            index: 1,
        },
    );
    assert_eq!(
        LoopTargets::projection_path(&typed, outer),
        Some((root, vec![1, 2]))
    );
    assert_eq!(LoopTargets::projection_path(&typed, ExprId(99)), None);
    typed.exprs.ty[inner.index()] = Ty::Ref {
        inner: Box::new(Ty::Unit),
        mutable: false,
    };
    assert_eq!(LoopTargets::projection_path(&typed, outer), None);
    typed.exprs.ty[inner.index()] = Ty::Unit;
    typed.exprs.kind[inner.index()] = TypedExprKind::AggregateGet {
        base: outer,
        index: 2,
    };
    assert_eq!(LoopTargets::projection_path(&typed, outer), None);
}

#[test]
fn projection_path_handles_deep_chains_on_small_stack() {
    let mut typed = TypedFileAst::new(FileId(0), SpanTable::new());
    let root = push_byte_expr(&mut typed, TypedExprKind::Lit(Literal::Int(0.into())));
    let mut expression = root;
    for index in 0..2000 {
        expression = push_byte_expr(
            &mut typed,
            TypedExprKind::AggregateGet {
                base: expression,
                index,
            },
        );
    }
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(move || {
            let (base, fields) = LoopTargets::projection_path(&typed, expression).unwrap();
            assert_eq!(base, root);
            assert_eq!(fields, (0..2000).rev().collect::<Vec<_>>());
        })
        .unwrap()
        .join()
        .unwrap();
}
