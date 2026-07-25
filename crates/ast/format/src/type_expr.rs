//! Surface formatting for `TypeExpr` — human-readable type summaries for hover and diagnostics.
//!
//! Also provides variant shape and parameter kind surface formatting for use
//! by hover, diagnostics, and code generation.

use std::fmt;

use ast::TypeExpr;
use ast::parameter::ParameterKind;

/// Extension trait providing surface formatting methods on [`TypeExpr`].
pub trait TypeExprFormatExt {
    /// Format this type expression to a string using the canonical Gin surface representation.
    fn format_surface(&self) -> String;

    /// Write this type expression surface representation to a formatter.
    fn write_surface(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result;

    /// Format this type expression as a union variant shape string.
    fn format_variant_shape(&self) -> String;

    /// Write this type expression as a union variant shape to a formatter.
    fn write_variant_shape(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result;
}

impl TypeExprFormatExt for TypeExpr {
    fn format_surface(&self) -> String {
        struct Fmt<'a>(&'a TypeExpr);
        impl fmt::Display for Fmt<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write_type_expr_surface(self.0, f)
            }
        }
        Fmt(self).to_string()
    }

    fn write_surface(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_type_expr_surface(self, f)
    }

    fn format_variant_shape(&self) -> String {
        struct Fmt<'a>(&'a TypeExpr);
        impl fmt::Display for Fmt<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write_variant_shape_surface(self.0, f)
            }
        }
        Fmt(self).to_string()
    }

    fn write_variant_shape(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_variant_shape_surface(self, f)
    }
}

/// Write a `TypeExpr` surface representation to a formatter.
pub(crate) fn write_type_expr_surface(e: &TypeExpr, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match e {
        TypeExpr::Nominal(name, _) => write!(f, "{}", name.as_str()),
        TypeExpr::Qualified(path) => {
            write!(f, "{}", path.root.as_str())?;
            for seg in &path.segments {
                write!(f, ".{}", seg.as_str())?;
            }
            Ok(())
        }
        TypeExpr::Generic { name, params, .. } => {
            write!(f, "{}(", name.as_str())?;
            let mut first = true;
            for (k, v) in params.iter() {
                if !first {
                    write!(f, ", ")?;
                }
                first = false;
                match v {
                    ParameterKind::Tagged(sp) => {
                        let te = &sp.value;
                        // Positional type arg (key equals mangled type name):
                        // just show the type. Named param (key differs): show key + type.
                        if te.surface_mangle_name() == k.as_str() {
                            write_type_expr_surface(te, f)?;
                        } else {
                            write!(f, "{} ", k.as_str())?;
                            write_type_expr_surface(te, f)?;
                        }
                    }
                    ParameterKind::ValueParam { ty } => {
                        write!(f, "{} ", k.as_str())?;
                        write_type_expr_surface(&ty.value, f)?;
                    }
                    ParameterKind::Inferred { ty } => {
                        write!(f, "{} ", k.as_str())?;
                        write_type_expr_surface(&ty.value, f)?;
                        write!(f, ": ?")?;
                    }
                    ParameterKind::Default(expr) => {
                        write!(f, "{}: {expr:?}", k.as_str())?;
                    }
                    ParameterKind::Generic => {
                        write!(f, "{}", k.as_str())?;
                    }
                }
            }
            write!(f, ")")
        }
        TypeExpr::Literal(..) => write!(f, "<type>"),
        TypeExpr::Pointer(inner) => {
            write!(f, "@")?;
            write_type_expr_surface(&inner.value, f)
        }
        TypeExpr::Ref {
            inner,
            mutable,
            group,
        } => {
            if *mutable {
                write!(f, "mut")?;
            } else {
                write!(f, "ref")?;
            }
            if let Some(group) = group {
                write!(f, "{{{}}}", group.as_str())?;
            }
            write!(f, " ")?;
            write_type_expr_surface(&inner.value, f)
        }
        TypeExpr::Unit => write!(f, "()"),
        TypeExpr::InRange { bounds, .. } => write!(f, "{bounds}"),
        TypeExpr::ListEmpty => write!(f, "[]"),
        TypeExpr::ListCons { head, tail } => {
            write!(f, "[")?;
            write_type_expr_surface(&head.value, f)?;
            write!(f, ", ...")?;
            write_type_expr_surface(&tail.value, f)?;
            write!(f, "]")
        }
        TypeExpr::Tuple(elems) => {
            write!(f, "(")?;
            for (i, e) in elems.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write_type_expr_surface(&e.value, f)?;
            }
            write!(f, ")")
        }
    }
}

