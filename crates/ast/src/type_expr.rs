use std::collections::HashMap;
use std::fmt;

use crate::GroupPath;
use crate::expr::{Expr, Literal};
use crate::parameter::ParameterKind;
use crate::path::ModPath;
use crate::span::{SpanId, Spanned};
use crate::ty::{Ty, VariantMap};
use i256::I256;
use internment::Intern;

/// A type expression — the right-hand side of a type annotation.
///
/// Type expressions remain distinct from ordinary value expressions while
/// preserving the source-level type surface used by declarations.
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
    /// Reference type with an optional alias group.
    Ref {
        inner: Box<Spanned<TypeExpr>>,
        mutable: bool,
        group: Option<GroupPath>,
    },
    /// The unit type `()`.
    Unit,
    /// Scalar in an inclusive range: `x in 0 to 10` or `week_num in WeekRange`.
    InRange { bounds: InRangeBounds, span: SpanId },
}

pub(crate) fn resolve_expr_type(expr: &Expr, tag_types: &HashMap<Intern<String>, Ty>) -> Ty {
    match expr {
        Expr::AnonymousTag(name) => tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name)),
        Expr::FnCall(call) if call.args.as_ref().is_none_or(Vec::is_empty) => {
            let name = call.path.segments.last().unwrap_or(&call.path.root);
            tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name))
        }
        Expr::TagCall(call) => tag_types
            .get(&call.name)
            .cloned()
            .unwrap_or(Ty::Opaque(call.name)),
        Expr::Ref { inner, mutable, .. } => Ty::Ref {
            inner: Box::new(resolve_expr_type(&inner.value, tag_types)),
            mutable: *mutable,
        },
        Expr::TakePtr(inner) => Ty::Ptr {
            inner: Box::new(resolve_expr_type(&inner.value, tag_types)),
        },
        _ => Ty::Opaque(Intern::from_ref("<expr>")),
    }
}

/// Right-hand side of `in` in a type position.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InRangeBounds {
    Literal(I256, I256),
    LiteralToTag(I256, Intern<String>),
    Tag(Intern<String>),
}

impl fmt::Display for InRangeBounds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "in ")?;
        match self {
            InRangeBounds::Literal(min, max) => write!(f, "{min} to {max}"),
            InRangeBounds::LiteralToTag(min, max) => write!(f, "{min} to {max}"),
            InRangeBounds::Tag(name) => write!(f, "{}", name.as_str()),
        }
    }
}

/// Payload fields for a variant, looking up the variant in the variant map
/// and matching against the subject type's union name.
pub(crate) fn payload_fields_for_variant(
    variant_name: Intern<String>,
    subject_ty: Option<&Ty>,
    variant_map: &VariantMap,
) -> Option<Vec<(Intern<String>, Ty)>> {
    if let Some(Ty::Union { variants, .. }) = subject_ty {
        return variants
            .iter()
            .find(|variant| variant.name == variant_name)
            .map(|variant| {
                variant
                    .fields
                    .iter()
                    .map(|(name, ty)| (*name, (**ty).clone()))
                    .collect()
            });
    }
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
