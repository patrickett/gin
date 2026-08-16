//! Comptime function inlining at runtime call sites and flow constant propagation on `:`.

use ast::ConstValue;
use flask::CompileTarget;
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::prepare_parse_ast;
use typecheck::transform::{TransformCtx, transform};
use typecheck::typed::CallCapability;
use typecheck::{BindBody, DefId, ExprId, FileId, TypedExprKind};

fn transform_prepared(source: &str) -> typecheck::TypedFileAst {
    let mut file_ast = TokenCursor::parse_source(source);
    let _ = prepare_parse_ast(&mut file_ast, &CompileTarget::Library);
    transform(&file_ast, FileId(0), &TransformCtx::new())
}

fn main_body_exprs(typed: &typecheck::TypedFileAst) -> Vec<ExprId> {
    let main = typed
        .defs
        .get(&DefId(Intern::from_ref("main")))
        .expect("main def");
    match &main.body {
        BindBody::Expr(id) => vec![*id],
        BindBody::Body { exprs, ret } => {
            let mut ids = exprs.clone();
            if let Some(r) = ret {
                ids.push(*r);
            }
            ids
        }
        BindBody::Extern => panic!("main should not be extern"),
    }
}

fn walk_exprs(
    typed: &typecheck::TypedFileAst,
    root: ExprId,
    f: &mut dyn FnMut(ExprId, &TypedExprKind),
) {
    let _ = typecheck::typed::walk_expr_preorder(typed, root, &mut |id| {
        f(id, &typed.exprs.kind[id.as_usize()]);
        std::ops::ControlFlow::Continue(())
    });
}

fn any_fn_call_named(typed: &typecheck::TypedFileAst, roots: &[ExprId], name: &str) -> bool {
    let mut found = false;
    for root in roots {
        walk_exprs(typed, *root, &mut |_, kind| {
            if let TypedExprKind::FnCall { target, .. } = kind
                && target.0.as_str() == name
            {
                found = true;
            }
        });
    }
    found
}

fn literal_string_values_for_name<'a>(
    typed: &'a typecheck::TypedFileAst,
    roots: &[ExprId],
    name: &str,
) -> Vec<&'a ConstValue> {
    let mut out = Vec::new();
    for root in roots {
        walk_exprs(typed, *root, &mut |_, kind| {
            let value_id = match kind {
                TypedExprKind::Bind { name: n, body, .. } if n.as_str() == name => Some(*body),
                TypedExprKind::Reassign { name: n, value, .. } if n.as_str() == name => {
                    Some(*value)
                }
                _ => None,
            };
            if let Some(id) = value_id
                && let Some(cv) = &typed.exprs.const_value[id.as_usize()]
            {
                out.push(cv);
            }
        });
    }
    out
}

#[test]
fn comptime_fn_inlined_in_main_with_type_arg() {
    let typed =
        transform_prepared("size_for(x Type) Int := 64\n\nmain:\n  n: size_for(Int)\n  return n\n");
    let size_for = typed
        .defs
        .get(&DefId(Intern::from_ref("size_for")))
        .expect("size_for def in typed ast");
    assert!(
        size_for.is_constant && size_for.call_capability == CallCapability::StagePolymorphic,
        "size_for must be a stage-polymorphic constant bind in typed defs"
    );
    let roots = main_body_exprs(&typed);
    assert!(
        !any_fn_call_named(&typed, &roots, "size_for"),
        "size_for call should be inlined in main; flaws: {:?}",
        typed.all_flaws()
    );
    let n_values = literal_string_values_for_name(&typed, &roots, "n");
    assert_eq!(n_values.len(), 1);
    assert!(matches!(n_values[0], ConstValue::Int(value) if *value == i256::I256::from(64)));
}

#[test]
fn comptime_fn_call_with_runtime_arg_reports_flaw() {
    let typed = transform_prepared(
        "size_for(x Type) Int := 64\n\npick(x Int) Int:\n  return x\n\nseed() Int:\n  return 1\n\nmain:\n  n: size_for(pick(seed()))\n  return 0\n",
    );
    assert!(
        typed.all_flaws().iter().any(|(_, f)| {
            f.code.slug() == "type-cannot-call-comptime-with-runtime-args"
                && f.arg("fn_name") == Some("size_for")
        }),
        "expected CannotCallComptimeWithRuntimeArgs: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn rebindable_string_literal_updates_const_value() {
    let typed = transform_prepared("main:\n  name: 'John'\n  name: 'Jane'\n  return 0\n");
    let roots = main_body_exprs(&typed);
    let values = literal_string_values_for_name(&typed, &roots, "name");
    assert_eq!(values.len(), 2);
    assert!(matches!(&values[0], ConstValue::String(s) if s == "John"));
    assert!(matches!(&values[1], ConstValue::String(s) if s == "Jane"));
}

#[test]
fn comptime_fn_inlined_returning_negative_int() {
    let typed = transform_prepared("neg_one() Int := -1\n\nmain:\n  n: neg_one()\n  return n\n");
    let roots = main_body_exprs(&typed);
    assert!(
        !any_fn_call_named(&typed, &roots, "neg_one"),
        "neg_one call should be inlined; flaws: {:?}",
        typed.all_flaws()
    );
    let mut inline_values = 0usize;
    for root in roots {
        walk_exprs(&typed, root, &mut |id, kind| {
            if let Some(cv) = &typed.exprs.const_value[id.as_usize()]
                && matches!(cv, ConstValue::Int(value) if *value == (-1).into())
            {
                inline_values += 1;
            }
            let _ = kind;
        });
    }
    assert_eq!(inline_values, 1);
}
