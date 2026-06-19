//! Intrinsic folding — walk `Typed<Expr>` trees to propagate `const_value`
//! and simplify compile-time-resolvable constructs.
//!
//! This runs after default materialization and entry-target merge. It is
//! orchestrated by [`inject_compiler_intrinsics`].

use std::ops::ControlFlow;

use internment::Intern;

use crate::analysis::pattern::find_matching_when_body;
use ast::ConstValue;
use ast::declare::DeclareValue;
use ast::expr::{Expr, FnCall, FormatPart, Typed};
use ast::folder::Folder;
use ast::type_expr::TypeExpr;
use ast::{BindValue, FileAst, Loop as LoopEnum, Spanned, WhenArm};

/// Entry point for the intrinsic folding pass.
///
/// 1. Simplifies `when` expressions in tag declares whose subjects are
///    compile-time constants.
/// 2. Walks all `BindValue` trees via the [`Folder`] trait, injecting
///    `const_value` on intrinsic `FnCall` references.
pub fn inject_compiler_intrinsics(ast: &mut FileAst) {
    inject_when_in_tag_declares(ast);
    let mut folder = IntrinsicFolder;
    let _ = folder.visit_file_ast(ast);
}

/// Propagates record field constants and simplifies compile-time `when`
/// expressions in tag declares.
///
/// Must run after [`crate::prepare_target::materialize_default_binds`] and
/// [`crate::prepare_target::apply_entry_target_merge`].
fn inject_when_in_tag_declares(ast: &mut FileAst) {
    for decl in ast.tags.values_mut() {
        let DeclareValue::When(when) = &mut decl.value else {
            continue;
        };
        if let Some(subject) = &mut when.subject {
            inject_typed_expr_intrinsics(subject);
        }
        let should_simplify = when
            .subject
            .as_ref()
            .and_then(|s| s.const_value.as_ref())
            .map(|cv| (when.arms.clone(), cv.clone()));
        if let Some((arms, cv)) = should_simplify
            && let Some(body) = find_matching_when_body(&arms, &cv)
        {
            let mut simplified = (*body).clone();
            inject_typed_expr_intrinsics(&mut simplified);
            decl.value = DeclareValue::Alias(Box::new(Spanned {
                value: type_expr_from_typed_expr(&simplified),
                span_id: simplified.span_id,
            }));
            continue;
        }
        for arm in &mut when.arms {
            match arm {
                WhenArm::Cond {
                    condition, body, ..
                } => {
                    inject_typed_expr_intrinsics(condition);
                    inject_typed_expr_intrinsics(body);
                }
                WhenArm::Is { body, .. } => inject_typed_expr_intrinsics(body),
                WhenArm::Else(body, _) => inject_typed_expr_intrinsics(body),
            }
        }
    }
}

/// Recover a [`TypeExpr`] from a simplified `Typed<Expr>` body for alias
/// resolution.
fn type_expr_from_typed_expr(expr: &Typed<Expr>) -> TypeExpr {
    match &expr.value {
        Expr::AnonymousTag(name) => TypeExpr::Nominal(*name, expr.span_id),
        Expr::TagCall(ast::TagCall { name, .. }) => TypeExpr::Nominal(*name, expr.span_id),
        Expr::FnCall(call) => TypeExpr::Nominal(call.path.root, call.path.span_id),
        _ => TypeExpr::Nominal(Intern::new(String::new()), expr.span_id),
    }
}

