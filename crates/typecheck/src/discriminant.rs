use ast::integer::IntegerDomain;
use ast::{ConstValue, Declare, DeclareValue, SpanId, SpanTable, Ty};
use diagnostic::code::UnionCode;
use diagnostic::{Diagnostic, RelatedSpan};
use std::collections::HashMap;

use crate::TypeRegistry;

pub fn conversion_domain(registry: &TypeRegistry, ty: &Ty) -> Option<IntegerDomain> {
    registry
        .resolved_definition_for_type(ty)
        .union_discriminant_domain()
        .or_else(|| {
            registry
                .integer_validity_for_type(ty)
                .map(|validity| validity.domain().clone())
        })
}

pub fn cast_constant(
    registry: &TypeRegistry,
    source: &Ty,
    target: &Ty,
    value: &ConstValue,
) -> Option<ConstValue> {
    let source = registry.resolved_definition_for_type(source);
    let target_definition = registry.resolved_definition_for_type(target);
    match value {
        ConstValue::Tag { name, args, .. }
            if args.is_empty()
                && source.union_discriminant_domain().is_some()
                && registry.integer_validity_for_type(target).is_some() =>
        {
            let Ty::Union { variants, .. } = source else {
                return None;
            };
            let variant = variants.iter().find(|variant| variant.name == *name)?;
            Some(ConstValue::Int(variant.discriminant?))
        }
        ConstValue::Int(value) if target_definition.union_discriminant_domain().is_some() => {
            let Ty::Union { name, variants, .. } = target_definition else {
                return None;
            };
            let variant = variants
                .iter()
                .find(|variant| variant.discriminant == Some(*value))?;
            Some(ConstValue::Tag {
                name: variant.name,
                qual_path: Some(name.as_str().to_owned()),
                args: Vec::new().into(),
            })
        }
        _ => None,
    }
}

pub fn is_numeric_cast(registry: &TypeRegistry, source: &Ty, target: &Ty) -> bool {
    let source_numbered = registry
        .resolved_definition_for_type(source)
        .union_discriminant_domain()
        .is_some();
    let target_numbered = registry
        .resolved_definition_for_type(target)
        .union_discriminant_domain()
        .is_some();
    source_numbered && registry.integer_validity_for_type(target).is_some()
        || target_numbered && registry.integer_validity_for_type(source).is_some()
}

pub(crate) fn validate_cast(
    typed: &crate::TypedFileAst,
    operand: crate::ExprId,
    target: &Ty,
) -> Vec<Diagnostic> {
    let source = &typed.exprs.ty[operand.index()];
    if source == target {
        return Vec::new();
    }
    if !is_numeric_cast(&typed.type_registry, source, target) {
        return if typed
            .type_registry
            .resolved_definition_for_type(target)
            .union_discriminant_domain()
            .is_some()
        {
            // type-invalid-union-discriminant-conversion
            vec![Diagnostic::new(
                UnionCode::DiscriminantConversionInvalid,
                "a numbered union requires an explicit conversion from an integer with a valid discriminant",
            )]
        } else {
            Vec::new()
        };
    }
    let actual = cast_source_domain(typed, operand, target);
    let expected = conversion_domain(&typed.type_registry, target).unwrap();
    let proof = actual
        .as_ref()
        .map_or(ast::integer::ContainmentProof::Unknown, |actual| {
            actual.prove_containment(&expected)
        });
    proof
        .diagnostic_code()
        .map(|code| {
            Diagnostic::new(
                code,
                "discriminant conversion requires every source value to be valid in the target",
            )
        })
        .into_iter()
        .collect()
}

pub(crate) fn cast_source_domain(
    typed: &crate::TypedFileAst,
    operand: crate::ExprId,
    target: &Ty,
) -> Option<IntegerDomain> {
    let source = &typed.exprs.ty[operand.index()];
    typed
        .exprs
        .const_value_of(operand)
        .and_then(Option::as_ref)
        .and_then(|value| match value {
            ConstValue::Int(value) => IntegerDomain::bounded(*value, *value),
            _ => cast_constant(&typed.type_registry, source, target, value).and_then(|value| {
                match value {
                    ConstValue::Int(value) => IntegerDomain::bounded(value, value),
                    _ => None,
                }
            }),
        })
        .or_else(|| {
            typed
                .exprs
                .integer_knowledge_of(operand)
                .and_then(Option::as_ref)
                .and_then(crate::transform::place::knowledge_domain)
        })
        .or_else(|| conversion_domain(&typed.type_registry, source))
}

pub(crate) fn validate_declaration(
    declare: &Declare,
    definition: &Ty,
    spans: &SpanTable,
) -> Vec<(SpanId, Diagnostic)> {
    let DeclareValue::Union { variants } = &declare.value else {
        return Vec::new();
    };
    if !variants
        .iter()
        .any(|variant| variant.discriminant().is_some())
    {
        return Vec::new();
    }
    let mut flaws = Vec::new();
    if variants
        .iter()
        .any(|variant| variant.discriminant().is_none())
    {
        flaws.push((
            declare.name_span,
            // type-incomplete-union-discriminants
            Diagnostic::new(
                UnionCode::DiscriminantsIncomplete,
                "every variant in a numbered union requires an explicit discriminant",
            )
            .at_span_id(declare.name_span, spans),
        ));
    }
    if definition.union_discriminant_domain().is_none()
        && variants
            .iter()
            .all(|variant| variant.discriminant().is_some())
    {
        flaws.push((
            declare.name_span,
            // type-discriminant-requires-unit-union
            Diagnostic::new(
                UnionCode::DiscriminantRequiresUnitVariants,
                "explicit discriminants require a union containing only unit constructors",
            )
            .at_span_id(declare.name_span, spans),
        ));
    }
    flaws.extend(validate_unique_discriminants(variants, spans));
    flaws
}

fn validate_unique_discriminants(
    variants: &[ast::Variant],
    spans: &SpanTable,
) -> Vec<(SpanId, Diagnostic)> {
    let mut flaws = Vec::new();
    let mut assigned = HashMap::new();
    for variant in variants {
        let Some(discriminant) = variant.discriminant() else {
            continue;
        };
        if let Some(previous) = assigned.get(&discriminant.value) {
            // type-duplicate-union-discriminant
            let mut diagnostic = Diagnostic::new(
                UnionCode::DiscriminantDuplicate,
                format!(
                    "discriminant value `{}` assigned more than once",
                    discriminant.value
                ),
            )
            .with_arg("value", discriminant.value.to_string())
            .at_span_id(discriminant.span_id, spans);
            diagnostic.related_mut().push(RelatedSpan {
                span: spans.get(*previous),
                label: "first assigned here".to_owned(),
            });
            flaws.push((discriminant.span_id, diagnostic));
        } else {
            assigned.insert(discriminant.value, discriminant.span_id);
        }
    }
    flaws
}
