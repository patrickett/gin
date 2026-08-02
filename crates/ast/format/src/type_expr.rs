//! Surface formatting for `TypeExpr` — human-readable type summaries for hover and diagnostics.
//!
//! Also provides variant shape and parameter kind surface formatting for use
//! by hover, diagnostics, and code generation.

use std::fmt;

use ast::expr::Expr;
use ast::parameter::ParameterKind;
use ast::{Pattern, TypeExpr};

pub trait ExprFormatExt {
    fn format_surface(&self) -> String;
    fn write_surface(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result;
}

impl ExprFormatExt for Expr {
    fn format_surface(&self) -> String {
        struct Fmt<'a>(&'a Expr);
        impl fmt::Display for Fmt<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.write_surface(f)
            }
        }
        Fmt(self).to_string()
    }

    fn write_surface(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::AnonymousTag(name) => write!(f, "{}", name.as_str()),
            Expr::FnCall(call) => {
                write!(f, "{}", call.path.value.root.as_str())?;
                for segment in &call.path.value.segments {
                    write!(f, ".{}", segment.as_str())?;
                }
                if let Some(args) = &call.args
                    && !args.is_empty()
                {
                    write!(f, "(")?;
                    for (index, arg) in args.iter().enumerate() {
                        if index > 0 {
                            write!(f, ", ")?;
                        }
                        arg.value.write_surface(f)?;
                    }
                    write!(f, ")")?;
                }
                Ok(())
            }
            Expr::TagCall(call) => {
                if let Some(path) = &call.qual_path {
                    write!(f, "{}", path.value.root.as_str())?;
                    for segment in &path.value.segments {
                        write!(f, ".{}", segment.as_str())?;
                    }
                } else {
                    write!(f, "{}", call.name.as_str())?;
                }
                if !call.args.is_empty() {
                    write!(f, "(")?;
                    for (index, arg) in call.args.iter().enumerate() {
                        if index > 0 {
                            write!(f, ", ")?;
                        }
                        arg.value.write_surface(f)?;
                    }
                    write!(f, ")")?;
                }
                Ok(())
            }
            Expr::Lit(lit) => write!(f, "{lit}"),
            Expr::Ref { inner, mutable, .. } => {
                if *mutable {
                    write!(f, "mut ")?;
                } else {
                    write!(f, "ref ")?;
                }
                inner.value.write_surface(f)
            }
            Expr::TakePtr(inner) => {
                write!(f, "@")?;
                inner.value.write_surface(f)
            }
            Expr::TupleLit(values) => {
                write!(f, "(")?;
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    value.value.write_surface(f)?;
                }
                write!(f, ")")
            }
            Expr::List(values) => {
                write!(f, "[")?;
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    value.value.write_surface(f)?;
                }
                write!(f, "]")
            }
            Expr::Range(range) => {
                range.start.value.write_surface(f)?;
                write!(f, "...")?;
                range.end.value.write_surface(f)
            }
            other => write!(f, "{other:?}"),
        }
    }
}

pub trait PatternFormatExt {
    fn format_variant_shape(&self) -> String;
}

