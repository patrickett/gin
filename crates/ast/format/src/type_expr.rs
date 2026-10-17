//! Surface formatting for `TypeExpr` — human-readable type summaries for hover and diagnostics.
//!
//! Also provides variant shape and parameter kind surface formatting for use
//! by hover, diagnostics, and code generation.

use internment::Intern;
use std::fmt;

use ast::expr::Expr;
use ast::parameter::ParameterKind;
use ast::{Pattern, Typed};

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
            Expr::Todo => write!(f, "#todo"),
            Expr::ResultValues(values) if values.is_empty() => write!(f, "()"),
            Expr::Binary(binary) if binary.op == ast::BinOp::LogicalOr => {
                binary.lhs.value.write_surface(f)?;
                write!(f, " or ")?;
                binary.rhs.value.write_surface(f)
            }
            Expr::Binary(binary) => {
                write!(f, "(")?;
                binary.lhs.value.write_surface(f)?;
                write!(f, " {} ", binary.op.symbol())?;
                binary.rhs.value.write_surface(f)?;
                write!(f, ")")
            }
            Expr::Negate(inner) => {
                write!(f, "-")?;
                inner.value.write_surface(f)
            }
            Expr::AnonymousTag(name) => write!(f, "{}", name.as_str()),
            Expr::SelfRef => write!(f, "self"),
            Expr::RecordGet { base, field } => {
                base.value.write_surface(f)?;
                write!(f, ".{field}")
            }
            Expr::FnCall(call) => {
                let args = call.args.as_deref();
                let args = if call.syntax == ast::CallSyntax::Receiver {
                    let (receiver, args) =
                        args.and_then(|args| args.split_first()).ok_or(fmt::Error)?;
                    receiver.value.write_surface(f)?;
                    write!(f, ".")?;
                    Some(args)
                } else {
                    args
                };
                write!(f, "{}", call.path.value.to_qualified_string())?;
                if let Some(args) = args
                    && (!args.is_empty() || call.syntax == ast::CallSyntax::Receiver)
                {
                    write_expr_list(f, "(", ")", args)?;
                }
                Ok(())
            }
            Expr::TagCall(call) => {
                if let Some(path) = &call.qual_path {
                    write!(f, "{}", path.value.to_qualified_string())?;
                } else {
                    write!(f, "{}", call.name.as_str())?;
                }
                if !call.args.is_empty() {
                    write_expr_list(f, "(", ")", &call.args)?;
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
            Expr::ArrayLit(values) | Expr::List(values) => write_expr_list(f, "[", "]", values),
            Expr::SequenceLiteral(ast::SequenceLiteral::Elements(elems)) => {
                write!(f, "[")?;
                for (index, elem) in elems.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    if let Some(name) = elem.name {
                        write!(f, "{name} ")?;
                    }
                    elem.value.write_surface(f)?;
                }
                write!(f, "]")
            }
            Expr::SequenceLiteral(ast::SequenceLiteral::Repeat { seed, count }) => {
                write!(f, "[")?;
                seed.value.write_surface(f)?;
                write!(f, "; ")?;
                count.value.write_surface(f)?;
                write!(f, "]")
            }
            Expr::DomainOf(inner) => {
                write!(f, "domain of {}", inner.value.format_surface())
            }
            Expr::IntegerDomain(inner) => {
                write!(f, "is in ")?;
                inner.value.write_surface(f)
            }
            Expr::Emit { code, arguments } => {
                write!(f, "#emit(")?;
                code.value.write_surface(f)?;
                write!(f, ", ")?;
                arguments.value.write_surface(f)?;
                write!(f, ")")
            }
            Expr::Range(range) => {
                range.start.value.write_surface(f)?;
                write!(f, " to ")?;
                range.end.value.write_surface(f)
            }
            other => write!(f, "{other:?}"),
        }
    }
}

fn write_expr_list(
    f: &mut fmt::Formatter<'_>,
    open: &str,
    close: &str,
    exprs: &[Typed<Expr>],
) -> fmt::Result {
    write!(f, "{open}")?;
    for (index, expr) in exprs.iter().enumerate() {
        if index > 0 {
            write!(f, ", ")?;
        }
        expr.value.write_surface(f)?;
    }
    write!(f, "{close}")
}

fn write_params(
    f: &mut impl fmt::Write,
    name: &str,
    params: &[(Intern<String>, ParameterKind)],
) -> fmt::Result {
    write!(f, "{name}(")?;
    for (index, (param_name, kind)) in params.iter().enumerate() {
        if index > 0 {
            write!(f, ", ")?;
        }
        write_param(f, param_name.as_str(), kind)?;
    }
    write!(f, ")")
}

fn write_param(f: &mut impl fmt::Write, name: &str, kind: &ParameterKind) -> fmt::Result {
    match kind {
        ParameterKind::Tagged(sp) => {
            let surface = sp.value.format_surface();
            if surface == name {
                write!(f, "{surface}")
            } else {
                write!(f, "{name} {surface}")
            }
        }
        ParameterKind::ValueParam { ty } => write!(f, "{name} {}", ty.value.format_surface()),
        ParameterKind::Inferred { ty } => {
            write!(f, "{name} {}: ?", ty.value.format_surface())
        }
        ParameterKind::Generic => write!(f, "{name}"),
        ParameterKind::Proposition(predicate) => write!(f, "{name} is {predicate}"),
        ParameterKind::Default(expr) => write!(f, "{name}: {:?}", expr.value),
    }
}

pub trait PatternFormatExt {
    fn format_variant_shape(&self) -> String;
}

impl PatternFormatExt for Pattern {
    fn format_variant_shape(&self) -> String {
        match self {
            Pattern::Invalid => "<invalid>".to_string(),
            Pattern::Nominal(name, _) => name.as_str().to_string(),
            Pattern::Qualified(path) => path.value.to_qualified_string(),
            Pattern::Generic { name, params, .. } => {
                let mut out = String::new();
                let _ = write_params(&mut out, name.as_str(), params);
                out
            }
            Pattern::Literal(lit, _) => lit.to_string(),
            Pattern::Pointer(inner) => format!("@{}", inner.value.format_variant_shape()),
            Pattern::Ref {
                inner,
                mutable,
                group,
                ..
            } => format!(
                "{}{}{}",
                if *mutable { "mut" } else { "ref" },
                group
                    .as_ref()
                    .map(|g| format!("{{{g}}}"))
                    .unwrap_or_default(),
                format_args!(" {}", inner.value.format_variant_shape())
            ),
            Pattern::Unit => "()".to_string(),
            Pattern::ListEmpty => "[]".to_string(),
            Pattern::ListCons { head, tail } => format!(
                "[{}, ...{}]",
                head.value.format_variant_shape(),
                tail.value.format_variant_shape()
            ),
            Pattern::InRange { bounds, .. } => bounds.to_string(),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/type_expr_tests.rs"]
mod tests;
