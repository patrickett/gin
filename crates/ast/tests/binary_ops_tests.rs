//! Binary arithmetic: parsing, `ConstValue::eval_binop`, compile-time `:=` folding, and lowering.

use ast::prelude::*;
use ast::{ConstValue, HashFloat};
use flask::CompileTarget;
use internment::Intern;
use std::collections::HashMap;
use typecheck::TypedExprKind;
use typecheck::analysis::{CompTimeEvaluator, ConstEnv, TyInfer, TyInferEnv};
use typecheck::intrinsic::IntrinsicOp;
use typecheck::prepare_parse_ast;
use typecheck::ty::Ty;

mod support;
use support::{body_expr_id, transform_source};

fn prepare(source: &str) -> ast::FileAst {
    let mut ast = parser::cursor::TokenCursor::parse_source(source);
    let _ = prepare_parse_ast(&mut ast, &CompileTarget::Library);
    ast
}

fn typed_bind_const_value<'a>(
    typed: &'a typecheck::TypedFileAst,
    name: &str,
) -> Option<&'a ConstValue> {
    let id = body_expr_id(typed, name)?;
    typed
        .exprs
        .get(id.as_usize())?
        .facts
        .const_facts
        .const_value
        .as_ref()
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
fn eval_binop_add_concatenates_compile_time_strings() {
    assert_eq!(
        ConstValue::Text("gin-asm ".into())
            .eval_binop(&ast::BinOp::Add, &ConstValue::Text("1".into())),
        Some(ConstValue::Text("gin-asm 1".into()))
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
fn eval_binop_equality_returns_proposition_status() {
    assert_eq!(
        ConstValue::Int(7.into()).eval_binop(&BinOp::Equal, &ConstValue::Int(7.into())),
        Some(ConstValue::Proposition(true))
    );
    assert_eq!(
        ConstValue::Int(7.into()).eval_binop(&BinOp::Equal, &ConstValue::Int(8.into())),
        Some(ConstValue::Proposition(false))
    );
}

#[test]
fn anonymous_integer_equality_has_a_proposition_type_and_folds_constants() {
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
        ast::Ty::Proposition(_)
    ));
    assert!(
        matches!(
            typed.exprs.facts.const_facts.const_value[expression.as_usize()],
            Some(ConstValue::Proposition(true))
        ),
        "{:?}",
        typed.exprs.facts.const_facts.const_value[expression.as_usize()]
    );
}

