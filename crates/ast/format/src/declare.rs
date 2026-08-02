use std::fmt::Write;

use crate::type_expr::{ExprFormatExt, PatternFormatExt};
use ast::declare::{Declare, DeclareValue, HasMember};
use ast::expr::Bind;
use ast::parameter::ParameterKind;

/// Extension trait providing [`Declare::surface_text`].
pub trait DeclareFormatExt {
    /// Type surface for hover and docs: name, params, and `is`/`has` value only.
    ///
    /// Includes the declaration surface while provision values remain in
    /// [`Declare::provided_traits`].
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
                let _ = match &v.kind {
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
        if matches!(&self.value, DeclareValue::Has(members) if members.is_empty())
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
        if matches!(&self.value, DeclareValue::Has(_)) {
            out.push_str(" has")
        }
        out.push_str(&self.value.format_value(continuation_indent));
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
                let kind = &kind.kind;
                match kind {
                    ParameterKind::Tagged(sp) => {
                        if let ast::Expr::Ref { inner, mutable, .. } = &sp.value
                        {
                            if *mutable {
                                sig.push_str("mut");
                            } else {
                                sig.push_str("ref");
                            }
                            let _ = write!(&mut sig, " {} ", name.as_str());
                            sig.push_str(&inner.value.format_surface());
                        } else {
                            sig.push_str(name.as_str());
                            sig.push(' ');
                            sig.push_str(&sp.value.format_surface());
                        }
                    }
                    ParameterKind::ValueParam { ty } => {
                        sig.push_str(name.as_str());
                        sig.push(' ');
                        sig.push_str(&ty.value.format_surface());
                    }
                    ParameterKind::Inferred { ty } => {
                        sig.push_str(name.as_str());
                        sig.push(' ');
                        sig.push_str(&ty.value.format_surface());
                        sig.push_str(": ?");
                    }
                    ParameterKind::Generic => sig.push_str(name.as_str()),
                    ParameterKind::Default(expr) => {
                        sig.push_str(name.as_str());
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

/// Extension trait providing the value-side surface text for a [`DeclareValue`].
trait DeclareValueFormatExt {
    fn format_value(&self, continuation_indent: usize) -> String;
}

impl DeclareValueFormatExt for DeclareValue {
    fn format_value(&self, continuation_indent: usize) -> String {
        match self {
            DeclareValue::Alias(sp) => format!(" is {}", sp.value.format_surface()),
            DeclareValue::When(_) => " is when …".to_string(),
            DeclareValue::Union { variants } => {
                let all_literal = variants
                    .iter()
                    .all(|v| matches!(v.shape().value, ast::Pattern::Literal(..)));
                let multiline = all_literal
                    || variants.iter().any(|v| {
                        matches!(
                            v.shape().value,
                            ast::Pattern::Generic { ref params, .. } if !params.is_empty()
                        )
                    });
                if multiline && !variants.is_empty() {
                    let mut out =
                        format!(" is {}", variants[0].shape().value.format_variant_shape());
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

            DeclareValue::Has(members) => {
                let members: Vec<_> = members
                    .iter()
                    .filter(|member| match member {
                        HasMember::Property(property) => property.qualifier.is_none(),
                        HasMember::Function(function) => function.qualifier.is_none(),
                    })
                    .cloned()
                    .collect();
                if members.is_empty() {
                    return String::new();
                }

                if members.iter().all(|m| match m {
                    HasMember::Property(_) => true,
                    HasMember::Function(f) => f.body.is_none() && f.params.is_empty(),
                }) && members.len() <= 3
                {
                    let mut inline = String::new();
                    for (i, m) in members.iter().enumerate() {
                        if i > 0 {
                            inline.push_str(", ");
                        }
                        match m {
                            HasMember::Property(p) => {
                                let _ = write!(inline, "{}", p.name.as_str());
                                if let Some(ty) = &p.ty {
                                    let _ = write!(inline, " {}", ty.value.format_surface());
                                }
                            }
                            HasMember::Function(f) => {
                                let _ = write!(inline, "{}", f.name.as_str());
                                if let Some(rt) = &f.return_ty {
                                    let _ = write!(inline, " {}", rt.value.format_surface());
                                }
                                if let Some(et) = &f.error_ty {
                                    let _ = write!(inline, " or {}", et.value.format_surface());
                                }
                            }
                        }
                    }
                    // Total rendered line = name + " has" + " " + inline
                    // = (continuation_indent - 1) + 4 + 1 + inline.len()
                    // = continuation_indent + 4 + inline.len()
                    if continuation_indent + 4 + inline.len() <= 40 {
                        let mut out = String::new();
                        out.push(' ');
                        out.push_str(&inline);
                        return out;
                    }
                }

                let mut out = String::new();
                for m in members.iter() {
                    match m {
                        ast::HasMember::Property(p) => {
                            let _ = write!(&mut out, "\n    {}", p.name.as_str());
                            if let Some(ty) = &p.ty {
                                out.push(' ');
                                out.push_str(&ty.value.format_surface());
                            }
                        }
                        ast::HasMember::Function(f) if f.body.is_none() => {
                            let _ = write!(&mut out, "\n    {}", f.name.as_str());
                            if !f.params.is_empty() {
                                out.push('(');
                                let mut first = true;
                                for (k, v) in &f.params {
                                    if !first {
                                        out.push_str(", ");
                                    }
                                    first = false;
                                    if let Some(conv) = f.conventions.get(k) {
                                        match conv {
                                            ast::ParamConvention::Observe => out.push_str("ref "),
                                            ast::ParamConvention::Mutate => out.push_str("mut "),
                                            ast::ParamConvention::Consume => out.push_str("eat "),
                                            ast::ParamConvention::Own => {}
                                        }
                                    }
                                    out.push_str(k.as_str());
                                    match &v.kind {
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
                            if let Some(rt) = &f.return_ty {
                                out.push(' ');
                                out.push_str(&rt.value.format_surface());
                            }
                            if let Some(et) = &f.error_ty {
                                out.push_str(" or ");
                                out.push_str(&et.value.format_surface());
                            }
                        }
                        ast::HasMember::Function(_) => {}
                    }
                }
                out
            }
            DeclareValue::Set() => " is set".to_string(),
            DeclareValue::Range(start, end) => format!(" is {start}...{end}"),
            DeclareValue::InRange(start, end) => format!(" is in {start}...{end}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use ast::declare::{
        Declare, DeclareValue, HasFunction, HasFunctionKind, HasMember, HasMemberBody,
        HasMemberQualifier, HasProperty,
    };
    use ast::parameter::{Parameter, ParameterKind};
    use ast::span::{SpanId, Spanned};
    use ast::{BindValue, Expr, Literal, Pattern, TypeExpr, Typed, Variant};
    use internment::Intern;

    use super::DeclareFormatExt;

    fn intern(value: &str) -> Intern<String> {
        Intern::new(value.to_string())
    }

    fn nominal(name: &str) -> TypeExpr {
        TypeExpr::Nominal(intern(name), SpanId::INVALID)
    }

    fn spanned_expr(value: Expr) -> Spanned<Expr> {
        Spanned {
            value,
            span_id: SpanId::INVALID,
        }
    }

    fn expr_from_type(value: TypeExpr) -> Expr {
        match value {
            TypeExpr::Nominal(name, _) => Expr::AnonymousTag(name),
            TypeExpr::Generic { name, params, .. } => Expr::TagCall(ast::TagCall {
                name,
                qual_path: None,
                args: params
                    .into_iter()
                    .map(|(_, kind)| match kind {
                        ParameterKind::Tagged(sp)
                        | ParameterKind::ValueParam { ty: sp }
                        | ParameterKind::Inferred { ty: sp } => {
                            Typed::infer(sp.value, SpanId::INVALID)
                        }
                        ParameterKind::Generic => {
                            Typed::infer(Expr::AnonymousTag(Intern::new(String::new())), SpanId::INVALID)
                        }
                        ParameterKind::Default(expr) => *expr,
                    })
                    .collect(),
            }),
            TypeExpr::Literal(lit, _) => Expr::Lit(lit),
            _ => Expr::Lit(Literal::Number(0)),
        }
    }

    fn tagged(value: TypeExpr) -> ParameterKind {
        ParameterKind::Tagged(Box::new(Spanned {
            value: expr_from_type(value),
            span_id: SpanId::INVALID,
        }))
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
            shape: Box::new(Spanned {
                value: Pattern::from(shape),
                span_id: SpanId::INVALID,
            }),
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
    fn declaration_surface_empty_has_uses_has_keyword() {
        let declare = Declare::new(
            intern("AllocError"),
            SpanId::INVALID,
            DeclareValue::Has(Vec::new()),
        );

        assert_eq!(declare.surface_text(), "AllocError has");
    }

    #[test]
    fn declaration_surface_simple_has_uses_inline() {
        let declare = Declare::new(
            intern("Bounded"),
            SpanId::INVALID,
            DeclareValue::Has(vec![
                HasMember::Property(HasProperty {
                    qualifier: None,
                    name: intern("min"),
                    name_span: SpanId::INVALID,
                    ty: None,
                    body: None,
                    doc_comment: None,
                    refinement: None,
                }),
                HasMember::Property(HasProperty {
                    qualifier: None,
                    name: intern("max"),
                    name_span: SpanId::INVALID,
                    ty: None,
                    body: None,
                    doc_comment: None,
                    refinement: None,
                }),
            ]),
        );

        assert_eq!(declare.surface_text(), "Bounded has min, max");
    }

    #[test]
    fn declaration_surface_excludes_trait_implementation_members() {
        let declare = Declare::new(
            intern("Range"),
            SpanId::INVALID,
            DeclareValue::Has(vec![
                HasMember::Property(HasProperty {
                    qualifier: None,
                    name: intern("start"),
                    name_span: SpanId::INVALID,
                    ty: Some(Box::new(spanned_expr(expr_from_type(nominal("x"))))),
                    body: None,
                    doc_comment: None,
                    refinement: None,
                }),
                HasMember::Property(HasProperty {
                    qualifier: Some(HasMemberQualifier {
                        name: intern("Bounded"),
                        span: SpanId::INVALID,
                    }),
                    name: intern("min"),
                    name_span: SpanId::INVALID,
                    ty: None,
                    body: Some(HasMemberBody::Overrideable(BindValue::Expr(Box::new(
                        Typed::infer(Expr::AnonymousTag(intern("start")), SpanId::INVALID),
                    )))),
                    doc_comment: None,
                    refinement: None,
                }),
            ]),
        );

        assert_eq!(declare.surface_text(), "Range has start x");
    }

    #[test]
    fn declaration_surface_has_members_uses_surface_types() {
        let declare = Declare::new(
            intern("Allocator"),
            SpanId::INVALID,
            DeclareValue::Has(vec![
                HasMember::Function(HasFunction {
                    qualifier: None,
                    name: intern("reserve"),
                    name_span: SpanId::INVALID,
                    params: vec![(
                        intern("l"),
                        Parameter::new(SpanId::INVALID, tagged(nominal("Layout"))),
                    )]
                    .into_iter()
                    .collect(),
                    conventions: Default::default(),
                    return_ty: Some(Box::new(spanned_expr(expr_from_type(generic(
                        "Slice",
                        vec![("Byte", nominal("Byte"))],
                    ))))),
                    error_ty: Some(Box::new(spanned_expr(expr_from_type(nominal("AllocError"))))),
                    body: None,
                    doc_comment: None,
                    refinement: None,
                    kind: HasFunctionKind::Instance,
                }),
                HasMember::Function(HasFunction {
                    qualifier: None,
                    name: intern("new"),
                    name_span: SpanId::INVALID,
                    params: vec![(
                        intern("size"),
                        Parameter::new(SpanId::INVALID, tagged(nominal("PointerSize"))),
                    )]
                    .into_iter()
                    .collect(),
                    conventions: Default::default(),
                    return_ty: Some(Box::new(spanned_expr(expr_from_type(nominal("Self"))))),
                    error_ty: None,
                    body: Some(HasMemberBody::Overrideable(BindValue::Expr(Box::new(
                        Typed::infer(Expr::AnonymousTag(intern("Self")), SpanId::INVALID),
                    )))),
                    doc_comment: None,
                    refinement: None,
                    kind: HasFunctionKind::Associated,
                }),
                HasMember::Property(HasProperty {
                    qualifier: None,
                    name: intern("length"),
                    name_span: SpanId::INVALID,
                    ty: Some(Box::new(spanned_expr(expr_from_type(generic(
                        "Pointer",
                        vec![("Byte", nominal("Byte"))],
                    ))))),
                    body: None,
                    doc_comment: None,
                    refinement: None,
                }),
            ]),
        );

        assert_eq!(
            declare.surface_text(),
            "Allocator has\n    reserve(l Layout) Slice(Byte) or AllocError\n    length Pointer(Byte)"
        );
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
