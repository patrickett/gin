mod definition;
pub use definition::{dot_type_at, find_definition_span, find_import_definition_span};

mod references;
pub use references::find_references;

use crate::flow::{Bound, ConstValue, FlowAnalysis, TypeConstraint};
use crate::flow_analyzer::FlowAnalyzer;
use ast::{Bind, DeclareValue, ParameterKind, Parameters, Variant, format_type_surface};
use internment::Intern;

/// Return markdown hover text for the expression at `byte_pos` in the given AST.
/// Returns `None` if there is nothing hover-able at that position.
pub fn hover_at(source: &str, ast: &ast::FileAst, byte_pos: usize) -> Option<String> {
    // Try AST-based lookup first — find the expression at this position.
    if let Some((expr, _span_id)) = ast.expr_at_byte(byte_pos)
        && let ast::Expr::AnonymousTag(name, _) | ast::Expr::TypeNominal(name, _) = expr
    {
        let word = name.as_str();
        let ty_env = crate::TyEnv::from_file_ast(ast);
        let mut analyzer = FlowAnalyzer::new(&ty_env);
        analyzer.analyze_file(ast);
        let _flow = analyzer.into_result();

        // Look for tag definitions
        if let Some(decl) = ast.tags().get(name) {
            let mut result = format!("```gin\n{decl}\n```");
            if let Some(doc) = decl.doc_comment() {
                result.push_str(&format!("\n\n---\n\n{}", doc.0));
            }
            if let Some(ty) = ty_env.lookup_tag(*name) {
                result.push_str(&format!(
                    "\n\n---\n\nsize = {}, align = {}",
                    crate::ty_byte_size_static(ty),
                    crate::ty_alignment(ty),
                ));
            }
            return Some(result);
        }

        // Look for variant names
        if let Some(variant_hover) = hover_for_variant(ast, word) {
            return Some(variant_hover);
        }

        return Some(format!("```gin\n{word}\n```"));
    }

    // Fall back to word-based lookup for identifiers not captured by expr_at_byte
    // (e.g. function names, parameter names, body-level binds).
    let word = crate::source::word_at_byte_offset(source, byte_pos)?;
    let ty_env = crate::TyEnv::from_file_ast(ast);

    let mut analyzer = FlowAnalyzer::new(&ty_env);
    analyzer.analyze_file(ast);
    let flow = analyzer.into_result();

    // Look for tag definitions (also done above for expr_at_byte hits,
    // but needed here for when the cursor is on a tag name not wrapped in an Expr)
    for (name, decl) in ast.tags() {
        if name.as_str() == word {
            let mut result = format!("```gin\n{decl}\n```");
            if let Some(doc) = decl.doc_comment() {
                result.push_str(&format!("\n\n---\n\n{}", doc.0));
            }
            if let Some(ty) = ty_env.lookup_tag(*name) {
                result.push_str(&format!(
                    "\n\n---\n\nsize = {}, align = {}",
                    crate::ty_byte_size_static(ty),
                    crate::ty_alignment(ty),
                ));
            }
            return Some(result);
        }
    }

    // Look for variant names inside tag declarations (e.g. `Some`, `None` in `Maybe`)
    if let Some(variant_hover) = hover_for_variant(ast, &word) {
        return Some(variant_hover);
    }

    // Look for function definitions
    for (name, bind) in ast.defs() {
        if name.as_str() != word && bind.name().as_str() != word {
            continue;
        }
        let display_name = name.as_str().to_string();
        return Some(format_bind_hover(name, bind, &display_name, &ty_env, &flow));
    }

    // Look for parameter names across all defs
    for bind in ast.defs().values() {
        if let Some(params) = bind.params()
            && let Some(kind) = params.get(&internment::Intern::<String>::from_ref(&word))
        {
            let label = match kind {
                ParameterKind::Tagged(_) => format!("{word}{kind}"),
                ParameterKind::Default(expr) => format!("{word}: {expr:?}"),
                ParameterKind::Generic => word.clone(),
            };

            let narrowed = narrowed_at_position(ast, &flow, byte_pos, &word);

            if let Some(constraint) = &narrowed {
                let suffix = match constraint {
                    TypeConstraint::IsVariant(_, variant) => Some(variant.as_str().to_string()),
                    TypeConstraint::IsNotVariant(union, excluded) => {
                        if let Some(variants) = flow.union_to_variants.get(union) {
                            let remaining: Vec<_> =
                                variants.iter().filter(|v| *v != excluded).collect();
                            if remaining.len() == 1 {
                                Some(remaining[0].as_str().to_string())
                            } else if !remaining.is_empty() {
                                let names: Vec<String> =
                                    remaining.iter().map(|v| v.as_str().to_string()).collect();
                                Some(names.join(" or "))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    TypeConstraint::Compare { op, bound } => {
                        let bound_str = match bound {
                            Bound::Variable(name) => name.as_str().to_string(),
                            Bound::Constant(val) => val.to_hover_string(),
                        };
                        Some(format!("{} {}", op.symbol(), bound_str))
                    }
                };
                if let Some(suffix) = suffix {
                    return Some(format!("```gin\n{label} {suffix}\n```"));
                }
            }

            return Some(format!("```gin\n{label}\n```"));
        }
    }

    // Look for body-level binds inside function bodies
    if let Some(body_bind) = find_body_bind(ast, &word) {
        let narrowed = narrowed_at_position(ast, &flow, byte_pos, &word);
        let const_val = const_at_position(ast, &flow, byte_pos, &word);

        let mut result = format!("```gin\n{word}");
        match &narrowed {
            Some(TypeConstraint::IsVariant(_, variant)) => match &const_val {
                Some(ConstValue::Tag { name, .. }) if name == variant => {
                    result.push_str(&format!(
                        " {}",
                        const_val.as_ref().unwrap().to_hover_string()
                    ));
                }
                _ => {
                    result.push_str(&format!(" {}", variant.as_str()));
                }
            },
            Some(TypeConstraint::IsNotVariant(union, excluded)) => {
                if let Some(variants) = flow.union_to_variants.get(union) {
                    let remaining: Vec<_> = variants.iter().filter(|v| *v != excluded).collect();
                    if remaining.len() == 1 {
                        result.push_str(&format!(" {}", remaining[0].as_str()));
                    } else if !remaining.is_empty() {
                        let names: Vec<String> =
                            remaining.iter().map(|v| v.as_str().to_string()).collect();
                        result.push_str(&format!(" {}", names.join(" or ")));
                    }
                }
            }
            Some(TypeConstraint::Compare { op, bound }) => {
                let bound_str = match bound {
                    Bound::Variable(name) => name.as_str().to_string(),
                    Bound::Constant(val) => val.to_hover_string(),
                };
                result.push_str(&format!(" {} {}", op.symbol(), bound_str));
            }
            _ => {
                if let Some((type_name, args)) = &body_bind.type_annotation {
                    result.push_str(&format!(
                        " {}",
                        format_type_annotation(type_name.as_str(), args)
                    ));
                } else if let Some(ref cv) = const_val {
                    result.push_str(&format!(" {}", cv.to_hover_string()));
                }
            }
        }
        result.push_str("\n```");
        if let Some((type_name, _)) = &body_bind.type_annotation
            && let Some(ty) = ty_env.lookup_tag(*type_name)
        {
            result.push_str(&format!(
                "\n\n---\n\nsize = {}, align = {}",
                crate::ty_byte_size_static(ty),
                crate::ty_alignment(ty),
            ));
        }
        return Some(result);
    }

    {
        let narrowed = narrowed_at_position(ast, &flow, byte_pos, &word);
        let const_val = const_at_position(ast, &flow, byte_pos, &word);

        if let Some(TypeConstraint::Compare { op, bound }) = &narrowed {
            let bound_str = match bound {
                Bound::Variable(name) => name.as_str().to_string(),
                Bound::Constant(val) => val.to_hover_string(),
            };
            return Some(format!("```gin\n{word} {} {}\n```", op.symbol(), bound_str,));
        }

        if let Some(const_val) = const_val {
            return Some(format!(
                "```gin\n{word} {}\n```",
                const_val.to_hover_string()
            ));
        }
    }

    Some(format!("```gin\n{word}\n```"))
}

fn format_bind_hover(
    def_name: &Intern<String>,
    bind: &Bind,
    display_name: &str,
    ty_env: &crate::TyEnv,
    flow: &FlowAnalysis,
) -> String {
    let mut result = format!("```gin\n{}", display_name);
    if let Some(params) = bind.params() {
        result.push_str(&format_params(params));
    }
    if let Some(sp) = &bind.return_tag {
        result.push_str(&format!(" {}", format_type_surface(&sp.0)));
    }
    // For parameterless const binds, show the constant value.
    let is_function = bind.params().is_some();
    if !is_function && bind.is_const {
        let var = Intern::<String>::from_ref(display_name);
        if let Some(const_val) = flow.final_context.get_constant(&var) {
            result.push_str(&format!(" {}", const_val.to_hover_string()));
        }
    }
    result.push_str("\n```");
    if let Some(doc) = bind.doc_comment() {
        result.push_str(&format!("\n\n---\n\n{}", doc.0));
    }
    let mut meta_parts = Vec::new();
    if !is_function && let Some(ty) = ty_env.fn_return_ty(def_name) {
        meta_parts.push(format!("size = {}", crate::ty_byte_size_static(ty)));
        meta_parts.push(format!("align = {}", crate::ty_alignment(ty)));
    }
    if let Some(complexity) = bind.attributes().complexity.as_ref() {
        meta_parts.push(format!("complexity = {}", complexity.display_big_o()));
    }
    if !meta_parts.is_empty() {
        result.push_str(&format!("\n\n---\n\n{}", meta_parts.join(", ")));
    }
    result
}

/// Extract the base name from a variant's shape expression.
fn variant_base_name(expr: &ast::Expr) -> Option<&str> {
    match expr {
        ast::Expr::TypeGeneric { name, .. } => Some(name.as_str()),
        ast::Expr::TypeNominal(name, _) => Some(name.as_str()),
        ast::Expr::AnonymousTag(name, _) => Some(name.as_str()),
        _ => None,
    }
}

/// Look for a variant name inside any tag declaration's union variants.
/// If found, return hover text showing the parent tag and variant doc.
fn hover_for_variant(ast: &ast::FileAst, word: &str) -> Option<String> {
    for (tag_name, decl) in ast.tags() {
        let variants = match decl.value() {
            DeclareValue::Union { variants } => variants,
            _ => continue,
        };

        for variant in variants {
            let shape = variant.shape();
            let name = variant_base_name(&shape.0)?;
            if name != word {
                continue;
            }

            let mut result = format!("```gin\n{tag_name}\n\n{variant}\n```");

            if let Variant::Local { doc_comment, .. } = variant
                && let Some(doc) = doc_comment
            {
                result.push_str(&format!("\n\n---\n\n{}", doc.0));
            }

            return Some(result);
        }
    }

    None
}

/// Check if the word at `byte_pos` is a variant name inside a tag declaration.
/// If so, return `(variant_name, parent_tag_name)`.
pub fn is_variant_at(
    source: &str,
    ast: &ast::FileAst,
    byte_pos: usize,
) -> Option<(String, String)> {
    // Try AST-based lookup first.
    if let Some((expr, _span_id)) = ast.expr_at_byte(byte_pos)
        && let ast::Expr::AnonymousTag(name, _) | ast::Expr::TypeNominal(name, _) = expr
    {
        let word = name.as_str();
        for (tag_name, decl) in ast.tags() {
            let variants = match decl.value() {
                DeclareValue::Union { variants } => variants,
                _ => continue,
            };
            for variant in variants {
                let shape = variant.shape();
                let vname = variant_base_name(&shape.0)?;
                if vname == word {
                    return Some((word.to_string(), tag_name.as_str().to_string()));
                }
            }
        }
    }

    // Fall back to word-based lookup.
    let word = crate::source::word_at_byte_offset(source, byte_pos)?;
    for (tag_name, decl) in ast.tags() {
        let variants = match decl.value() {
            DeclareValue::Union { variants } => variants,
            _ => continue,
        };
        for variant in variants {
            let shape = variant.shape();
            let name = variant_base_name(&shape.0)?;
            if name == word {
                return Some((word, tag_name.as_str().to_string()));
            }
        }
    }
    None
}

/// Find the innermost (smallest) expression index whose span contains `byte_pos`.
fn innermost_expr_index(
    ast: &ast::FileAst,
    analysis: &FlowAnalysis,
    byte_pos: usize,
) -> Option<usize> {
    let span_table = ast.span_table();
    let mut best_idx: Option<usize> = None;
    let mut best_len = usize::MAX;

    for (&span_id, &idx) in &analysis.expr_spans {
        let span = span_table.get(span_id);
        if byte_pos >= span.start && byte_pos <= span.end {
            let len = span.end - span.start;
            if len < best_len {
                best_len = len;
                best_idx = Some(idx);
            }
        }
    }

    best_idx
}

/// Find the narrowed type constraint for `var_name` at a given byte position.
fn narrowed_at_position(
    ast: &ast::FileAst,
    analysis: &FlowAnalysis,
    byte_pos: usize,
    var_name: &str,
) -> Option<TypeConstraint> {
    let idx = innermost_expr_index(ast, analysis, byte_pos)?;
    analysis.narrowed_at(idx, var_name).cloned()
}

/// Find the known constant value for `var_name` at a given byte position.
fn const_at_position(
    ast: &ast::FileAst,
    analysis: &FlowAnalysis,
    byte_pos: usize,
    var_name: &str,
) -> Option<ConstValue> {
    let idx = innermost_expr_index(ast, analysis, byte_pos)?;
    for offset in 0..3 {
        if let Some(val) = analysis.value_at(idx + offset, var_name) {
            return Some(val.clone());
        }
    }
    None
}

/// Format a type annotation like `Maybe(3)` from its name and args.
fn format_type_annotation(type_name: &str, args: &[ast::Spanned<ast::Expr>]) -> String {
    if args.is_empty() {
        return type_name.to_string();
    }
    let parts: Vec<String> = args
        .iter()
        .map(|a| match &a.0 {
            ast::Expr::Lit(lit) => match lit {
                ast::Literal::Number(n) => n.to_string(),
                ast::Literal::Float(f) => f.to_string(),
                ast::Literal::Int(i) => i.to_string(),
                ast::Literal::String(s) => format!("\"{s}\""),
            },
            ast::Expr::FnCall(call) if call.args.is_none() => call.path.root.as_str().to_string(),
            other => format!("{other:?}"),
        })
        .collect();
    format!("{}({})", type_name, parts.join(", "))
}

fn format_params(params: &Parameters) -> String {
    crate::format_params(params)
}

/// Search all def bodies for a local bind matching `word`.
fn find_body_bind<'a>(ast: &'a ast::FileAst, word: &str) -> Option<&'a ast::Bind> {
    let key = Intern::<String>::from_ref(word);
    for bind in ast.defs().values() {
        if let Some(found) = search_bind_value(bind.value(), key) {
            return Some(found);
        }
    }
    None
}

fn search_bind_value(value: &ast::BindValue, name: Intern<String>) -> Option<&ast::Bind> {
    match value {
        ast::BindValue::Expr(e) => search_expr(&e.0, name),
        ast::BindValue::Body { exprs, ret } => {
            for e in exprs {
                if let Some(found) = search_expr(&e.0, name) {
                    return Some(found);
                }
            }
            if let Some(r) = &ret.0 {
                return search_expr(&r.0, name);
            }
            None
        }
        ast::BindValue::Extern => None,
    }
}

fn search_expr(expr: &ast::Expr, name: Intern<String>) -> Option<&ast::Bind> {
    use ast::Expr;
    match expr {
        Expr::Bind(bind) => {
            if bind.name() == name {
                return Some(bind);
            }
            search_bind_value(bind.value(), name)
        }
        Expr::If(if_expr) => {
            for e in &if_expr.body {
                if let Some(found) = search_expr(e, name) {
                    return Some(found);
                }
            }
            None
        }
        Expr::When(when_expr) => {
            if let Some(subject) = &when_expr.subject
                && let Some(found) = search_expr(subject, name)
            {
                return Some(found);
            }
            for arm in &when_expr.arms {
                match arm {
                    ast::WhenArm::Cond { condition, body } => {
                        if let Some(found) = search_expr(condition, name) {
                            return Some(found);
                        }
                        if let Some(found) = search_expr(body, name) {
                            return Some(found);
                        }
                    }
                    ast::WhenArm::Is { body, .. } | ast::WhenArm::Else(body) => {
                        if let Some(found) = search_expr(body, name) {
                            return Some(found);
                        }
                    }
                }
            }
            None
        }
        Expr::Loop(loop_enum) => match loop_enum {
            ast::LoopEnum::ForIn(for_loop) => {
                for e in &for_loop.exprs {
                    if let Some(found) = search_expr(&e.0, name) {
                        return Some(found);
                    }
                }
                None
            }
            ast::LoopEnum::While(while_loop) => {
                for e in &while_loop.exprs {
                    if let Some(found) = search_expr(&e.0, name) {
                        return Some(found);
                    }
                }
                None
            }
        },
        Expr::Binary(bin) => {
            search_expr(&bin.lhs.0, name).or_else(|| search_expr(&bin.rhs.0, name))
        }
        Expr::FnCall(call) => {
            if let Some(args) = &call.args {
                for arg in args {
                    if let Some(found) = search_expr(&arg.0, name) {
                        return Some(found);
                    }
                }
            }
            None
        }
        _ => None,
    }
}
