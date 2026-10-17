use ast::ty::{Ty, UnionVariant, UnionVariantKind};
use ast::{Pattern, Spanned, SubSpan};
use internment::Intern;

use crate::typed::{
    ExprId, TagId, TypedExprKind, TypedFileAst, TypedWhenArm, TypedWhenExpr, VariantId,
};

pub(super) fn materialize_inferred_union(typed: &mut TypedFileAst, arms: &mut [TypedWhenArm]) {
    let types = branch_types(typed, arms);
    if types.len() < 2 {
        return;
    }
    let integer_types = types
        .iter()
        .all(|ty| typed.type_registry.integer_validity_for_type(ty).is_some());
    let named_types = types
        .iter()
        .filter(|ty| ty.anonymous_integer_validity().is_none())
        .count();
    if integer_types && named_types <= 1 {
        return;
    }
    let union = inferred_union(types);
    for arm in arms {
        let body = match arm {
            TypedWhenArm::Cond { body, .. }
            | TypedWhenArm::Is { body, .. }
            | TypedWhenArm::Else(body, _) => body,
        };
        if !crate::analysis::expression_returns(typed.resolved_program(), *body) {
            *body = materialize_member(typed, *body, &union);
        }
    }
}

fn branch_types(typed: &TypedFileAst, arms: &[TypedWhenArm]) -> Vec<Ty> {
    let mut types = Vec::with_capacity(arms.len());
    for arm in arms {
        let body = match arm {
            TypedWhenArm::Cond { body, .. }
            | TypedWhenArm::Is { body, .. }
            | TypedWhenArm::Else(body, _) => *body,
        };
        if crate::analysis::expression_returns(typed.resolved_program(), body) {
            continue;
        }
        let mut pending = vec![&typed.exprs.ty[body.index()]];
        while let Some(ty) = pending.pop() {
            if let Some(variants) = direct_union_variants(ty) {
                pending.extend(
                    variants
                        .iter()
                        .rev()
                        .map(|variant| variant.fields[0].1.as_ref()),
                );
            } else if !types.contains(ty) {
                types.push(ty.clone());
            }
        }
    }
    types
}

fn direct_union_variants(ty: &Ty) -> Option<&[UnionVariant]> {
    let Ty::Union {
        variants,
        literal_values: None,
        ..
    } = ty
    else {
        return None;
    };
    variants
        .iter()
        .all(|variant| variant.kind == UnionVariantKind::Type && variant.fields.len() == 1)
        .then_some(variants)
}

fn materialize_member(typed: &mut TypedFileAst, value: ExprId, union: &Ty) -> ExprId {
    let actual = &typed.exprs.ty[value.index()];
    if actual == union {
        return value;
    }
    if let Some(variants) = direct_union_variants(actual) {
        let variants = variants.to_vec();
        return materialize_union(typed, value, &variants, union);
    }
    inject_member(typed, value, union)
}

fn materialize_union(
    typed: &mut TypedFileAst,
    value: ExprId,
    variants: &[UnionVariant],
    union: &Ty,
) -> ExprId {
    let span = typed.exprs.span[value.index()];
    let mut arms = Vec::with_capacity(variants.len());
    for (index, variant) in variants.iter().enumerate() {
        let payload = super::lower_exprs::append_derived_expr(
            typed,
            value,
            TypedExprKind::UnionPayload {
                expr: value,
                variant: index,
            },
            *variant.fields[0].1.clone(),
        );
        let body = inject_member(typed, payload, union);
        arms.push(TypedWhenArm::Is {
            pattern: Box::new(Spanned {
                value: Pattern::Nominal(variant.name, span),
                span_id: span,
            }),
            body,
            owned_bindings: Vec::new(),
            arm_span: SubSpan::new(span),
        });
    }
    super::lower_exprs::append_derived_expr(
        typed,
        value,
        TypedExprKind::When(TypedWhenExpr {
            subject: Some(value),
            arms,
            place_joins: Vec::new(),
            body_span: SubSpan::new(span),
        }),
        union.clone(),
    )
}

fn inject_member(typed: &mut TypedFileAst, value: ExprId, union: &Ty) -> ExprId {
    let Ty::Union { name, variants, .. } = union else {
        unreachable!();
    };
    let actual = &typed.exprs.ty[value.index()];
    let discriminant = variants
        .iter()
        .position(|variant| variant.fields[0].1.as_ref() == actual)
        .expect("every branch type belongs to the inferred union");
    super::lower_exprs::append_derived_expr(
        typed,
        value,
        TypedExprKind::TagCall {
            variant_id: VariantId {
                union: TagId(*name),
                name: variants[discriminant].name,
            },
            discriminant,
            args: Some(vec![value]),
            field_names: vec![Intern::from_ref("value")],
        },
        union.clone(),
    )
}

fn inferred_union(types: Vec<Ty>) -> Ty {
    assert!(types.len() >= 2);
    let variants: Vec<_> = types
        .into_iter()
        .map(|ty| {
            let name = ty
                .type_name()
                .copied()
                .unwrap_or_else(|| Intern::new(ty.format_for_hover()));
            let mut variant =
                UnionVariant::new(name, vec![(Intern::from_ref("value"), Box::new(ty))]);
            variant.kind = UnionVariantKind::Type;
            variant
        })
        .collect();
    union_type(variants)
}

pub(crate) fn union_type(variants: Vec<UnionVariant>) -> Ty {
    Ty::Union {
        name: Intern::new(format!(
            "({})",
            variants
                .iter()
                .map(|variant| variant.name.as_str())
                .collect::<Vec<_>>()
                .join(" or ")
        )),
        variants,
        literal_values: None,
        resolved_params: None,
    }
}
