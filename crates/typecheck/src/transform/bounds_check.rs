//! Compile-time bounds checking for bounded `in` parameters and range values.

use crate::ty::Ty;
use crate::typed::{ExprId, TypedFileAst};
use diagnostic::Diagnostic;

pub fn check_fn_call_bounds(
    typed: &TypedFileAst,
    target: &crate::typed::DefId,
    args: Option<&[ExprId]>,
    flaws: &mut Vec<Diagnostic>,
) {
    let Some(bind) = typed.defs.get(target) else {
        return;
    };
    let Some(arg_ids) = args else {
        return;
    };
    for ((_, param_ty), arg_id) in bind.params.iter().zip(arg_ids.iter()) {
        let Some(arg_ty) = typed.exprs.ty.get(arg_id.as_usize()) else {
            continue;
        };
        check_value_against_param_ty(arg_ty, param_ty, flaws);
    }
}

pub fn check_value_against_param_ty(arg_ty: &Ty, param_ty: &Ty, flaws: &mut Vec<Diagnostic>) {
    if param_ty.int_bounds().is_some() {
        check_bounded_scalar_arg(arg_ty, param_ty, flaws);
        return;
    }
    if param_ty.is_range_value_record() {
        check_range_value_arg(arg_ty, param_ty, flaws);
    }
}

fn check_bounded_scalar_arg(arg_ty: &Ty, param_ty: &Ty, flaws: &mut Vec<Diagnostic>) {
    if let Some(val) = arg_ty.int_known_value() {
        if let Err(bounds) = param_ty.check_int_in_bounds(val) {
            flaws.push(
                Diagnostic::new(
                    "type-out-of-range",
                    format!(
                        "value {} is out of range (min: {}, max: {})",
                        val, bounds.min, bounds.max
                    ),
                )
                .with_arg("value", format!("{}", val))
                .with_arg("min", format!("{}", bounds.min))
                .with_arg("max", format!("{}", bounds.max)),
            );
        }
        return;
    }
    if arg_ty.int_bounds().is_some() {
        if !arg_ty.int_assignable_to(param_ty) {
            push_bounds_mismatch(flaws, arg_ty, param_ty);
        }
        return;
    }
    if arg_ty.is_int() {
        flaws.push(Diagnostic::new("type-mismatch", "type mismatch"));
    }
}

fn check_range_value_arg(arg_ty: &Ty, param_ty: &Ty, flaws: &mut Vec<Diagnostic>) {
    let Some(expected) = param_ty.scalar_bounds_from_range_value() else {
        return;
    };
    if let Ty::Record { fields, .. } = arg_ty {
        let start_v = fields
            .iter()
            .find(|(n, _)| n.as_str() == "start")
            .and_then(|(_, t)| t.int_known_value());
        let end_v = fields
            .iter()
            .find(|(n, _)| n.as_str() == "end")
            .and_then(|(_, t)| t.int_known_value());
        if let (Some(s), Some(e)) = (start_v, end_v) {
            if s > e {
                flaws.push(
                    Diagnostic::new(
                        "type-out-of-range",
                        format!(
                            "value {} is out of range (min: {}, max: {})",
                            s, expected.min, expected.max
                        ),
                    )
                    .with_arg("value", format!("{}", s))
                    .with_arg("min", format!("{}", expected.min))
                    .with_arg("max", format!("{}", expected.max)),
                );
            }
            for v in [s, e] {
                if !expected.contains(v) {
                    flaws.push(
                        Diagnostic::new(
                            "type-out-of-range",
                            format!(
                                "value {} is out of range (min: {}, max: {})",
                                v, expected.min, expected.max
                            ),
                        )
                        .with_arg("value", format!("{}", v))
                        .with_arg("min", format!("{}", expected.min))
                        .with_arg("max", format!("{}", expected.max)),
                    );
                }
            }
        }
    }
}

fn push_bounds_mismatch(flaws: &mut Vec<Diagnostic>, arg_ty: &Ty, param_ty: &Ty) {
    let _ = (arg_ty, param_ty);
    flaws.push(Diagnostic::new("type-mismatch", "type mismatch"));
}