impl PatternFormatExt for Pattern {
    fn format_variant_shape(&self) -> String {
        match self {
            Pattern::Nominal(name, _) => name.as_str().to_string(),
            Pattern::Qualified(path) => {
                let mut out = path.value.root.as_str().to_string();
                for segment in &path.value.segments {
                    out.push('.');
                    out.push_str(segment.as_str());
                }
                out
            }
            Pattern::Generic { name, params, .. } => {
                let params = params
                    .iter()
                    .map(|(param_name, kind)| match kind {
                        ParameterKind::Tagged(sp) => {
                            let surface = sp.value.format_surface();
                            if surface == param_name.as_str() {
                                surface
                            } else {
                                format!("{} {}", param_name.as_str(), surface)
                            }
                        }
                        ParameterKind::ValueParam { ty } => {
                            format!("{} {}", param_name.as_str(), ty.value.format_surface())
                        }
                        ParameterKind::Inferred { ty } => format!(
                            "{} {}: ?",
                            param_name.as_str(),
                            ty.value.format_surface()
                        ),
                        ParameterKind::Generic => param_name.as_str().to_string(),
                        ParameterKind::Default(expr) => {
                            format!("{}: {:?}", param_name.as_str(), expr.value)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}({})", name.as_str(), params)
            }
            Pattern::Literal(lit, _) => lit.to_string(),
            Pattern::Pointer(inner) => format!("@{}", inner.value.format_variant_shape()),
            Pattern::Ref {
                inner, mutable, group, ..
            } => format!(
                "{}{}{}",
                if *mutable { "mut" } else { "ref" },
                group.as_ref().map(|g| format!("{{{g}}}")).unwrap_or_default(),
                format_args!(" {}", inner.value.format_variant_shape())
            ),
            Pattern::Unit => "()".to_string(),
            Pattern::ListEmpty => "[]".to_string(),
            Pattern::ListCons { head, tail } => format!(
                "[{}, ...{}]",
                head.value.format_variant_shape(),
                tail.value.format_variant_shape()
            ),
            Pattern::Tuple(elements) => format!(
                "({})",
                elements
                    .iter()
                    .map(|element| element.value.format_variant_shape())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Pattern::InRange { bounds, .. } => bounds.to_string(),
        }
    }
}

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
                self.0.write_surface(f)
            }
        }
        Fmt(self).to_string()
    }

    fn write_surface(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
                            let surface = te.format_surface();
                            // Positional type arg (key equals mangled type name):
                            // just show the type. Named param (key differs): show key + type.
                            if surface == k.as_str() {
                                write!(f, "{surface}")?;
                            } else {
                                write!(f, "{} ", k.as_str())?;
                                write!(f, "{surface}")?;
                            }
                        }
                        ParameterKind::ValueParam { ty } => {
                            write!(f, "{} ", k.as_str())?;
                            ty.value.write_surface(f)?;
                        }
                        ParameterKind::Inferred { ty } => {
                            write!(f, "{} ", k.as_str())?;
                            ty.value.write_surface(f)?;
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
                inner.value.write_surface(f)
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
                    write!(f, "{{{group}}}")?;
                }
                write!(f, " ")?;
                inner.value.write_surface(f)
            }
            TypeExpr::Unit => write!(f, "()"),
            TypeExpr::InRange { bounds, .. } => write!(f, "{bounds}"),
        }
    }

    fn format_variant_shape(&self) -> String {
        struct Fmt<'a>(&'a TypeExpr);
        impl fmt::Display for Fmt<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.write_variant_shape(f)
            }
        }
        Fmt(self).to_string()
    }

    fn write_variant_shape(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
                inner.value.write_variant_shape(f)
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
                    write!(f, "{{{group}}}")?;
                }
                write!(f, " ")?;
                inner.value.write_variant_shape(f)
            }
            TypeExpr::Unit => write!(f, "()"),
            TypeExpr::InRange { bounds, .. } => write!(f, "{bounds}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use ast::parameter::ParameterKind;
    use ast::span::{SpanId, Spanned};
    use ast::{Expr, Pattern, TypeExpr};
    use internment::Intern;

    use super::{PatternFormatExt, TypeExprFormatExt};

    fn pattern_name(name: &str) -> Spanned<Pattern> {
        Spanned {
            value: Pattern::Nominal(Intern::new(name.to_string()), SpanId::INVALID),
            span_id: SpanId::INVALID,
        }
    }

    fn list_cons(head: Spanned<Pattern>, tail: Spanned<Pattern>) -> Pattern {
        Pattern::ListCons {
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
                            value: match ty {
                                TypeExpr::Nominal(name, _) => Expr::AnonymousTag(name),
                                TypeExpr::Literal(lit, _) => Expr::Lit(lit),
                                _ => Expr::Lit(ast::Literal::Number(0)),
                            },
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
    fn list_cons_variant_shape_surface_uses_js_rest_syntax() {
        let expr = list_cons(pattern_name("head"), pattern_name("tail"));

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
