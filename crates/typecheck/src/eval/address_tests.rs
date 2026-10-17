use super::*;
use crate::intrinsic::IntrinsicOp;
use crate::test_support::push_expr_typed;

fn fixture() -> (crate::TypedFileAst, ExprId, ExprId) {
    let mut typed = crate::TypedFileAst::new(crate::FileId(0), ast::SpanTable::new());
    let word = Ty::anonymous_integer_for_width(8, false);
    let pointer_ty = Ty::Address {
        pointee: Box::new(word.clone()),
        address_space: 0,
    };
    assert_eq!(
        IntrinsicOp::AddressStore.validate_signature(
            &[pointer_ty.clone(), word.clone()],
            &Ty::Unit,
            None
        ),
        Ok(())
    );
    let pointer = push_expr_typed(
        &mut typed,
        TypedExprKind::SelfRef {
            target: crate::DefId::named(Intern::from_ref("pointer")),
        },
        pointer_ty,
    );
    let value = push_expr_typed(
        &mut typed,
        TypedExprKind::Lit(Literal::Int(7.into())),
        word.clone(),
    );
    let store = push_expr_typed(
        &mut typed,
        TypedExprKind::IntrinsicCall {
            op: IntrinsicOp::AddressStore,
            args: vec![pointer, value],
        },
        Ty::Unit,
    );
    let load = push_expr_typed(
        &mut typed,
        TypedExprKind::IntrinsicCall {
            op: IntrinsicOp::AddressLoad,
            args: vec![pointer],
        },
        word,
    );
    (typed, store, load)
}

#[test]
fn address_store_returns_unit_and_updates_the_referent() {
    let (typed, store, load) = fixture();
    let target = flask::CompileTarget::Library;
    let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
    let address = evaluator.heap.allocate(ConstValue::ONE);
    evaluator.locals.push(HashMap::from([(
        Intern::from_ref("pointer"),
        LocalValue::Reference(address),
    )]));
    assert_eq!(
        evaluator.complete(store),
        Ok(ConstValue::Record {
            fields: Vec::new().into()
        })
    );
    assert_eq!(evaluator.complete(load), Ok(ConstValue::Int(7.into())));
    assert_eq!(
        evaluator.heap.freeze(address),
        Ok(ConstValue::Int(7.into()))
    );
}

#[test]
fn an_invalid_store_does_not_change_other_storage() {
    let (typed, store, _) = fixture();
    let target = flask::CompileTarget::Library;
    let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
    let address = evaluator.heap.allocate(ConstValue::ONE);
    let invalid = address.offset(1).unwrap();
    evaluator.locals.push(HashMap::from([(
        Intern::from_ref("pointer"),
        LocalValue::Reference(invalid),
    )]));
    assert!(evaluator.complete(store).is_err());
    assert_eq!(evaluator.heap.freeze(address), Ok(ConstValue::ONE));
}
