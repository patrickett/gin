use std::fmt::Write;

use crate::type_expr::TypeExprFormatExt;
use ast::declare::{Declare, DeclareValue};
use ast::expr::Bind;
use ast::parameter::ParameterKind;

/// Extension trait providing [`Declare::surface_text`].
pub trait DeclareFormatExt {
    /// Type surface for hover and docs: name, params, and `is`/`has` value only.
    ///
    /// Omits `Type.Trait(...)` implementations (those live in
    /// [`Declare::provided_traits`]).
    fn surface_text(&self) -> String;
}

impl DeclareFormatExt for Declare {
    fn surface_text(&self) -> String {
        let mut out = String::new();
        out.push_str(self.name.as_str());
        if let Some(params) = &self.params {
            out.push('(');
            let mut first = true;
            for (k, v) in params {
                if !first {
                    out.push_str(", ");
                }
                first = false;
                let _ = match v {
                    ParameterKind::Tagged(sp) => {
                        let _ = write!(&mut out, "{} ", k.as_str());
                        out.push_str(&sp.value.format_surface());
                        Ok(())
                    }
                    ParameterKind::ValueParam { ty } => {
                        let _ = write!(&mut out, "{} ", k.as_str());
                        out.push_str(&ty.value.format_surface());
                        Ok(())
                    }
                    kind => write!(&mut out, "{}{kind}", k.as_str()),
                };
            }
            out.push(')');
        }
        if matches!(&self.value, DeclareValue::Interface(members) if members.is_empty())
            && !self.provided_traits.is_empty()
            && self.provided_traits.iter().all(|pt| pt.fields.is_empty())
        {
            out.push_str(" is ");
            let mut first = true;
            for pt in &self.provided_traits {
                if !first {
                    out.push_str(" and ");
                }
                first = false;
                out.push_str(pt.trait_name.as_str());
            }
            return out;
        }

        let continuation_indent = out.len() + 1;
        if let DeclareValue::Interface(_) = &self.value {
            out.push_str(" has")
        }
        out.push_str(&format_declare_value(&self.value, continuation_indent));
        out
    }
}

/// Extension trait providing [`Bind::signature_surface`].
pub trait BindFormatExt {
    /// Source-level signature for IDE hover (parameter and return type surfaces as written).
    fn signature_surface(&self) -> String;
}

impl BindFormatExt for Bind {
    fn signature_surface(&self) -> String {
        let mut sig = self.name.as_str().to_string();
        if let Some(params) = &self.params {
            sig.push('(');
            let mut first = true;
            for (name, kind) in params {
                if !first {
                    sig.push_str(", ");
                }
                first = false;
                sig.push_str(name.as_str());
                match kind {
                    ParameterKind::Tagged(sp) => {
                        sig.push(' ');
                        sig.push_str(&sp.value.format_surface());
                    }
                    ParameterKind::ValueParam { ty } => {
                        sig.push(' ');
                        sig.push_str(&ty.value.format_surface());
                    }
                    ParameterKind::Generic => {}
                    ParameterKind::Default(expr) => {
                        sig.push_str(&format!(": {:?}", expr.value));
                    }
                }
            }
            sig.push(')');
            if let Some(sp) = &self.return_tag {
                sig.push(' ');
                sig.push_str(&sp.value.format_surface());
            }
        } else if let Some(sp) = &self.return_tag {
            sig.push(' ');
            sig.push_str(&sp.value.format_surface());
        } else if let Some(rtn) = &self.return_type_name {
            sig.push(' ');
            sig.push_str(rtn.as_str());
        }
        sig
    }
}

