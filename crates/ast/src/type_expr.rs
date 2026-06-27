use std::collections::HashMap;
use std::fmt;

use crate::ConstExpr;
use crate::expr::Literal;
use crate::parameter::ParameterKind;
use crate::path::ModPath;
use crate::pattern::PatternNameExt;
use crate::span::{SpanId, Spanned};
use crate::ty::{Ty, VariantMap};
use i256::I256;
use internment::Intern;

/// A type expression — the right-hand side of a type annotation.
///
/// This replaces the old `Expr::TypeNominal`, `Expr::TypeQualified`, and
/// `Expr::TypeGeneric` variants so that type expressions are a distinct AST
/// node from value expressions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeExpr {
    /// Bare `Tag` (e.g. `Str` in `(x Str)`).
    Nominal(Intern<String>, SpanId),
    /// Qualified path `Tag.Tag…`.
    Qualified(Spanned<ModPath>),
    /// `Tag(...)` with generic / named parameters.
    ///
    /// `span` covers the head tag name only (for diagnostics). The enclosing
    /// [`Spanned`] `span_id`, when present, covers the full `Tag(...)` site.
    Generic {
        name: Intern<String>,
        params: Vec<(Intern<String>, ParameterKind)>,
        /// Span of each parameter name (wildcard or binder); parallel to `params`.
        param_spans: Vec<(Intern<String>, SpanId)>,
        span: SpanId,
    },
    /// Literal value in type position (e.g. union variant like `X0 is 'x0'`).
    Literal(Literal, SpanId),
    /// Pointer to another type (e.g. `@x` — raw pointer to `x`).
    Pointer(Box<Spanned<TypeExpr>>),
    /// Reference type: `ref T` or `mut T`.
    Ref {
        inner: Box<Spanned<TypeExpr>>,
        mutable: bool,
    },
    /// The unit type `()`.
    Unit,
    /// Empty list pattern `[]` (compile-time `when` arms).
    ListEmpty,
    /// List cons pattern `[head, ...tail]` (compile-time `when` arms).
    ListCons {
        head: Box<Spanned<TypeExpr>>,
        tail: Box<Spanned<TypeExpr>>,
    },
    /// Tuple pattern `(A, B, ...)` in compile-time `when ... is` arms.
    Tuple(Vec<Spanned<TypeExpr>>),
    /// Scalar in an inclusive range: `x in 0...10` or `week_num in WeekRange`.
    InRange { bounds: InRangeBounds, span: SpanId },
}

/// Right-hand side of `in` in a type position.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InRangeBounds {
    Literal(I256, I256),
    Tag(Intern<String>),
}

impl fmt::Display for InRangeBounds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "in ")?;
        match self {
            InRangeBounds::Literal(min, max) => write!(f, "{min}...{max}"),
            InRangeBounds::Tag(name) => write!(f, "{}", name.as_str()),
        }
    }
}

impl TypeExpr {
    /// Whether this type expression is a "type surface" — a nominal, qualified,
    /// generic, in-range, or reference type (as opposed to a literal value or
    /// structural pattern).
    pub fn is_type_surface(&self) -> bool {
        match self {
            TypeExpr::Nominal(..) | TypeExpr::Qualified(_) | TypeExpr::Generic { .. } => true,
            TypeExpr::InRange { .. } => true,
            TypeExpr::Ref { inner, .. } => inner.value.is_type_surface(),
            _ => false,
        }
    }

    /// A pattern that matches any remaining value (wildcard `_` or generic with wildcard param).
    pub fn is_catch_all_pattern(&self) -> bool {
        match self {
            TypeExpr::Nominal(name, _) if name.as_str() == "_" => true,
            TypeExpr::ListCons { tail, .. } => tail.value.is_catch_all_pattern(),
            TypeExpr::Generic { params, .. } => {
                params.len() == 1
                    && params[0].0.as_str() == "_"
                    && matches!(params[0].1, ParameterKind::Generic)
            }
            _ => false,
        }
    }