/// Write a union variant shape surface representation to a formatter.
pub(crate) fn write_variant_shape_surface(e: &TypeExpr, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match e {
        TypeExpr::Generic { name, params, .. } => {
            write!(f, "{}(", name.as_str())?;
            let mut first = true;
            for (k, v) in params {
                if !first {
                    write!(f, ", ")?;
                }
                first = false;
                match v {
                    ParameterKind::Tagged(sp) => {
                        let type_str = sp.value.format_surface();
                        if type_str == k.as_str() {
                            write!(f, "{}", type_str)?;
                        } else {
                            write!(f, "{} {}", k.as_str(), type_str)?;
                        }
                    }
                    ParameterKind::ValueParam { ty } => {
                        write!(f, "{} {}", k.as_str(), ty.value.format_surface())?;
                    }
                    ParameterKind::Inferred { ty } => {
                        write!(f, "{} {}: ?", k.as_str(), ty.value.format_surface())?;
                    }
                    ParameterKind::Generic => write!(f, "{}", k.as_str())?,
                    ParameterKind::Default(expr) => write!(f, "{}: {:?}", k.as_str(), expr)?,
                }
            }
            write!(f, ")")
        }
        TypeExpr::Nominal(name, _) => write!(f, "{}", name.as_str()),
        TypeExpr::Qualified(path) => {
            write!(f, "{}", path.root.as_str())?;
            for seg in &path.segments {
                write!(f, ".{}", seg.as_str())?;
            }
            Ok(())
        }
        TypeExpr::Literal(lit, _) => write!(f, "{lit}"),
        TypeExpr::Pointer(inner) => {
            write!(f, "@")?;
            write_variant_shape_surface(&inner.value, f)
        }
        TypeExpr::Ref {
            inner,
            mutable,
            group,
        } => {
            if *mutable {
                write!(f, "mut")?;
            } else {
                write!(f, "ref")?;
            }
            if let Some(group) = group {
                write!(f, "{{{}}}", group.as_str())?;
            }
            write!(f, " ")?;
            write_variant_shape_surface(&inner.value, f)
        }
        TypeExpr::Unit => write!(f, "()"),
        TypeExpr::InRange { bounds, .. } => write!(f, "{bounds}"),
        TypeExpr::ListEmpty => write!(f, "[]"),
        TypeExpr::ListCons { head, tail } => {
            write!(f, "[")?;
            write_variant_shape_surface(&head.value, f)?;
            write!(f, ", ...")?;
            write_variant_shape_surface(&tail.value, f)?;
            write!(f, "]")
        }
        TypeExpr::Tuple(elems) => {
            write!(f, "(")?;
            for (i, e) in elems.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write_variant_shape_surface(&e.value, f)?;
            }
            write!(f, ")")
        }
    }
}

#[cfg(test)]
mod tests {
    use ast::TypeExpr;
    use ast::parameter::ParameterKind;
    use ast::span::{SpanId, Spanned};
    use internment::Intern;

    use super::TypeExprFormatExt;

    fn name(name: &str) -> Spanned<TypeExpr> {
        Spanned {
            value: TypeExpr::Nominal(Intern::new(name.to_string()), SpanId::INVALID),
            span_id: SpanId::INVALID,
        }
    }

    fn list_cons(head: Spanned<TypeExpr>, tail: Spanned<TypeExpr>) -> TypeExpr {
        TypeExpr::ListCons {
            head: Box::new(head),
            tail: Box::new(tail),
        }
    }

    fn generic(name: &str, params: Vec<(&str, TypeExpr)>) -> TypeExpr {
        TypeExpr::Generic {
            name: Intern::new(name.to_string()),
            params: params
                .into_iter()
                .map(|(name, ty)| {
                    (
                        Intern::new(name.to_string()),
                        ParameterKind::Tagged(Box::new(Spanned {
                            value: ty,
                            span_id: SpanId::INVALID,
                        })),
                    )
                })
                .collect(),
            param_spans: Vec::new(),
            span: SpanId::INVALID,
        }
    }

    #[test]
    fn list_cons_type_surface_uses_js_rest_syntax() {
        let expr = list_cons(name("head"), name("tail"));

        assert_eq!(expr.format_surface(), "[head, ...tail]");
    }

    #[test]
    fn list_cons_variant_shape_surface_uses_js_rest_syntax() {
        let expr = list_cons(name("head"), name("tail"));

        assert_eq!(expr.format_variant_shape(), "[head, ...tail]");
    }

    #[test]
    fn variant_shape_surface_keeps_field_names() {
        let expr = generic(
            "Primitive",
            vec![
                (
                    "width",
                    TypeExpr::Nominal(Intern::new("BigInt".to_string()), SpanId::INVALID),
                ),
                (
                    "signed",
                    TypeExpr::Nominal(Intern::new("Bool".to_string()), SpanId::INVALID),
                ),
            ],
        );

        assert_eq!(
            expr.format_variant_shape(),
            "Primitive(width BigInt, signed Bool)"
        );
    }
}
