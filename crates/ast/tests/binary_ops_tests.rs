//! Binary arithmetic: parsing, `ConstValue::eval_binop`, compile-time `:=` folding, and lowering.

use ast::prelude::*;
use ast::{ConstValue, HashFloat};
use flask::CompileTarget;
use internment::Intern;
use std::collections::HashMap;
use typecheck::analysis::{CompTimeEvaluator, ConstEnv, TyInfer, TyInferEnv};
use typecheck::intrinsic::IntrinsicOp;
use typecheck::prepare_parse_ast;
use typecheck::ty::Ty;
use typecheck::{DefId, ExprId, TypedExprKind};

mod support;
use support::transform_source;

fn prepare(source: &str) -> ast::FileAst {
    let mut ast = parser::cursor::TokenCursor::parse_source(source);
    let _ = prepare_parse_ast(&mut ast, &CompileTarget::Library);
    ast
}

fn body_expr_id(typed: &typecheck::TypedFileAst, def_name: &str) -> Option<ExprId> {
    let def_id = DefId(Intern::new(def_name.to_string()));
    let bind = typed.defs.get(&def_id)?;
    match &bind.body {
        typecheck::BindBody::Expr(eid) => Some(*eid),
        typecheck::BindBody::Body { exprs, ret } => ret.or_else(|| exprs.last().copied()),
        typecheck::BindBody::Extern => None,
    }
}

fn typed_bind_const_value<'a>(
    typed: &'a typecheck::TypedFileAst,
    name: &str,
) -> Option<&'a ConstValue> {
    let id = body_expr_id(typed, name)?;
    typed.exprs.get(id.as_usize())?.const_value.as_ref()
}

fn assert_const_int(cv: Option<&ConstValue>, n: i128) {
    assert_eq!(
        cv.and_then(ConstValue::as_const_size_int),
        Some(n),
        "expected const int {n}, got {cv:?}"
    );
}

#[test]
fn eval_binop_int_arithmetic_uses_size_const_tags() {
    // Plain `Int` operands route through `as_const_size_int` and fold to `Size::Const(n)`.
    assert_const_int(
        ConstValue::Int(1.into())
            .eval_binop(&BinOp::Add, &ConstValue::Int(1.into()))
            .as_ref(),
        2,
    );
    assert_const_int(
        ConstValue::Int(10.into())
            .eval_binop(&BinOp::Subtract, &ConstValue::Int(3.into()))
            .as_ref(),
        7,
    );
    assert_const_int(
        ConstValue::Int(6.into())
            .eval_binop(&BinOp::Multiply, &ConstValue::Int(7.into()))
            .as_ref(),
        42,
    );
    assert_const_int(
        ConstValue::Int(20.into())
            .eval_binop(&BinOp::Divide, &ConstValue::Int(4.into()))
            .as_ref(),
        5,
    );
    assert_eq!(
        ConstValue::Int(17.into()).eval_binop(&BinOp::Modulo, &ConstValue::Int(5.into())),
        Some(ConstValue::Int(2.into()))
    );
}

#[test]
fn eval_binop_int_divide_by_zero_is_none() {
    assert_eq!(
        ConstValue::Int(1.into()).eval_binop(&BinOp::Divide, &ConstValue::Int(0.into())),
        None
    );
}

#[test]
fn eval_binop_float_arithmetic() {
    assert_eq!(
        ConstValue::Float(HashFloat(1.5))
            .eval_binop(&BinOp::Add, &ConstValue::Float(HashFloat(2.5))),
        Some(ConstValue::Float(HashFloat(4.0)))
    );
}

#[test]
fn eval_binop_equality_returns_bool_tags() {
    assert_eq!(
        ConstValue::Int(7.into()).eval_binop(&BinOp::Equal, &ConstValue::Int(7.into())),
        Some(ConstValue::Tag {
            name: Intern::from_ref("True"),
            qual_path: None,
            args: vec![].into(),
        })
    );
    assert_eq!(
        ConstValue::Int(7.into()).eval_binop(&BinOp::Equal, &ConstValue::Int(8.into())),
        Some(ConstValue::Tag {
            name: Intern::from_ref("False"),
            qual_path: None,
            args: vec![].into(),
        })
    );
}

