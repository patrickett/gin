//! Tests that characterize staged-expression behavior in resolved calls and literals.

use ast::{ConstValue, NormalExpr};
use flask::CompileTarget;
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{BindBody, DefId, ExprId, FileId, TypedExprKind, TypedFileAst};

mod support;
use support::transform_source;

fn transform_prepared(source: &str) -> TypedFileAst {
    let mut file_ast = TokenCursor::parse_source(source);
    let _ = typecheck::prepare_parse_ast(&mut file_ast, &CompileTarget::Library);
    transform(&file_ast, FileId(0), &TransformCtx::new())
}

fn body_expr_id(typed: &TypedFileAst, def_name: &str) -> Option<ExprId> {
    let def_id = DefId(Intern::new(def_name.to_string()));
    let bind = typed.defs.get(&def_id).expect("def exists");
    match &bind.body {
        BindBody::Expr(eid) => Some(*eid),
        BindBody::Body { exprs, ret } => {
            if let Some(ret_id) = ret {
                Some(*ret_id)
            } else {
                exprs.last().copied()
            }
        }
        BindBody::Extern => None,
    }
}

fn has_flaw_with_slug(typed: &TypedFileAst, slug: &str) -> bool {
    typed.all_flaws().iter().any(|(_, f)| f.code.slug() == slug)
}

#[test]
fn type_argument_is_not_lowered_as_runtime_arg() {
    let typed = transform_source("id_ty(x Type) Int: 1\nmain: id_ty(Int)");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::FnCall { target, args, .. } => {
            assert_eq!(target.0.as_str(), "id_ty");
            assert_eq!(args.as_ref().map(Vec::len), Some(0));
        }
        other => panic!("Expected FnCall, got {other:?}"),
    }
}

#[test]
fn type_argument_substituted_return_type_is_available() {
    let typed = transform_source("size_of(x Type) Int: 64\nmain: size_of(Int)");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::FnCall {
            target,
            args,
            substituted_ty,
            ..
        } => {
            assert_eq!(target.0.as_str(), "size_of");
            assert_eq!(args.as_ref().map(Vec::len), Some(0));
            assert!(substituted_ty.is_some());
        }
        other => panic!("Expected FnCall, got {other:?}"),
    }
}

#[test]
fn literal_arithmetic_expr_is_typed_as_binary() {
    let typed = transform_source("main: 1 + 2");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");
    assert!(
        matches!(expr.kind, TypedExprKind::IntrinsicCall { .. }),
        "expected intrinsic call, got {:?}",
        expr.kind
    );
}

#[test]
fn mixed_type_and_value_call_preserves_runtime_arg_positions() {
    let typed = transform_source("id(x Type, y Int) Int: y\nmain: id(Int, 3)");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::FnCall {
            args,
            substituted_ty,
            ..
        } => {
            assert_eq!(args.as_ref().map(Vec::len), Some(1), "{expr:?}");
            assert!(substituted_ty.is_some());
        }
        other => panic!("Expected FnCall, got {other:?}"),
    }
}

#[test]
fn named_type_and_value_call_preserves_runtime_arg_positions() {
    let typed = transform_source("id(x Type, y Int) Int: y\nmain: id(y: 3, x: Int)");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::FnCall {
            args,
            substituted_ty,
            ..
        } => {
            assert_eq!(args.as_ref().map(Vec::len), Some(1), "{expr:?}");
            assert!(substituted_ty.is_some());
        }
        other => panic!("Expected FnCall, got {other:?}"),
    }
}

#[test]
fn arithmetic_type_value_argument_is_used_for_substitution() {
    let source = "\
Array(x Type, n Int) has length Int\n\
make(x Type, n Int) Array(x, n): Array(length: n)\n\
main: make(Int, 1 + 2)\n\
";
    let typed = transform_source(source);
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::FnCall {
            target,
            args,
            substituted_ty,
            ..
        } => {
            assert_eq!(target.0.as_str(), "make");
            assert_eq!(args.as_ref().map(Vec::len), Some(1));
            assert!(substituted_ty.is_some());
        }
        other => panic!("Expected FnCall, got {other:?}"),
    }
}

#[test]
fn runtime_when_expression_is_typed_as_when_node() {
    let source = "\
value_or_default(x Int) Int := when x is 0 then 0\n\
                                   else x\n\
";
    let typed = transform_source(source);
    let value_or_default_body =
        body_expr_id(&typed, "value_or_default").expect("value_or_default has body");
    let expr = typed
        .exprs
        .get(value_or_default_body.as_usize())
        .expect("value_or_default body");

    assert!(matches!(expr.kind, TypedExprKind::When(_)));
}

