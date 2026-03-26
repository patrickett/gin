use crate::util::format_params;
use ginc::ast::{Bind, Declare, DeclareValue, DocComment};
use ginc::typeck::{ty_alignment, ty_byte_size_static, Ty};
use ginc::DefMap;

/// Build hover content for a Gin language keyword.
pub fn build_keyword_hover(keyword: &str) -> Option<String> {
    let doc = match keyword {
        "is" => "Tag declaration keyword.\n\nDeclares a tag as a specific value or union.\n\n```gin\nColor is Red or Green or Blue\n```",
        "has" => "Struct field declaration keyword.\n\nDeclares that a tag has named fields.\n\n```gin\nPoint has x Float, y Float\n```",
        "or" => "Union variant separator.\n\nSeparates variants in a union tag declaration.\n\n```gin\nResult is Ok or Err\n```",
        "return" => "Return a value from a function.",
        ":" => "Binding operator.\n\nAnnotates a value with a tag or binds a default parameter value.",
        "if" => "Conditional expression.",
        "for" => "For-in loop.",
        "while" => "While loop.",
        "when" => "Pattern matching expression.",
        "in" => "Range or iteration operator.",
        "as" => "Type cast operator.",
        "not" => "Logical negation.",
        "use" => "Import declaration.",
        _ => return None,
    };

    Some(format!("```gin\n{keyword}\n```\n\n---\n\n{doc}"))
}

/// Pretty-print a declaration from the AST for hover display.
/// Handles multiline formatting for unions with ≥3 variants or >80 chars.
fn format_declare_hover(decl: &Declare) -> String {
    // Build the LHS: "Name" or "Name(params)"
    let mut lhs = decl.name().as_str().to_string();
    if let Some(params) = decl.params() {
        lhs.push('(');
        let mut first = true;
        for (k, v) in params {
            if !first {
                lhs.push_str(", ");
            }
            first = false;
            lhs.push_str(k.as_str());
            lhs.push_str(&v.to_string());
        }
        lhs.push(')');
    }

    match decl.value() {
        DeclareValue::Union { variants } => {
            let single_line = format!("{lhs} is {}", decl.value());
            if variants.len() <= 2 && single_line.len() <= 80 {
                single_line
            } else {
                let mut lines = vec![format!("{lhs} is")];
                for (i, v) in variants.iter().enumerate() {
                    let suffix = if i < variants.len() - 1 { " or" } else { "" };
                    lines.push(format!("    {v}{suffix}"));
                }
                lines.join("\n")
            }
        }
        _ => format!("{decl}"),
    }
}

/// Check if a type contains any Opaque (generic) fields.
fn contains_opaque(ty: &Ty) -> bool {
    match ty {
        Ty::Opaque(_) => true,
        Ty::Record { fields, .. } => fields.iter().any(|(_, t)| contains_opaque(t)),
        Ty::Union { variants, .. } => variants
            .iter()
            .any(|(_, fields)| fields.iter().any(|(_, t)| contains_opaque(t))),
        Ty::Array { elem, .. } | Ty::Ptr { inner: elem } | Ty::Ref { inner: elem } => {
            contains_opaque(elem)
        }
        Ty::Tuple(fields) => fields.iter().any(contains_opaque),
        _ => false,
    }
}

/// Build hover content for a type declaration using AST pretty-printing.
pub fn build_declare_hover(
    module: &str,
    decl: &Declare,
    doc: Option<&DocComment>,
    ty: Option<&Ty>,
) -> String {
    let mut md = format!("*{module}*\n\n");
    let block = format_declare_hover(decl);
    md.push_str(&format!("```gin\n{block}\n```\n"));

    if let Some(ty) = ty {
        // Skip layout for Opaque (unresolved) types and types containing generics (Opaque fields)
        if !matches!(ty, Ty::Opaque(_)) && !contains_opaque(ty) {
            let size = ty_byte_size_static(ty);
            let align = ty_alignment(ty);
            md.push_str(&format!(
                "\n---\n\nsize = {size} ({size:#x}), align = {align:#x}\n"
            ));
        }
    }

    if let Some(doc) = doc {
        md.push_str("\n---\n\n");
        md.push_str(&doc.0);
    }
    md
}

/// Build hover content for a variant within a union
pub fn build_variant_hover(module: &str, variant: &str, doc: Option<&DocComment>) -> String {
    let mut result = format!("*{module}*\n\n```gin\n{variant}\n```");

    if let Some(doc) = doc {
        result.push_str("\n\n---\n\n");
        result.push_str(&doc.0);
    }

    result
}

/// Build hover content for `self` within a method body.
///
/// Shows `self TypeName` (with params if the type has them, e.g. `self Maybe(x)`).
pub fn build_self_hover(receiver_name: &str, decl: Option<&Declare>) -> String {
    let type_str = if let Some(d) = decl {
        if let Some(params) = d.params() {
            if params.is_empty() {
                receiver_name.to_string()
            } else {
                let param_names: Vec<String> = params.keys().map(|k| k.to_string()).collect();
                format!("{}({})", receiver_name, param_names.join(", "))
            }
        } else {
            receiver_name.to_string()
        }
    } else {
        receiver_name.to_string()
    };
    format!("```gin\nself {type_str}\n```")
}