#[test]
fn anonymous_integer_equality_has_structural_result_family_and_folds_constants() {
    let typed = transform_source("same := 7 = 7\n");
    let expression = body_expr_id(&typed, "same").expect("same body");

    assert!(matches!(
        typed.exprs.kind[expression.as_usize()],
        TypedExprKind::IntrinsicCall {
            op: typecheck::intrinsic::IntrinsicOp::BitsCompare {
                predicate: typecheck::intrinsic::ComparisonPredicate::Equal,
                ..
            },
            ..
        }
    ));
    assert!(matches!(
        typed.exprs.ty[expression.as_usize()],
        ast::Ty::ResultFamily {
            owner: ast::ResultFamilyOwner::Structural(ast::OperatorRole::Equal),
            ..
        }
    ));
    assert!(
        matches!(
            typed.exprs.const_value[expression.as_usize()],
            Some(ConstValue::ResultAlternative {
                owner: ast::ResultFamilyOwner::Structural(ast::OperatorRole::Equal),
                label,
            }) if label.as_str() == "True"
        ),
        "{:?}",
        typed.exprs.const_value[expression.as_usize()]
    );
}

#[test]
fn equality_rejects_distinct_nominal_types() {
    let typed = transform_source(
        "Left is in 0...255\nRight is in 0...255\nBool is True or False\nsame(a Left, b Right) Bool: a = b\n",
    );

    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-nominal-operator-unavailable")
    );
}

#[test]
fn equality_rejects_pointer_values() {
    let typed = transform_source(
        "Int is in 0...255\nBool is True or False\nPointer(x) is @x\nsame(a Pointer(Int), b Pointer(Int)) Bool: a = b\n",
    );

    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-nominal-operator-unavailable")
    );
}

#[test]
fn compile_time_bind_folds_addition() {
    let typed = transform_source(
        "Word is in 0...255\n#intrinsic(BitsAdd)\nadd_bits(a Word, b Word) Word extern\n#operator(Add)\n#inline\nword_add(a Word, b Word) Word: add_bits(a, b)\ntwo() Word:\n    a Word := 1\n    b Word := 1\nreturn a + b\n",
    );
    assert_const_int(typed_bind_const_value(&typed, "two"), 2);
}

#[test]
fn compile_time_bind_folds_nested_addition() {
    let typed = transform_source(
        "Word is in 0...255\n#intrinsic(BitsAdd)\nadd_bits(a Word, b Word) Word extern\n#operator(Add)\n#inline\nword_add(a Word, b Word) Word: add_bits(a, b)\nsum() Word:\n    a Word := 1\n    b Word := 2\n    c Word := 3\nreturn (a + b) + c\n",
    );
    assert_const_int(typed_bind_const_value(&typed, "sum"), 6);
}

#[test]
fn eval_compile_time_expr_folds_addition() {
    let main = parser::cursor::TokenCursor::parse_source("x: 1 + 2\n");
    let const_binds: ConstEnv = main.defs.keys().map(|name| (*name, None)).collect();
    let bind = main.defs.get(&Intern::from_ref("x")).unwrap();
    let ast::BindValue::Expr(e) = &bind.value else {
        panic!("expected expr bind");
    };
    let evaluator = CompTimeEvaluator::new(&const_binds, &main);
    let cv = evaluator.eval(&e.value).expect("fold 1+2");
    assert_const_int(Some(&cv), 3);
}

#[test]
fn compile_time_function_body_exposes_local_binds_to_return() {
    let ast = parser::cursor::TokenCursor::parse_source(
        "identity(value Int) Int :=\n    result := value\n    doubled := result + result\nreturn doubled\nanswer := identity(42)\n",
    );
    let const_binds = ast.defs.keys().map(|name| (*name, None)).collect();
    let answer = ast.defs.get(&Intern::from_ref("answer")).expect("answer");
    let ast::BindValue::Expr(expr) = &answer.value else {
        panic!("expected expression body");
    };
    let evaluator = CompTimeEvaluator::new(&const_binds, &ast);

    let value = evaluator.eval(&expr.value);

    assert_const_int(value.as_ref(), 84);
}

#[test]
fn compile_time_call_with_wrong_arity_is_not_evaluated() {
    let ast = parser::cursor::TokenCursor::parse_source(
        "pick(first Int, second Int) Int := first\nanswer := pick(1)\n",
    );
    let const_binds = ast.defs.keys().map(|name| (*name, None)).collect();
    let answer = ast.defs.get(&Intern::from_ref("answer")).expect("answer");
    let ast::BindValue::Expr(expr) = &answer.value else {
        panic!("expected expression body");
    };
    let evaluator = CompTimeEvaluator::new(&const_binds, &ast);

    assert_eq!(evaluator.eval(&expr.value), None);
}