#[test]
fn equality_compares_distinct_nominal_numeric_types() {
    let typed = transform_source(
        "Left is in 0 to 255\nLeft has release(self): return\nRight is in -128 to 127\nRight has release(self): return\nsame(a ref Left, b ref Right): return a = b\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let expression = body_expr_id(&typed, "same").expect("same body");
    assert!(matches!(
        typed.exprs.ty[expression.as_usize()],
        Ty::Proposition(_)
    ));
    assert!(matches!(
        typed.exprs.kind[expression.as_usize()],
        TypedExprKind::IntrinsicCall {
            op: IntrinsicOp::BitsCompare {
                interpretation: typecheck::intrinsic::IntegerComparison::Signed,
                predicate: typecheck::intrinsic::ComparisonPredicate::Equal,
            },
            ..
        }
    ));
}

#[test]
fn equality_rejects_pointer_values() {
    let typed = transform_source(
        "Int is in 0 to 255\nInt has release(self): return\nsame(a @Int, b @Int): return a = b\n",
    );

    assert_eq!(
        typed
            .all_flaws()
            .iter()
            .map(|(_, flaw)| flaw.code.slug())
            .collect::<Vec<_>>(),
        ["type-numeric-predicate-operands"]
    );
}

#[test]
fn compile_time_anonymous_arithmetic_preserves_folded_values_after_flow_analysis() {
    for (expression, expected) in [
        ("1 + 2", 3),
        ("7 - 3", 4),
        ("3 * 4", 12),
        ("(1 + 2) * 2", 6),
        ("(1 + 2) * 4", 12),
        ("4 * (1 + 2)", 12),
        ("(1 + 2) + 4", 7),
        ("4 + (1 + 2)", 7),
        ("(1 + 2) - 4", -1),
        ("(1 + 2) + 18446744073709551615", 18446744073709551618),
    ] {
        let typed = transform_source(&format!("answer := {expression}\n"));
        assert!(
            typed.all_flaws().is_empty(),
            "{expression}: {:?}",
            typed.all_flaws()
        );
        assert_const_int(typed_bind_const_value(&typed, "answer"), expected);
    }
}

#[test]
fn inferred_arithmetic_does_not_relax_explicit_or_nominal_bounds() {
    for (source, expected) in [
        (
            "Small is in 0 to 3\nanswer is in Small := (1 + 2) + 4\n",
            "type-integer-literal-out-of-range",
        ),
        (
            "Small is in 0 to 3\nanswer is in Small := (1 + 2) * 2\n",
            "type-integer-narrowing-failed",
        ),
        (
            "Small is in 0 to 3\nleft Small := 3\nanswer := left + 4\n",
            "type-integer-literal-out-of-range",
        ),
        (
            "Small is in 0 to 3\nright Small := 3\nanswer := 4 + right\n",
            "type-integer-literal-out-of-range",
        ),
    ] {
        let typed = transform_source(source);
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.code.slug() == expected),
            "{source}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn compile_time_bind_folds_addition() {
    let typed = transform_source(
        r#"Word is in 0 to 255
add_bits(a Word, b Word) Word:
    left is in 0 to 255: a
    right is in 0 to 255: b
    return ((left + right) & 255) as Word
#infix('+')
#inline
word_add(a Word, b Word) Word: return add_bits(a, b)
two() Word:
    a Word := 1
    b Word := 1
    return a + b
"#,
    );
    assert_const_int(typed_bind_const_value(&typed, "two"), 2);
}

#[test]
fn compile_time_bind_folds_nested_addition() {
    let typed = transform_source(
        r#"Word is in 0 to 255
add_bits(a Word, b Word) Word:
    left is in 0 to 255: a
    right is in 0 to 255: b
    return ((left + right) & 255) as Word
#infix('+')
#inline
word_add(a Word, b Word) Word: return add_bits(a, b)
sum() Word:
    a Word := 1
    b Word := 2
    c Word := 3
    return (a + b) + c
"#,
    );
    assert_const_int(typed_bind_const_value(&typed, "sum"), 6);
}

#[test]
fn eval_compile_time_expr_folds_addition() {
    let main = parser::cursor::TokenCursor::parse_source("x: 1 + 2\n");
    let const_binds: ConstEnv = main.defs.keys().map(|name| (name.0, None)).collect();
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
        "identity(value Int) Int :=\n    result := value\n    doubled := result + result\n    return doubled\nanswer := identity(42)\n",
    );
    let const_binds = ast.defs.keys().map(|name| (name.0, None)).collect();
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
        "pick(first Int, second Int) Int := return first\nanswer := pick(1)\n",
    );
    let const_binds = ast.defs.keys().map(|name| (name.0, None)).collect();
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
        "recurse(value Int) Int := return recurse(value)\nanswer := recurse(1)\n",
    );
    let const_binds = ast.defs.keys().map(|name| (name.0, None)).collect();
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
    assert_eq!(*expr.facts.const_facts.const_value, None);
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
        "Int is in 0 to 255\n#infix('+')\nadd_int(a Int, b Int) Int: return a\nx: 10\ny: 20\nmain: x + y",
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
    assert!(expr.facts.const_facts.const_value.is_none());
    assert!(matches!(
        typecheck::representation::Repr::derive(expr.ty, Some(&typed.type_registry)),
        Ok(typecheck::representation::Repr::Bits { .. })
    ));
}
