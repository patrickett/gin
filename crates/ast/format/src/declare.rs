use std::fmt::Write;

use crate::type_expr::{ExprFormatExt, PatternFormatExt};
use ast::declare::{Declare, DeclareValue, HasMember};
use ast::expr::Bind;
use ast::parameter::ParameterKind;

pub fn format_variant(variant: &ast::Variant) -> String {
    let mut surface = variant.shape().value.format_variant_shape();
    if let Some(discriminant) = variant.discriminant() {
        let _ = write!(surface, " := {}", discriminant.value);
    }
    surface
}

pub fn format_parameter_constraints(parameter: &ast::Parameter) -> String {
    parameter
        .constraints
        .iter()
        .map(|constraint| {
            let mut surface = constraint.trait_name.to_string();
            if !constraint.args.is_empty() {
                surface.push('(');
                surface.push_str(
                    &constraint
                        .args
                        .iter()
                        .map(|arg| arg.value.format_surface())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                surface.push(')');
            }
            surface
        })
        .collect::<Vec<_>>()
        .join(" and ")
}

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
                if let Some(pattern) = &v.range_pattern {
                    let _ = write!(&mut out, "{} to {}", pattern.start, pattern.end);
                    continue;
                }
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
                if !v.constraints.is_empty() {
                    let _ = write!(&mut out, " has {}", format_parameter_constraints(v));
                }
            }
            out.push(')');
        }
        let continuation_indent = out.len() + 1;
        out.push_str(&self.value.format_value(continuation_indent));
        out
    }
}

/// Extension trait providing [`Bind::signature_surface`].
pub trait BindFormatExt {
    /// Source-level signature for IDE hover (parameter and return type surfaces as written).
    fn signature_surface(&self) -> String;
}

fn push_named_results(sig: &mut String, bind: &Bind) {
    for (index, result) in bind.named_results.iter().enumerate() {
        if index > 0 {
            sig.push_str(", ");
        }
        if let Some(name) = result.name {
            sig.push_str(name.as_str());
            sig.push(' ');
        }
        sig.push_str(&result.ty.value.format_surface());
    }
}

fn push_return_tag(sig: &mut String, tag: &ast::Spanned<ast::Expr>, bind: &Bind) {
    sig.push_str(&payload_result_surface(
        &tag.value,
        &bind.payload_postconditions,
    ));
}

