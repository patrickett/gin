use crate::util::format_params;
use ginc::ast::{Bind, Declare, DeclareValue, DocComment};

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

/// Build hover content for a type declaration using AST pretty-printing.
pub fn build_declare_hover(
    module: &str,
    decl: &Declare,
    doc: Option<&DocComment>,
) -> String {
    let mut md = format!("*{module}*\n\n");
    let block = format_declare_hover(decl);
    md.push_str(&format!("```gin\n{block}\n```\n"));
    if let Some(doc) = doc {
        md.push_str("\n---\n\n");
        md.push_str(&doc.0);
    }
    md
}

/// Build hover content for a variant within a union
pub fn build_variant_hover(
    module: &str,
    variant: &str,
    parent_tag: &str,
    doc: Option<&DocComment>,
) -> String {
    let mut result = format!(
        "*{module}.{parent_tag}*\n\n\
        ```gin\n\
        {variant}\n\
        ```"
    );

    if let Some(doc) = doc {
        result.push_str("\n\n---\n\n");
        result.push_str(&doc.0);
    }

    result
}

/// Build hover content for a local binding (no module header).
pub fn build_local_binding_hover(bind: &Bind) -> String {
    let mut md = format!("```gin\n{}", bind.name().as_str());

    if let Some(params) = bind.params() {
        md.push_str(&format_params(params));
    }

    if let Some(ret_type) = bind.infer_return_type() {
        md.push_str(&format!(" {ret_type}"));
    }

    md.push_str("\n```\n");

    if let Some(doc) = bind.doc_comment() {
        md.push_str("\n---\n\n");
        md.push_str(&doc.0);
    }

    md
}

/// Build hover content for a binding from the AST.
pub fn build_binding_hover(module: &str, bind: &Bind) -> String {
    let mut md = format!("*{module}*\n\n```gin\n{}", bind.name().as_str());

    if let Some(params) = bind.params() {
        md.push_str(&format_params(params));
    }

    if let Some(ret_type) = bind.infer_return_type() {
        md.push_str(&format!(" {ret_type}"));
    }

    md.push_str("\n```\n");

    if let Some(doc) = bind.doc_comment() {
        md.push_str("\n---\n\n");
        md.push_str(&doc.0);
    }

    md
}