/// Format the value side of a `Declare` for surface text (e.g. `is True or False`).
fn format_declare_value(value: &DeclareValue, continuation_indent: usize) -> String {
    match value {
        DeclareValue::Alias(sp) => format!(" is {}", sp.value.format_surface()),
        DeclareValue::When(_) => " is when …".to_string(),
        DeclareValue::Union { variants } => {
            let all_literal = variants
                .iter()
                .all(|v| matches!(v.shape().value, ast::TypeExpr::Literal(..)));
            let multiline = all_literal
                || variants.iter().any(|v| {
                    matches!(
                        v.shape().value,
                        ast::TypeExpr::Generic { ref params, .. } if !params.is_empty()
                    )
                });
            if multiline && !variants.is_empty() {
                let mut out = format!(" is {}", variants[0].shape().value.format_variant_shape());
                let indent = " ".repeat(continuation_indent);
                for v in &variants[1..] {
                    let _ = write!(
                        &mut out,
                        "\n{indent}or {}",
                        v.shape().value.format_variant_shape()
                    );
                }
                return out;
            }
            let mut out = " is ".to_string();
            let mut first = true;
            for v in variants {
                if !first {
                    out.push_str(" or ");
                }
                first = false;
                out.push_str(&v.shape().value.format_variant_shape());
            }
            out
        }
        DeclareValue::Interface(members) => {
            if members.is_empty() {
                return " ()".to_string();
            }
            let mut out = " (\n".to_string();
            for (i, m) in members.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                let _ = write!(&mut out, "    {}", m.name.as_str());
                if !m.params.is_empty() {
                    out.push('(');
                    let mut param_first = true;
                    for (k, v) in &m.params {
                        if !param_first {
                            out.push_str(", ");
                        }
                        param_first = false;
                        if let Some(conv) = m.conventions.get(k) {
                            match conv {
                                ast::ParamConvention::Ref(false) => out.push_str("ref "),
                                ast::ParamConvention::Ref(true) => out.push_str("mut "),
                                ast::ParamConvention::Eat => out.push_str("eat "),
                                ast::ParamConvention::Inferred => {}
                            }
                        }
                        let _ = write!(&mut out, "{}", k.as_str());
                        match v {
                            ast::ParameterKind::Tagged(sp) => {
                                out.push(' ');
                                out.push_str(&sp.value.format_surface());
                            }
                            ast::ParameterKind::ValueParam { ty } => {
                                out.push(' ');
                                out.push_str(&ty.value.format_surface());
                            }
                            kind => {
                                let _ = write!(&mut out, "{kind}");
                            }
                        }
                    }
                    out.push(')');
                }
                if let Some(rt) = &m.return_ty {
                    out.push(' ');
                    out.push_str(&rt.value.format_surface());
                }
                if let Some(et) = &m.error_ty {
                    out.push_str(" or ");
                    out.push_str(&et.value.format_surface());
                }
            }
            out.push_str(",\n)");
            out
        }
        DeclareValue::Set() => " is set".to_string(),
        DeclareValue::Range(start, end) => format!(" is {start}...{end}"),
        DeclareValue::InRange(start, end) => format!(" is in {start}...{end}"),
    }
}

#[cfg(test)]
mod tests {
    use ast::declare::{Declare, DeclareValue};
    use ast::parameter::ParameterKind;
    use ast::span::{SpanId, Spanned};
    use ast::{TypeExpr, Variant};
    use internment::Intern;

    use super::DeclareFormatExt;

    fn intern(value: &str) -> Intern<String> {
        Intern::new(value.to_string())
    }

    fn nominal(name: &str) -> TypeExpr {
        TypeExpr::Nominal(intern(name), SpanId::INVALID)
    }

    fn spanned(value: TypeExpr) -> Spanned<TypeExpr> {
        Spanned {
            value,
            span_id: SpanId::INVALID,
        }
    }

    fn tagged(value: TypeExpr) -> ParameterKind {
        ParameterKind::Tagged(Box::new(spanned(value)))
    }

    fn generic(name: &str, params: Vec<(&str, TypeExpr)>) -> TypeExpr {
        TypeExpr::Generic {
            name: intern(name),
            params: params
                .into_iter()
                .map(|(name, ty)| (intern(name), tagged(ty)))
                .collect(),
            param_spans: Vec::new(),
            span: SpanId::INVALID,
        }
    }

    fn variant(shape: TypeExpr) -> Variant {
        Variant::External {
            shape: Box::new(spanned(shape)),
            result_ty: None,
        }
    }

    #[test]
    fn declaration_surface_keeps_simple_nominal_union_compact() {
        let declare = Declare::new(
            intern("Bool"),
            SpanId::INVALID,
            DeclareValue::Union {
                variants: vec![variant(nominal("True")), variant(nominal("False"))],
            },
        );

        assert_eq!(declare.surface_text(), "Bool is True or False");
    }

    #[test]
    fn declaration_surface_empty_interface_uses_compact_parens() {
        let declare = Declare::new(
            intern("AllocError"),
            SpanId::INVALID,
            DeclareValue::Interface(Vec::new()),
        );

        assert_eq!(declare.surface_text(), "AllocError has ()");
    }

    #[test]
    fn declaration_surface_multilines_payload_union_with_named_fields() {
        let declare = Declare::new(
            intern("Type"),
            SpanId::INVALID,
            DeclareValue::Union {
                variants: vec![
                    variant(generic(
                        "Primitive",
                        vec![("width", nominal("BigInt")), ("signed", nominal("Bool"))],
                    )),
                    variant(generic(
                        "Record",
                        vec![
                            ("name", nominal("String")),
                            (
                                "fields",
                                generic("List", vec![("NamedTy", nominal("NamedTy"))]),
                            ),
                        ],
                    )),
                    variant(generic("Ptr", vec![("inner", nominal("Type"))])),
                ],
            },
        );

        assert_eq!(
            declare.surface_text(),
            "Type is Primitive(width BigInt, signed Bool)\n     or Record(name String, fields List(NamedTy))\n     or Ptr(inner Type)"
        );
    }
}