impl BindFormatExt for Bind {
    fn signature_surface(&self) -> String {
        let mut sig = self.name.as_str().to_string();
        if let Some(params) = &self.params {
            sig.push('(');
            let mut first = true;
            for (name, parameter) in params {
                if !first {
                    sig.push_str(", ");
                }
                first = false;
                let kind = &parameter.kind;
                match kind {
                    ParameterKind::Tagged(sp) => {
                        if let ast::Expr::Ref { inner, mutable, .. } = &sp.value {
                            sig.push_str(name.as_str());
                            sig.push(' ');
                            if *mutable {
                                sig.push_str("mut ");
                            } else {
                                sig.push_str("ref ");
                            }
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
                    ParameterKind::Proposition(_) | ParameterKind::Generic => {
                        sig.push_str(name.as_str());
                        match self.param_conventions.get(name) {
                            Some(ast::ParamConvention::Mutate) => sig.push_str(" mut"),
                            Some(ast::ParamConvention::Observe) => sig.push_str(" ref"),
                            Some(ast::ParamConvention::Own) => sig.push_str(" own"),
                            _ => {}
                        }
                        if let ParameterKind::Proposition(predicate) = kind {
                            let _ = write!(&mut sig, " is {predicate}");
                        }
                    }
                    ParameterKind::Default(expr) => {
                        sig.push_str(name.as_str());
                        sig.push_str(&format!(": {:?}", expr.value));
                    }
                }
                if !parameter.constraints.is_empty() {
                    let _ = write!(&mut sig, " has {}", format_parameter_constraints(parameter));
                }
            }
            sig.push(')');
            if !self.named_results.is_empty() {
                sig.push(' ');
                push_named_results(&mut sig, self);
            } else if let Some(sp) = &self.return_tag {
                sig.push(' ');
                push_return_tag(&mut sig, sp, self);
            }
        } else if !self.named_results.is_empty() {
            sig.push(' ');
            push_named_results(&mut sig, self);
        } else if let Some(sp) = &self.return_tag {
            sig.push(' ');
            push_return_tag(&mut sig, sp, self);
        } else if let Some(rtn) = &self.return_type_name {
            sig.push(' ');
            sig.push_str(rtn.as_str());
        }
        if !self.anonymous_result_alternatives.is_empty() {
            sig.push(' ');
            for (index, alternative) in self.anonymous_result_alternatives.iter().enumerate() {
                if index > 0 {
                    sig.push_str(" or ");
                }
                sig.push_str(alternative.value.label.as_str());
                if let Some(proposition) = &alternative.value.proposition {
                    let _ = write!(&mut sig, " thus {proposition}");
                }
            }
        }
        if self.return_tag.is_none()
            && let Some(ast::PredicateExpr::Proposition(predicate)) = &self.return_refinement
        {
            let _ = write!(&mut sig, " is {predicate}");
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
            DeclareValue::Alias(sp) => match &sp.value {
                ast::Expr::IntegerDomain(inner) => {
                    format!(" is in {}", inner.value.format_surface())
                }
                _ => format!(" is {}", sp.value.format_surface()),
            },
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
                    let mut out = format!(" is {}", format_variant(&variants[0]));
                    let indent = " ".repeat(continuation_indent);
                    for v in &variants[1..] {
                        let _ = write!(&mut out, "\n{indent}or {}", format_variant(v));
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
                    out.push_str(&format_variant(v));
                }
                out
            }

            DeclareValue::Has(members) => {
                let fields: Vec<_> = members
                    .iter()
                    .filter_map(|member| match member {
                        HasMember::Property(property)
                            if property.qualifier.is_none()
                                && !property.attributes.is_private() =>
                        {
                            Some(property)
                        }
                        _ => None,
                    })
                    .collect();
                if fields.is_empty() {
                    return String::new();
                }

                if fields.len() <= 3 {
                    let mut inline = String::new();
                    for (i, field) in fields.iter().enumerate() {
                        if i > 0 {
                            inline.push_str(", ");
                        }
                        inline.push_str(field.name.as_str());
                        if field.owned_storage {
                            inline.push_str(" own");
                        }
                        if let Some(ty) = &field.ty {
                            let _ = write!(&mut inline, " {}", ty.value.format_surface());
                        }
                    }
                    if continuation_indent + 4 + inline.len() <= 40 {
                        return format!(" has {inline}");
                    }
                }

                let mut out = " has".to_string();
                for field in fields {
                    let _ = write!(&mut out, "\n    {}", field.name.as_str());
                    if field.owned_storage {
                        out.push_str(" own");
                    }
                    if let Some(ty) = &field.ty {
                        out.push(' ');
                        out.push_str(&ty.value.format_surface());
                    }
                }
                out
            }
            DeclareValue::Set() => " is set".to_string(),
            DeclareValue::Range(start, end) => format!(" is {start} to {end}"),
            DeclareValue::InRange(start, end) => format!(" is in {start} to {end}"),
            DeclareValue::Refinement(predicate) => {
                match ast::integer::IntegerDomain::from_refinement(predicate).closed_bounds() {
                    Some((lower, upper)) => format!(" is in {lower} to {upper}"),
                    None => format!(" is {predicate:?}"),
                }
            }
        }
    }
}
#[cfg(test)]
#[path = "../../tests/declare_tests.rs"]
mod tests;

pub fn payload_result_surface(
    surface: &ast::Expr,
    contracts: &[ast::PayloadPostcondition],
) -> String {
    let mut pending = vec![surface];
    let mut alternatives = Vec::new();
    while let Some(surface) = pending.pop() {
        if let ast::Expr::Binary(binary) = surface
            && binary.op == ast::BinOp::LogicalOr
        {
            pending.push(&binary.rhs.value);
            pending.push(&binary.lhs.value);
            continue;
        }
        let ty = surface.format_surface();
        let contracts: Vec<_> = contracts
            .iter()
            .filter(|contract| contract.ty.value == *surface)
            .collect();
        alternatives.push(if let Some(contract) = contracts.first() {
            let propositions = contracts
                .iter()
                .map(|contract| contract.proposition.to_string())
                .collect::<Vec<_>>()
                .join(" and ");
            format!("{} {ty} thus {propositions}", contract.name)
        } else {
            ty
        });
    }
    alternatives.join(" or ")
}
