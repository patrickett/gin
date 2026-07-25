//! Classify binds as comptime-foldable vs runtime (independent of `:=` constant semantics).

use internment::Intern;

use ast::expr::{BindValue, Expr, Loop};
use ast::parameter::ParameterKind;
use ast::type_expr::TypeExpr;
use ast::{Bind, FileAst};

/// How a bind participates in compile-time evaluation / codegen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComptimeClass {
    /// `extern`, unassigned declare, etc.
    NotApplicable,
    /// Emits runtime code (`write`, `main`, …).
    RuntimeFn,
    /// Foldable at prepare; no runtime entry required (`is_copy`, `compute_size`, …).
    ComptimeFn,
    /// Top-level value whose initializer can be constant-folded (`target`, `write_spec`).
    FoldableValue,
}

/// Extension trait adding [`Bind::classify`] — classify a bind as comptime or runtime.
pub trait BindComptimeExt {
    /// Classify this top-level bind for prepare-time folding and validation.
    fn classify(&self, ast: &FileAst) -> ComptimeClass;
}

impl BindComptimeExt for Bind {
    fn classify(&self, _ast: &FileAst) -> ComptimeClass {
        match &self.value {
            BindValue::Extern | BindValue::Unassigned => return ComptimeClass::NotApplicable,
            _ => {}
        }

        if self.name.as_str() == "main" {
            return ComptimeClass::RuntimeFn;
        }

        if self.params.is_some() {
            if self.is_method() {
                // Methods follow their receiver; treat as runtime unless body is pure comptime (rare).
                return if bind_body_is_comptime_safe(self) && params_are_comptime_surfaces(self) {
                    ComptimeClass::ComptimeFn
                } else {
                    ComptimeClass::RuntimeFn
                };
            }
            if !params_are_comptime_surfaces(self) || bind_body_has_runtime_ops(self) {
                return ComptimeClass::RuntimeFn;
            }
            return ComptimeClass::ComptimeFn;
        }

        // Top-level value bind: fold when the initializer is comptime-safe.
        if bind_body_is_comptime_safe(self) {
            ComptimeClass::FoldableValue
        } else {
            ComptimeClass::NotApplicable
        }
    }
}

/// Extension trait adding [`FileAst::apply_comptime_classification`].
pub trait FileAstComptimeExt {
    /// Set [`Bind::is_compile_time`] from [`Bind::classify`] (comptime fn + foldable values only).
    fn apply_comptime_classification(&mut self);
}

impl FileAstComptimeExt for FileAst {
    fn apply_comptime_classification(&mut self) {
        let classes: Vec<(Intern<String>, ComptimeClass)> = self
            .defs
            .iter()
            .map(|(name, bind)| (*name, bind.classify(self)))
            .collect();
        for (name, class) in classes {
            let Some(bind) = self.defs.get_mut(&name) else {
                continue;
            };
            bind.is_compile_time = matches!(
                class,
                ComptimeClass::ComptimeFn | ComptimeClass::FoldableValue
            );
        }
    }
}

fn params_are_comptime_surfaces(bind: &Bind) -> bool {
    let Some(params) = &bind.params else {
        return true;
    };
    params.values().all(|kind| match kind {
        ParameterKind::Tagged(sp)
        | ParameterKind::ValueParam { ty: sp }
        | ParameterKind::Inferred { ty: sp } => {
            sp.value.is_type_surface() && !is_value_type_parameter(&sp.value)
        }
        ParameterKind::Generic => true,
        ParameterKind::Default(_) => true,
    })
}

/// Value-type parameters (`x Int`, `b Bool`, …) are not `Type`-style compile-time surfaces.
fn is_value_type_parameter(e: &TypeExpr) -> bool {
    matches!(
        e,
        TypeExpr::Nominal(name, _) if matches!(
            name.as_str(),
            "Int" | "Bool" | "Float" | "String" | "Char" | "Byte" | "Nothing"
        )
    )
}

fn bind_body_is_comptime_safe(bind: &Bind) -> bool {
    match &bind.value {
        BindValue::Expr(e) => expr_is_comptime_safe(&e.value),
        BindValue::Body { exprs, ret } => {
            exprs.iter().all(|e| expr_is_comptime_safe(&e.value))
                && ret
                    .value
                    .as_ref()
                    .map(|r| expr_is_comptime_safe(&r.value))
                    .unwrap_or(true)
        }
        BindValue::Extern | BindValue::Unassigned => false,
    }
}