#[test]
fn recursive_compile_time_call_is_not_evaluated() {
    let ast = parser::cursor::TokenCursor::parse_source(
        "recurse(value Int) Int := recurse(value)\nanswer := recurse(1)\n",
    );
    let const_binds = ast.defs.keys().map(|name| (*name, None)).collect();
    let answer = ast.defs.get(&Intern::from_ref("answer")).expect("answer");
    let ast::BindValue::Expr(expr) = &answer.value else {
        panic!("expected expression body");
    };
    let evaluator = CompTimeEvaluator::new(&const_binds, &ast);

    assert_eq!(evaluator.eval(&expr.value), None);
}

#[test]
fn infer_ty_leaves_literal_addition_unresolved() {
    let file = parser::cursor::TokenCursor::parse_source("main: 1 + 1\n");
    let bind = file.defs.get(&Intern::from_ref("main")).unwrap();
    let ast::BindValue::Expr(e) = &bind.value else {
        panic!("expected expr");
    };
    let ast::Expr::Binary(bin) = &e.value else {
        panic!("expected binary body");
    };
    let env = TyInferEnv {
        tag_types: &HashMap::new(),
        fn_return_types: &HashMap::new(),
        locals: &HashMap::new(),
        tag_params: None,
    };
    let ty = bin.infer_ty(&env);
    assert!(matches!(
        ty,
        Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer)
    ));
}

#[test]
fn runtime_anonymous_arithmetic_uses_structural_intrinsic() {
    let typed = transform_source("two: 1 + 1\n");
    let two_id = body_expr_id(&typed, "two").expect("two body");
    let expr = typed.exprs.get(two_id.as_usize()).expect("two expr");
    assert!(
        matches!(
            expr.kind,
            TypedExprKind::IntrinsicCall {
                op: IntrinsicOp::BitsAdd,
                ..
            }
        ),
        "bind body should now resolve implicit anonymous arithmetic, got {:?}",
        expr.kind
    );
    assert!(
        matches!(expr.ty, Ty::AnonymousInteger { .. } | Ty::Named { .. }),
        "unexpected type: {:?}",
        expr.ty
    );
    if let TypedExprKind::IntrinsicCall { op, args } = &expr.kind {
        assert!(matches!(
            op,
            IntrinsicOp::BitsAdd | IntrinsicOp::BitsSubtract | IntrinsicOp::BitsMultiply
        ));
        assert_eq!(args.len(), 2);
    }
    assert_eq!(*expr.const_value, None);
}

#[test]
fn rebindable_top_level_stays_runtime() {
    let raw = parser::cursor::TokenCursor::parse_source("two: 1 + 1\n");
    let raw_bind = raw.defs.get(&Intern::from_ref("two")).expect("two");
    assert!(matches!(
        &raw_bind.value,
        ast::BindValue::Expr(expr) if expr.const_value.is_none()
    ));
    let prepared = prepare("two: 1 + 1\n");
    let bind = prepared.defs.get(&Intern::from_ref("two")).expect("two");
    assert!(!bind.is_constant(), "`:=` is required for constant bind");
    assert!(bind_expr_const_value_removed(&prepared, "two"));
}

fn bind_expr_const_value_removed(ast: &ast::FileAst, name: &str) -> bool {
    let bind = ast.defs.get(&Intern::from_ref(name)).expect("bind");
    match &bind.value {
        ast::BindValue::Expr(expr) => expr.const_value.is_none(),
        _ => false,
    }
}

#[test]
fn runtime_binary_with_variables_stays_dynamic() {
    let typed = transform_source(
        "#default(IntegerLiteral)
Int is in 0...255
#operator(Add)
add_int(a Int, b Int) Int: a
x: 10\ny: 20\nmain: x + y",
    );
    let main_id = body_expr_id(&typed, "main").expect("main");
    let expr = typed.exprs.get(main_id.as_usize()).expect("main expr");
    assert!(matches!(
        expr.kind,
        TypedExprKind::FnCall { .. } | TypedExprKind::IntrinsicCall { .. }
    ));
    assert!(
        typed
            .type_registry
            .integer_width_for_type(expr.ty)
            .is_some(),
        "unexpected type: {:?}",
        expr.ty
    );
    assert!(matches!(
        expr.ty,
        Ty::AnonymousInteger { .. } | Ty::Named { .. } if expr.ty.type_name() == Some(&Intern::from_ref("Int")) || typed.type_registry.integer_width_for_type(expr.ty).is_some()
    ));
    assert!(expr.const_value.is_none());
    assert!(matches!(
        typecheck::representation::Repr::derive(expr.ty, Some(&typed.type_registry)),
        Ok(typecheck::representation::Repr::Bits { .. })
    ));
}
