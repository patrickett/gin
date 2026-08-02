//! Intrinsic folding — walk `Typed<Expr>` trees to propagate `const_value`
//! and simplify compile-time-resolvable constructs.
//!
//! This runs after default materialization and entry-target merge. It is
//! orchestrated by [`inject_compiler_intrinsics`].

use std::ops::ControlFlow;

use crate::analysis::pattern::find_matching_when_body;
use ast::ConstValue;
use ast::declare::DeclareValue;
use ast::expr::{Expr, Typed};
use ast::folder::Folder;
use ast::{BindValue, FileAst, Spanned, WhenArm};

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
                value: simplified.value,
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

/// Recursively descend through a `Typed<Expr>` tree to propagate record-field
/// constants and simplify compile-time `when` expressions.
fn inject_typed_expr_intrinsics(expr: &mut Typed<Expr>) {
    propagate_record_field_on_expr(expr);

    if let Expr::When(when) = &mut expr.value {
        if let Some(subject) = &mut when.subject {
            inject_typed_expr_intrinsics(subject);
        }

        let should_simplify = when
            .subject
            .as_ref()
            .and_then(|subject| subject.const_value.as_ref())
            .map(|value| (when.arms.clone(), value.clone()));

        if let Some((arms, value)) = should_simplify
            && let Some(body) = find_matching_when_body(&arms, &value)
        {
            let mut simplified = (*body).clone();
            inject_typed_expr_intrinsics(&mut simplified);
            *expr = simplified;
            return;
        }

        for arm in &mut when.arms {
            match arm {
                WhenArm::Cond { condition, body, .. } => {
                    inject_typed_expr_intrinsics(condition);
                    inject_typed_expr_intrinsics(body);
                }
                WhenArm::Is { body, .. } | WhenArm::Else(body, _) => {
                    inject_typed_expr_intrinsics(body);
                }
            }
        }
        return;
    }

    let _ = ast::folder::walk_typed_expr_children_mut(&mut expr.value, &mut |child| {
        inject_typed_expr_intrinsics(child);
        ControlFlow::Continue(())
    });
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