fn bind_body_has_runtime_ops(bind: &Bind) -> bool {
    match &bind.value {
        BindValue::Expr(e) => expr_has_runtime_ops(&e.value),
        BindValue::Body { exprs, ret } => {
            exprs.iter().any(|e| expr_has_runtime_ops(&e.value))
                || ret
                    .value
                    .as_ref()
                    .is_some_and(|r| expr_has_runtime_ops(&r.value))
        }
        BindValue::Extern | BindValue::Unassigned => false,
    }
}

fn expr_is_comptime_safe(expr: &Expr) -> bool {
    !expr_has_runtime_ops(expr)
}

fn expr_has_runtime_ops(expr: &Expr) -> bool {
    match expr {
        Expr::Asm(_) => true,
        Expr::FnCall(call) => {
            if call.path.value.root.as_str() == "asm" {
                return true;
            }
            call.args
                .as_ref()
                .is_some_and(|args| args.iter().any(|a| expr_has_runtime_ops(&a.value)))
        }
        Expr::Binary(bin) => {
            expr_has_runtime_ops(&bin.lhs.value) || expr_has_runtime_ops(&bin.rhs.value)
        }
        Expr::Bind(b) => {
            if b.params.is_some() {
                return true;
            }
            match &b.value {
                BindValue::Expr(e) => expr_has_runtime_ops(&e.value),
                BindValue::Body { exprs, ret } => {
                    exprs.iter().any(|e| expr_has_runtime_ops(&e.value))
                        || ret
                            .value
                            .as_ref()
                            .is_some_and(|r| expr_has_runtime_ops(&r.value))
                }
                BindValue::Extern | BindValue::Unassigned => false,
            }
        }
        Expr::When(w) => {
            w.subject
                .as_ref()
                .is_some_and(|s| expr_has_runtime_ops(&s.value))
                || w.arms.iter().any(|arm| match arm {
                    ast::WhenArm::Is {
                        pattern: _, body, ..
                    } => expr_has_runtime_ops(&body.value),
                    ast::WhenArm::Cond {
                        condition, body, ..
                    } => {
                        expr_has_runtime_ops(&condition.value) || expr_has_runtime_ops(&body.value)
                    }
                    ast::WhenArm::Else(body, _) => expr_has_runtime_ops(&body.value),
                })
        }
        Expr::If(ifx) => {
            let cond_rt = expr_has_runtime_ops(&ifx.subject.value);
            cond_rt
                || ifx.body.iter().any(|s| expr_has_runtime_ops(&s.value))
                || ifx
                    .ret
                    .value
                    .as_ref()
                    .is_some_and(|r| expr_has_runtime_ops(&r.value))
        }
        Expr::Loop(l) => loop_has_runtime_ops(l),
        Expr::TupleLit(elems) | Expr::List(elems) => {
            elems.iter().any(|e| expr_has_runtime_ops(&e.value))
        }
        Expr::TupleAlloc { init, size } => {
            expr_has_runtime_ops(&init.value) || expr_has_runtime_ops(&size.value)
        }
        Expr::TupleGet { base, .. } | Expr::RecordGet { base, .. } => {
            expr_has_runtime_ops(&base.value)
        }
        Expr::Destructure { value, .. } => expr_has_runtime_ops(&value.value),
        Expr::RecordSet { base, value, .. } | Expr::TupleSet { base, value, .. } => {
            expr_has_runtime_ops(&base.value) || expr_has_runtime_ops(&value.value)
        }
        Expr::BufGet { buf, index } => {
            expr_has_runtime_ops(&buf.value) || expr_has_runtime_ops(&index.value)
        }
        Expr::FormatString(fs) => fs
            .parts
            .iter()
            .any(|p| matches!(p, ast::FormatPart::Expr(e, _) if expr_has_runtime_ops(&e.value))),
        Expr::Range(r) => {
            expr_has_runtime_ops(&r.start.value) || expr_has_runtime_ops(&r.end.value)
        }
        Expr::Lit(_) | Expr::TagCall(_) | Expr::AnonymousTag(_) => false,
        _ => false,
    }
}

fn loop_has_runtime_ops(l: &Loop) -> bool {
    match l {
        Loop::While(w) => {
            expr_has_runtime_ops(&w.cond.value)
                || w.exprs.iter().any(|e| expr_has_runtime_ops(&e.value))
        }
        Loop::ForIn(f) => {
            expr_has_runtime_ops(&f.iter.value)
                || f.exprs.iter().any(|e| expr_has_runtime_ops(&e.value))
        }
    }
}