#[test]
fn type_valued_call_in_main_retains_call_form() {
    let typed = transform_source("size_of(x Type) Int: 64\nmain: size_of(Int)");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::FnCall {
            target,
            args,
            substituted_ty,
            ..
        } => {
            assert_eq!(target.0.as_str(), "size_of");
            assert_eq!(args.as_ref().map(Vec::len), Some(0));
            assert!(substituted_ty.is_some());
        }
        other => panic!("Expected FnCall, got {other:?}"),
    }
}

#[test]
fn pure_literal_arithmetic_expression_reaches_const_value() {
    let typed = transform_prepared("main: 1 + 2");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");
    let TypedExprKind::IntrinsicCall { args, op, .. } = expr.kind else {
        panic!("Expected intrinsic operator, got {:?}", expr.kind)
    };
    assert_eq!(op.name(), "BitsAdd");
    assert_eq!(args.len(), 2);
    assert!(matches!(
        typed.exprs.const_value[args[0].as_usize()],
        Some(ConstValue::Int(value)) if value == 1.into()
    ));
    assert!(matches!(
        typed.exprs.const_value[args[1].as_usize()],
        Some(ConstValue::Int(value)) if value == 2.into()
    ));
}

#[test]
fn generic_value_parameter_substitutes_array_size_in_call_type() {
    let typed = transform_source(
        "sized_array(n Int): Array(Int, n): (0; n)\n\
         main: sized_array(2)",
    );
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::FnCall {
            target,
            args,
            substituted_ty,
            ..
        } => {
            assert_eq!(target.0.as_str(), "sized_array");
            assert_eq!(args.as_ref().map(Vec::len), Some(1));
            assert!(substituted_ty.is_some());
        }
        other => panic!("Expected FnCall, got {other:?}"),
    }
}

#[test]
fn generic_array_return_type_preserves_symbolic_addition_expression() {
    let typed = transform_source("make(n Int): (0; (n + 1))\nmain: make(2)");
    let def = typed
        .defs
        .get(&DefId(Intern::new("make".to_string())))
        .expect("make exists");
    let body = match &def.body {
        BindBody::Expr(id) => typed.exprs.get(id.as_usize()).expect("make body"),
        other => panic!("Expected make body expression, got {other:?}"),
    };
    match &body.kind {
        TypedExprKind::TupleAlloc { size, .. } => {
            assert!(
                !matches!(size, NormalExpr::Value(_)),
                "expected symbolic tuple size expression, got {size:?}"
            );
        }
        other => panic!("Expected TupleAlloc in make body, got {other:?}"),
    }

    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");
    match &expr.kind {
        TypedExprKind::FnCall {
            target,
            args,
            substituted_ty,
            ..
        } => {
            assert_eq!(target.0.as_str(), "make");
            assert_eq!(args.as_ref().map(Vec::len), Some(1));
            assert!(substituted_ty.is_some());
        }
        other => panic!("Expected FnCall, got {other:?}"),
    }
}

#[test]
fn tuple_alloc_arithmetic_size_is_not_forced_to_zero() {
    let typed = transform_prepared("main: (0; 1 + 2)");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::TupleAlloc { size, .. } => {
            assert_ne!(*size, NormalExpr::from(0));
        }
        other => panic!("Expected TupleAlloc, got {other:?}"),
    }
}

#[test]
fn tuple_alloc_subtract_size_is_preserved_as_const_expr() {
    let typed = transform_prepared("main: (0; 6 - 1)");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::TupleAlloc { size, .. } => {
            assert!(matches!(size, NormalExpr::Sub(..)));
        }
        other => panic!("Expected TupleAlloc, got {other:?}"),
    }
}

#[test]
fn tuple_alloc_multiply_size_is_preserved_as_const_expr() {
    let typed = transform_prepared("main: (0; 2 * 3)");
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");

    match &expr.kind {
        TypedExprKind::TupleAlloc { size, .. } => {
            assert!(matches!(size, NormalExpr::Mul(..)));
        }
        other => panic!("Expected TupleAlloc, got {other:?}"),
    }
}

#[test]
fn runtime_argument_to_type_only_call_reports_diag() {
    let typed = transform_prepared(
        "size_of(x Type) Int: 64\n\npick(x Int) Int:\n  return x\n\nmain:\n  return size_of(pick(1))\n",
    );
    assert!(
        has_flaw_with_slug(&typed, "type-cannot-call-comptime-with-runtime-args"),
        "expected type-cannot-call-comptime-with-runtime-args: {:?}",
        typed.all_flaws()
    );
}