/// Recursively descend through a `Typed<Expr>` tree to propagate record-field
/// constants and simplify compile-time `when` expressions.
fn inject_typed_expr_intrinsics(expr: &mut Typed<Expr>) {
    propagate_record_field_on_expr(expr);

    match &mut expr.value {
        Expr::FnCall(FnCall {
            args: Some(args), ..
        }) => {
            for arg in args.iter_mut() {
                inject_typed_expr_intrinsics(arg);
            }
        }
        Expr::FnCall(FnCall { args: None, .. }) => {}
        Expr::Binary(bin) => {
            inject_typed_expr_intrinsics(&mut bin.lhs);
            inject_typed_expr_intrinsics(&mut bin.rhs);
        }
        Expr::Bind(b) => match &mut b.value {
            BindValue::Expr(e) => inject_typed_expr_intrinsics(e),
            BindValue::Body { exprs, ret } => {
                for e in exprs.iter_mut() {
                    inject_typed_expr_intrinsics(e);
                }
                if let Some(e) = &mut ret.value {
                    inject_typed_expr_intrinsics(e);
                }
            }
            BindValue::Extern | BindValue::Unassigned => {}
        },
        Expr::When(when) => {
            if let Some(subject) = &mut when.subject {
                inject_typed_expr_intrinsics(subject);
            }

            let should_simplify = when
                .subject
                .as_ref()
                .and_then(|s| s.const_value.as_ref())
                .map(|cv| (when.arms.clone(), cv.clone()));

            if let Some((arms, cv)) = should_simplify
                && let Some(body) = find_matching_when_body(&arms, &cv)
            {
                let mut simplified = (*body).clone();
                inject_typed_expr_intrinsics(&mut simplified);
                *expr = simplified;
                return;
            }

            for arm in &mut when.arms {
                match arm {
                    WhenArm::Cond {
                        condition, body, ..
                    } => {
                        inject_typed_expr_intrinsics(condition);
                        inject_typed_expr_intrinsics(body);
                    }
                    WhenArm::Is { body, .. } => {
                        inject_typed_expr_intrinsics(body);
                    }
                    WhenArm::Else(body, _) => {
                        inject_typed_expr_intrinsics(body);
                    }
                }
            }
        }
        Expr::If(ifx) => {
            inject_typed_expr_intrinsics(&mut ifx.subject);
            for e in &mut ifx.body {
                inject_typed_expr_intrinsics(e);
            }
            if let Some(e) = &mut ifx.ret.value {
                inject_typed_expr_intrinsics(e);
            }
        }
        Expr::Loop(loop_enum) => match loop_enum {
            LoopEnum::While(while_loop) => {
                inject_typed_expr_intrinsics(&mut while_loop.cond);
                for e in &mut while_loop.exprs {
                    inject_typed_expr_intrinsics(e);
                }
            }
            LoopEnum::ForIn(for_in) => {
                inject_typed_expr_intrinsics(&mut for_in.pat);
                inject_typed_expr_intrinsics(&mut for_in.iter);
                for e in &mut for_in.exprs {
                    inject_typed_expr_intrinsics(e);
                }
            }
        },
        Expr::TagCall(tc) => {
            for arg in &mut tc.args {
                inject_typed_expr_intrinsics(arg);
            }
        }
        Expr::FormatString(fs) => {
            for part in &mut fs.parts {
                if let FormatPart::Expr(e, _) = part {
                    inject_typed_expr_intrinsics(e);
                }
            }
        }
        Expr::Range(r) => {
            inject_typed_expr_intrinsics(&mut r.start);
            inject_typed_expr_intrinsics(&mut r.end);
        }
        Expr::Asm(a) => {
            if let Some(spec) = &mut a.spec_expr {
                inject_typed_expr_intrinsics(spec);
            }
            for o in &mut a.operand_values {
                inject_typed_expr_intrinsics(o);
            }
        }
        Expr::RecordLit(fields) => {
            for (_, e) in fields.iter_mut() {
                inject_typed_expr_intrinsics(e);
            }
        }
        Expr::TupleLit(elems) | Expr::List(elems) => {
            for e in elems.iter_mut() {
                inject_typed_expr_intrinsics(e);
            }
        }
        Expr::TupleAlloc { init, .. } => inject_typed_expr_intrinsics(init),
        Expr::TupleGet { base, .. } | Expr::RecordGet { base, .. } => {
            inject_typed_expr_intrinsics(base);
        }
        Expr::Destructure { value, .. } => {
            inject_typed_expr_intrinsics(value);
        }
        Expr::TupleSet { base, value, .. } | Expr::RecordSet { base, value, .. } => {
            inject_typed_expr_intrinsics(base);
            inject_typed_expr_intrinsics(value);
        }
        Expr::BufGet { buf, index } => {
            inject_typed_expr_intrinsics(buf);
            inject_typed_expr_intrinsics(index);
        }
        Expr::BufSet {
            buf, index, value, ..
        } => {
            inject_typed_expr_intrinsics(buf);
            inject_typed_expr_intrinsics(index);
            inject_typed_expr_intrinsics(value);
        }
        Expr::Cast { expr: e, .. } => inject_typed_expr_intrinsics(e),
        Expr::TakePtr(e)
        | Expr::Ref { inner: e, .. }
        | Expr::ConsumeArg(e)
        | Expr::Eat(e)
        | Expr::Deref(e)
        | Expr::Negate(e) => inject_typed_expr_intrinsics(e),
        Expr::Lit(_)
        | Expr::SelfRef
        | Expr::AnonymousTag(..)
        | Expr::TypeNominal(..)
        | Expr::TypeInRange(..)
        | Expr::TypeQualified(_)
        | Expr::TypeGeneric { .. }
        | Expr::TypeRef { .. } => {}
    }
}

/// Check if a `Typed<Expr>` base has a `ConstValue::Record` and propagate the
/// matching field's value.
fn record_from_bind_base(base: &Typed<Expr>) -> Option<&ConstValue> {
    if let Some(cv @ ConstValue::Record { .. }) = base.const_value.as_ref() {
        return Some(cv);
    }
    if let Expr::Bind(b) = &base.value
        && b.params.is_none()
        && let BindValue::Expr(inner) = &b.value
    {
        return inner.const_value.as_ref();
    }
    None
}

fn propagate_record_field_on_expr(expr: &mut Typed<Expr>) {
    if let Expr::RecordGet { base, field } = &expr.value
        && let Some(ConstValue::Record { fields }) = record_from_bind_base(base)
        && let Some((_, v)) = fields.iter().find(|(n, _)| n == field)
    {
        expr.const_value = Some(v.clone());
    }
}

/// A [`Folder`] that walks all `BindValue` trees and injects `const_value` on
/// intrinsic `FnCall` references found inside `Typed<Expr>` wrappers.
struct IntrinsicFolder;

impl Folder for IntrinsicFolder {
    fn visit_bind_value(&mut self, val: &mut BindValue) -> ControlFlow<()> {
        match val {
            BindValue::Expr(e) => {
                inject_typed_expr_intrinsics(e);
            }
            BindValue::Body { exprs, ret } => {
                for expr in exprs.iter_mut() {
                    inject_typed_expr_intrinsics(expr);
                }
                if let Some(e) = &mut ret.value {
                    inject_typed_expr_intrinsics(e);
                }
            }
            BindValue::Extern | BindValue::Unassigned => {}
        }
        ControlFlow::Continue(())
    }
}
