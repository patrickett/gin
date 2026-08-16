//! Compile-time bounds checking for bounded `in` parameters and range values.

use crate::ty::ParamKind;
use crate::ty::Ty;
use crate::typed::{ExprId, TypedFileAst};
use ast::ConstValue;
use diagnostic::Diagnostic;
use itertools::izip;

pub fn check_fn_call_bounds(
    typed: &TypedFileAst,
    target: &crate::typed::DefId,
    args: Option<&[ExprId]>,
    substituted_return: Option<&Ty>,
    flaws: &mut Vec<Diagnostic>,
) {
    let Some(bind) = typed.defs.get(target) else {
        return;
    };
    let Some(arg_ids) = args else {
        return;
    };
    let substitution = substituted_return.and_then(|actual_return| {
        let expected = bind
            .return_type
            .named_instance_stripping_reference_wrappers()?;
        let actual = actual_return.named_instance_stripping_reference_wrappers()?;
        (expected.declaration == actual.declaration).then(|| {
            let expected = expected
                .arguments
                .iter()
                .map(|(_, argument)| argument.clone())
                .collect::<Vec<_>>();
            let actual = actual
                .arguments
                .iter()
                .map(|(_, argument)| argument.clone())
                .collect::<Vec<_>>();
            crate::analysis::unify_type_args_with_registry(
                &expected,
                &actual,
                Some(&typed.type_registry),
            )
            .ok()
        })?
    });
    for ((_, declared_param_ty), param_kind, arg_id) in
        izip!(&bind.params, &bind.param_kinds, arg_ids)
    {
        if !matches!(param_kind, ParamKind::Value(_)) {
            continue;
        }
        let param_ty = substitution.as_ref().map_or_else(
            || declared_param_ty.clone(),
            |subst| subst.apply_to_ty(declared_param_ty),
        );
        let Some(arg_ty) = typed.exprs.ty.get(arg_id.as_usize()) else {
            continue;
        };
        if matches!(
            typed.exprs.kind.get(arg_id.as_usize()),
            Some(crate::typed::TypedExprKind::TargetQuery {
                kind: ast::TargetQueryKind::Alignment,
                ..
            })
        ) && typed.target_layout.is_some_and(|layout| {
            typed
                .type_registry
                .integer_validity_for_type(&param_ty)
                .is_some_and(|validity| {
                    let domain = validity.domain();
                    domain.contains(1.into()) && domain.contains(layout.max_alignment().into())
                })
        }) {
            continue;
        }
        check_value_against_param_ty(
            arg_ty,
            typed.exprs.const_value[arg_id.as_usize()].as_ref(),
            &param_ty,
            &typed.type_registry,
            flaws,
        );
    }
}

pub fn check_value_against_param_ty(
    arg_ty: &Ty,
    value: Option<&ConstValue>,
    param_ty: &Ty,
    registry: &crate::TypeRegistry,
    flaws: &mut Vec<Diagnostic>,
) {
    if registry.integer_validity_for_type(param_ty).is_some() {
        check_bounded_scalar_arg(arg_ty, value, param_ty, registry, flaws);
        return;
    }
    if param_ty.is_range_value_record() {
        check_range_value_arg(value, param_ty, flaws);
    }
}

fn check_bounded_scalar_arg(
    arg_ty: &Ty,
    value: Option<&ConstValue>,
    param_ty: &Ty,
    registry: &crate::TypeRegistry,
    flaws: &mut Vec<Diagnostic>,
) {
    if let Some(ConstValue::Int(val)) = value {
        let Some(validity) = registry.integer_validity_for_type(param_ty) else {
            return;
        };
        if !validity.domain().contains(*val) {
            let Some(hull) = validity.domain().storage_hull() else {
                return;
            };
            flaws.push(
                Diagnostic::new(
                    "type-out-of-range",
                    format!(
                        "value {} is out of range (min: {}, max: {})",
                        val,
                        hull.min(),
                        hull.max()
                    ),
                )
                .with_arg("value", format!("{}", val))
                .with_arg("min", hull.min().to_string())
                .with_arg("max", hull.max().to_string()),
            );
        }
        return;
    }
    if arg_ty == param_ty {
        return;
    }
    if let (Some(actual), Some(expected)) = (
        registry.integer_validity_for_type(arg_ty),
        registry.integer_validity_for_type(param_ty),
    ) {
        let assignable = actual.domain().storage_hull().is_some_and(|actual| {
            expected.domain().contains(actual.min()) && expected.domain().contains(actual.max())
        });
        let nominally_compatible = match (arg_ty.named_instance(), param_ty.named_instance()) {
            (Some(actual), Some(expected)) => {
                actual.declaration == expected.declaration
                    && (actual.arguments == expected.arguments
                        || expected.arguments.iter().any(|(_, argument)| {
                            matches!(argument, ast::TyArg::Type(ty) if matches!(ty.as_ref(), Ty::Opaque(_)))
                        })
                        || expected.arguments.iter().zip(&actual.arguments).all(
                            |((_, expected), (_, actual))| {
                                matches!(
                                    (expected, actual),
                                    (ast::TyArg::Type(expected), ast::TyArg::Type(actual))
                                        if matches!(expected.as_ref(), Ty::AnonymousInteger { .. })
                                            && !actual.is_int()
                                )
                            },
                        ))
            }
            (None, None) => true,
            _ => false,
        };
        if !assignable || !nominally_compatible {
            push_bounds_mismatch(flaws, arg_ty, param_ty);
        }
        return;
    }
    if arg_ty.is_int() {
        flaws.push(Diagnostic::new("type-mismatch", "type mismatch"));
    }
}

fn check_range_value_arg(value: Option<&ConstValue>, param_ty: &Ty, flaws: &mut Vec<Diagnostic>) {
    let Some(expected) = param_ty.scalar_bounds_from_range_value() else {
        return;
    };
    if let Some(ConstValue::Record { fields }) = value {
        let start_v = fields
            .iter()
            .find(|(n, _)| n.as_str() == "start")
            .and_then(|(_, value)| value.as_const_size_int());
        let end_v = fields
            .iter()
            .find(|(n, _)| n.as_str() == "end")
            .and_then(|(_, value)| value.as_const_size_int());
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