    /// Collect every type-parameter name introduced by a method's receiver type
    /// surface, e.g. `x` in `Range[x].new(...)`. The returned substitution maps
    /// each name to `Ty::Opaque(name)` (the identity binding used while
    /// typechecking the method itself).
    ///
    /// Returns an empty map for non-`Generic` receivers (`Bool.to_string`
    /// introduces no type variables).
    pub fn typevars_from_receiver(&self) -> HashMap<Intern<String>, Ty> {
        let mut out = HashMap::new();
        if let TypeExpr::Generic { params, .. } = self {
            for (name, kind) in params {
                if matches!(kind, ParameterKind::Generic) {
                    out.insert(*name, Ty::Opaque(*name));
                }
            }
        }
        out
    }

    /// Source span of a variant name inside a union variant shape (`False` in `Bool is True or False`).
    pub fn variant_shape_name_span(&self, variant_name: &str) -> Option<SpanId> {
        match self {
            TypeExpr::Nominal(n, span) if n.as_str() == variant_name => Some(*span),
            TypeExpr::Generic { name, span, .. } if name.as_str() == variant_name => Some(*span),
            TypeExpr::Literal(Literal::String(s), span) if s == variant_name => Some(*span),
            _ => None,
        }
    }

    /// Types introduced by matching this pattern against a value of `subject_ty`.
    pub fn pattern_binding_types(
        &self,
        subject_ty: Option<&Ty>,
        variant_map: &VariantMap,
        tag_types: &HashMap<Intern<String>, Ty>,
    ) -> HashMap<Intern<String>, Ty> {
        let mut out = HashMap::new();
        match self {
            TypeExpr::Generic {
                params,
                param_spans,
                ..
            } => {
                let variant_name = Intern::<String>::from_ref(self.surface_mangle_name());
                if let Some(payload_fields) =
                    payload_fields_for_variant(variant_name, subject_ty, variant_map)
                {
                    for (slot, (param_name, _span)) in param_spans.iter().enumerate() {
                        let ty = if let Some((_, t)) = payload_fields.get(slot) {
                            t.clone()
                        } else if let Some((_, kind)) = params.iter().find(|(n, _)| n == param_name)
                        {
                            match kind {
                                ParameterKind::Tagged(sp) => sp.value.resolve_type(tag_types),
                                ParameterKind::Generic => Ty::Opaque(*param_name),
                                ParameterKind::Default(_) => continue,
                            }
                        } else {
                            continue;
                        };
                        if !param_name.is_pattern_wildcard() {
                            out.insert(*param_name, ty);
                        }
                    }
                } else {
                    for (param_name, kind) in params {
                        if param_name.is_pattern_wildcard() {
                            continue;
                        }
                        if let ParameterKind::Tagged(sp) = kind {
                            out.insert(*param_name, sp.value.resolve_type(tag_types));
                        } else {
                            out.insert(*param_name, Ty::Opaque(*param_name));
                        }
                    }
                }
            }
            TypeExpr::Nominal(name, _)
                if name
                    .as_str()
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase())
                    && !name.is_pattern_wildcard() =>
            {
                if let Some(subject_ty) = subject_ty {
                    out.insert(*name, subject_ty.clone());
                } else {
                    out.insert(*name, Ty::Opaque(*name));
                }
            }
            TypeExpr::ListCons { head, tail } => {
                if let Some(elem_ty) = subject_ty.and_then(|ty| ty.list_elem_ty(tag_types)) {
                    for (name, ty) in
                        head.value
                            .pattern_binding_types(Some(&elem_ty), variant_map, tag_types)
                    {
                        out.insert(name, ty);
                    }
                    let tail_ty = match subject_ty {
                        Some(Ty::Array { elem, .. }) => Ty::Array {
                            elem: elem.clone(),
                            size: ConstExpr::from(0),
                        },
                        Some(ty) => ty.clone(),
                        None => Ty::Opaque(Intern::new("list_tail".to_string())),
                    };
                    for (name, ty) in
                        tail.value
                            .pattern_binding_types(Some(&tail_ty), variant_map, tag_types)
                    {
                        out.insert(name, ty);
                    }
                }
            }
            TypeExpr::Tuple(elems) => {
                if let Some(Ty::Tuple(subject_elems)) = subject_ty {
                    for (pat, subj) in elems.iter().zip(subject_elems.iter()) {
                        for (name, ty) in
                            pat.value
                                .pattern_binding_types(Some(subj), variant_map, tag_types)
                        {
                            out.insert(name, ty);
                        }
                    }
                }
            }
            _ => {}
        }
        out
    }

    /// Type at parameter slot `slot` when matching this pattern against `subject_ty`.
    pub fn pattern_param_type_at_slot(
        &self,
        slot: usize,
        subject_ty: Option<&Ty>,
        variant_map: &VariantMap,
        tag_types: &HashMap<Intern<String>, Ty>,
    ) -> Option<Ty> {
        match self {
            TypeExpr::Generic {
                params,
                param_spans,
                ..
            } => {
                let (param_name, _) = param_spans.get(slot)?;
                let variant_name = Intern::<String>::from_ref(self.surface_mangle_name());
                if let Some(payload_fields) =
                    payload_fields_for_variant(variant_name, subject_ty, variant_map)
                    && let Some((_, ty)) = payload_fields.get(slot)
                {
                    return Some(ty.clone());
                }
                let kind = params.iter().find(|(n, _)| n == param_name).map(|(_, k)| k);
                match kind {
                    Some(ParameterKind::Tagged(sp)) => Some(sp.value.resolve_type(tag_types)),
                    Some(ParameterKind::Generic) if !param_name.is_pattern_wildcard() => {
                        Some(Ty::Opaque(*param_name))
                    }
                    _ => None,
                }
            }
            TypeExpr::ListCons { head, tail } => {
                let elem_ty = subject_ty.and_then(|ty| ty.list_elem_ty(tag_types))?;
                if slot == 0 {
                    head.value
                        .pattern_param_type_at_slot(0, Some(&elem_ty), variant_map, tag_types)
                } else {
                    let tail_ty = match subject_ty {
                        Some(Ty::Array { elem, .. }) => Ty::Array {
                            elem: elem.clone(),
                            size: ConstExpr::from(0),
                        },
                        _ => tag_types
                            .get(&Intern::from_ref("List"))
                            .cloned()
                            .unwrap_or(Ty::Opaque(Intern::new("list_tail".to_string()))),
                    };
                    tail.value.pattern_param_type_at_slot(
                        slot.saturating_sub(1),
                        Some(&tail_ty),
                        variant_map,
                        tag_types,
                    )
                }
            }
            _ => None,
        }
    }

    /// Resolve this type expression to its [`Ty`] via simple name lookup.
    ///
    /// This is a simplified version kept in `ast` for `pub(crate)` callers.
    /// The full resolution (with generics/substitution) lives in
    /// `typecheck::analysis::type_surface::TypeEnv`.
    fn resolve_type(&self, tag_types: &HashMap<Intern<String>, Ty>) -> Ty {
        match self {
            TypeExpr::Nominal(name, _) => tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name)),
            TypeExpr::Qualified(mp) => {
                if let Some(last) = mp.segments.last() {
                    tag_types.get(last).cloned().unwrap_or(Ty::Opaque(*last))
                } else {
                    tag_types
                        .get(&mp.root)
                        .cloned()
                        .unwrap_or(Ty::Opaque(mp.root))
                }
            }
            TypeExpr::Ref { inner, .. } => {
                let inner_ty = inner.value.resolve_type(tag_types);
                Ty::Ref {
                    inner: Box::new(inner_ty),
                    mutable: false,
                }
            }
            TypeExpr::Generic { name, .. } => {
                tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name))
            }
            _ => Ty::Opaque(Intern::from_ref("<expr>")),
        }
    }
}

/// Payload fields for a variant, looking up the variant in the variant map
/// and matching against the subject type's union name.
fn payload_fields_for_variant(
    variant_name: Intern<String>,
    subject_ty: Option<&Ty>,
    variant_map: &VariantMap,
) -> Option<Vec<(Intern<String>, Ty)>> {
    let entries = variant_map.get(&variant_name)?;
    let subject_union = match subject_ty {
        Some(Ty::Union { name, .. }) => Some(*name),
        Some(Ty::Opaque(name)) => Some(*name),
        _ => None,
    };
    if let Some(subject_union) = subject_union {
        for (union_name, _, fields) in entries {
            if *union_name == subject_union {
                return Some(fields.clone());
            }
        }
    }
    entries.last().map(|(_, _, fields)| fields.clone())
}
