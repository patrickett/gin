//! Comptime function inlining at runtime call sites and flow constant propagation on `:`.

use ast::ConstValue;
use flask::CompileTarget;
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::prepare_file_ast;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{BindBody, DefId, ExprId, FileId, TypedExprKind};

fn transform_prepared(source: &str) -> typecheck::TypedFileAst {
    let mut file_ast = TokenCursor::parse_source(source);
    let _ = prepare_file_ast(&mut file_ast, &CompileTarget::Library);
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
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let idx = id.as_usize();
        if idx >= typed.exprs.kind.len() {
            continue;
        }
        f(id, &typed.exprs.kind[idx]);
        stack.extend(child_expr_ids(&typed.exprs.kind[idx]));
    }
}

fn child_expr_ids(kind: &TypedExprKind) -> Vec<ExprId> {
    match kind {
        TypedExprKind::Lit(_)
        | TypedExprKind::Asm(_)
        | TypedExprKind::SelfRef { .. }
        | TypedExprKind::FormatString(_) => vec![],
        TypedExprKind::Binary { lhs, rhs, .. } => vec![*lhs, *rhs],
        TypedExprKind::FnCall { args, .. } => args.clone().unwrap_or_default(),
        TypedExprKind::TagCall { args, .. } => args.clone().unwrap_or_default(),
        TypedExprKind::Bind { stmts, body, .. } => {
            let mut ids = stmts.clone();
            ids.push(*body);
            ids
        }
        TypedExprKind::Reassign { value, .. } => vec![*value],
        TypedExprKind::When(w) => {
            let mut ids = w.subject.map(|s| vec![s]).unwrap_or_default();
            for arm in &w.arms {
                use typecheck::TypedWhenArm;
                match arm {
                    TypedWhenArm::Cond {
                        condition, body, ..
                    } => {
                        ids.push(*condition);
                        ids.push(*body);
                    }
                    TypedWhenArm::Is { body, .. } => ids.push(*body),
                    TypedWhenArm::Else(body, _) => ids.push(*body),
                }
            }
            ids
        }
        TypedExprKind::If(if_expr) => {
            let mut ids = vec![if_expr.subject];
            ids.extend(if_expr.stmts.clone());
            if let Some(ret) = if_expr.ret {
                ids.push(ret);
            }
            ids
        }
        TypedExprKind::Loop(l) => l.stmts.clone(),
        TypedExprKind::Range { start, end } => vec![*start, *end],
        TypedExprKind::TupleLit(items) | TypedExprKind::List(items) => items.clone(),
        TypedExprKind::Cast { expr, .. } => vec![*expr],
        TypedExprKind::TupleAlloc { init, .. } => vec![*init],
        TypedExprKind::TupleGet { base, .. } => vec![*base],
        TypedExprKind::TupleSet { base, value, .. } => vec![*base, *value],
        TypedExprKind::Destructure { value, .. } => vec![*value],
        TypedExprKind::RecordSet { base, value, .. } => vec![*base, *value],
        TypedExprKind::BufGet { buf, index } => vec![*buf, *index],
        TypedExprKind::BufSet { buf, index, value } => vec![*buf, *index, *value],
        TypedExprKind::TakePtr(e)
        | TypedExprKind::Ref(e)
        | TypedExprKind::ConsumeArg(e)
        | TypedExprKind::Deref(e)
        | TypedExprKind::Negate(e)
        | TypedExprKind::Eat(e) => vec![*e],
    }
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
                TypedExprKind::Reassign { name: n, value } if n.as_str() == name => Some(*value),
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
        size_for.is_compile_time,
        "size_for must be comptime-classified in typed defs"
    );
    let roots = main_body_exprs(&typed);
    assert!(
        !any_fn_call_named(&typed, &roots, "size_for"),
        "size_for call should be inlined in main; flaws: {:?}",
        typed.all_flaws()
    );
    let n_values = literal_string_values_for_name(&typed, &roots, "n");
    assert_eq!(n_values.len(), 1);
    assert!(matches!(n_values[0], ConstValue::Int(64)));
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