/// Build hover content for a local binding with optional narrowed type and AST for type lookup.
pub fn build_local_binding_hover_with_narrowing_and_ast(
    bind: &Bind,
    narrowed_type: Option<&str>,
    ast: Option<&ginc::ast::FileAst>,
) -> String {
    let mut md = format!("```gin\n{}", bind.name().as_str());

    if let Some(params) = bind.params() {
        md.push_str(&format_params(params));
    }

    // Show narrowed type if available, otherwise show explicit annotation or inferred type
    if let Some(narrowed) = narrowed_type {
        md.push_str(&format!(" {narrowed}"));
    } else if let Some((type_name, args)) = bind.type_annotation.as_ref() {
        // Explicit annotation like `val Maybe(3): Some(3)` → show `Maybe(3)`
        let args_str: Vec<String> = args.iter().map(display_lit_expr).collect();
        md.push_str(&format!(" {}({})", type_name.as_str(), args_str.join(", ")));
    } else if let Some(ast) = ast {
        if let Some(inferred) = infer_type_from_bind_with_ast(bind, ast) {
            md.push_str(&format!(" {inferred}"));
        } else if let Some(inferred_type) = bind.infer_return_type() {
            md.push_str(&format!(" {inferred_type}"));
        }
    } else if let Some(inferred_type) = bind.infer_return_type() {
        md.push_str(&format!(" {inferred_type}"));
    }

    md.push_str("\n```\n");

    if let Some(doc) = bind.doc_comment() {
        md.push_str("\n---\n\n");
        md.push_str(&doc.0);
    }

    md
}

fn display_lit_expr(expr: &ginc::ast::Expr) -> String {
    use ginc::ast::{Expr, Literal};
    match expr {
        Expr::Lit(Literal::Int(n)) => n.to_string(),
        Expr::Lit(Literal::Number(n)) => n.to_string(),
        Expr::Lit(Literal::Float(f)) => f.to_string(),
        Expr::Lit(Literal::String(s)) => format!("\"{}\"", s),
        _ => "_".to_string(),
    }
}

/// Infer the type of a bind's value using the AST for type lookup.
fn infer_type_from_bind_with_ast(bind: &Bind, ast: &ginc::ast::FileAst) -> Option<String> {
    use ginc::ast::{BindValue, DeclareValue, Expr, Literal, Tag};

    let expr = match bind.value() {
        BindValue::Expr(e) => Some(e.as_ref()),
        BindValue::Body { ret, .. } => ret.0.as_ref().map(|e| e.as_ref()),
        BindValue::Extern => return None,
    };

    let expr = expr?;

    // For TagCall, look up the parent union type and substitute literal args.
    if let Expr::TagCall(tc) = expr {
        // Search for a union that contains this variant
        for decl in ast.tags().values() {
            if let DeclareValue::Union { variants } = decl.value() {
                for variant in variants {
                    let tag = variant.tag();
                    if tag.name() == tc.name.as_str() {
                        // Build the union's display params by substituting TagCall args
                        // into the union's type parameters via the variant's param names.
                        let params_str = if let Some(union_params) = decl.params() {
                            if union_params.is_empty() {
                                String::new()
                            } else {
                                // Map variant param names → TagCall arg index
                                let variant_param_order: Vec<&str> = match tag {
                                    Tag::Generic(_, vparams) => {
                                        vparams.keys().map(|k| k.as_str()).collect()
                                    }
                                    Tag::Nominal(_) => vec![],
                                    Tag::Qualified(_) => vec![],
                                };
                                let args: Vec<String> = union_params
                                    .keys()
                                    .map(|uparam| {
                                        // Find this union param in the variant params
                                        let idx = variant_param_order
                                            .iter()
                                            .position(|&vp| vp == uparam.as_str());
                                        let arg_expr = idx.and_then(|i| tc.args.get(i));
                                        match arg_expr {
                                            Some(Expr::Lit(Literal::Int(n))) => n.to_string(),
                                            Some(Expr::Lit(Literal::Number(n))) => n.to_string(),
                                            Some(Expr::Lit(Literal::Float(f))) => f.to_string(),
                                            _ => uparam.to_string(),
                                        }
                                    })
                                    .collect();
                                format!("({})", args.join(", "))
                            }
                        } else {
                            String::new()
                        };
                        return Some(format!("{}{}", decl.name(), params_str));
                    }
                }
            }
        }
        // Fallback: just show the variant name if no parent found
        return Some(tc.name.to_string());
    }

    None
}

/// Build hover content for a binding from the AST.
///
/// `ret_type_override` replaces the inferred return type when `Some`.
pub fn build_binding_hover(
    module: &str,
    bind: &Bind,
    defs: &DefMap,
    ret_type_override: Option<String>,
) -> String {
    let name = if let Some(recv) = bind.receiver_type() {
        format!("{}.{}", recv.name(), bind.name().as_str())
    } else {
        bind.name().as_str().to_string()
    };
    let mut md = format!("*{module}*\n\n```gin\n{name}");

    if let Some(params) = bind.params() {
        md.push_str(&format_params(params));
    }

    // Use override if provided, otherwise infer
    let ret_type = ret_type_override.or_else(|| bind.infer_return_type_union_with_defs(defs));
    if let Some(ret_type) = ret_type {
        md.push_str(&format!(" {ret_type}"));
    }

    md.push_str("\n```\n");

    if let Some(doc) = bind.doc_comment() {
        md.push_str("\n---\n\n");
        md.push_str(&doc.0);
    }

    md
}
