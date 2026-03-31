//! Hover functionality for the Gin language server.
//!
//! This module provides hover information and dot type completion
//! for the LSP.

/// Return markdown hover text for the word at `byte_pos` in the given AST.
/// Returns `None` if there is nothing hover-able at that position.
pub fn hover_at(source: &str, ast: &ast::FileAst, byte_pos: usize) -> Option<String> {
    let word = crate::word_at_byte_offset(source, byte_pos)?;

    // Look for tag definitions
    for (name, decl) in ast.tags() {
        if name.as_str() == word {
            return Some(format!("```gin\n{decl}\n```"));
        }
    }

    // Look for function definitions
    for (name, bind) in ast.defs() {
        if name.as_str() == word {
            let mut result = format!("```gin\n{}", name.as_str());
            if let Some(params) = bind.params() {
                result.push_str(&crate::format_params(params));
            }
            result.push_str("\n```");
            return Some(result);
        }
    }

    None
}

/// Return the union type reachable via a dot expression at `byte_pos`.
pub fn dot_type_at(
    source: &str,
    ast: &ast::FileAst,
    ty_env: &typeck::TyEnv,
    byte_pos: usize,
) -> Option<typeck::Ty> {
    // Check if cursor is right after a dot
    let dot_pos = byte_pos.checked_sub(1)?;
    if source.as_bytes().get(dot_pos) != Some(&b'.') {
        return None;
    }

    // Find the name before the dot using the AST
    let name = find_name_before_dot(ast, dot_pos)?;
    ty_env.resolve_dot_type(ast, name)
}

fn find_name_before_dot(ast: &ast::FileAst, dot_pos: usize) -> Option<internment::Intern<String>> {
    for bind in ast.defs().values() {
        if let ast::BindValue::Body { exprs, ret } = bind.value() {
            for expr in exprs {
                if let Some(name) = find_name_in_expr(expr, dot_pos) {
                    return Some(name);
                }
            }
            if let Some(ret_expr) = ret.0.as_ref()
                && let Some(name) = find_name_in_expr(ret_expr, dot_pos)
            {
                return Some(name);
            }
        }
    }
    None
}

fn find_name_in_expr(
    expr: &ast::Spanned<ast::Expr>,
    dot_pos: usize,
) -> Option<internment::Intern<String>> {
    use ast::Expr;

    let span = expr.1;
    if span.end < dot_pos || span.start > dot_pos {
        return None;
    }

    match &expr.0 {
        Expr::AnonymousTag(name, inner_span) if inner_span.end == dot_pos => Some(*name),
        Expr::FnCall(call) if call.args.is_none() && call.path.span.end == dot_pos => {
            Some(call.path.root)
        }
        _ => None,
    }
}

/// Return the byte range of `name` at its definition site, using the parsed AST.
///
/// For defs, uses the `name_span` recorded during parsing.
/// For tags, uses the `name_span` recorded during parsing.
/// Method defs are matched by their unqualified name (e.g. `"foo"` matches `"Point.foo"`).
pub fn find_definition_span(ast: &ast::FileAst, name: &str) -> Option<std::ops::Range<usize>> {
    let key = internment::Intern::<::std::string::String>::new(name.to_string());
    if let Some(decl) = ast.tags().get(&key) {
        return Some(decl.name_span.start..decl.name_span.end);
    }
    ast.defs()
        .iter()
        .find(|(k, _)| {
            let s = k.as_str();
            s == name || (s.contains('.') && s.split('.').next_back() == Some(name))
        })
        .map(|(_, bind)| bind.name_span.start..bind.name_span.end)
}

/// Find all use-sites of `name` in the AST, returning byte ranges suitable for LSP locations.
///
/// Matches plain function calls, method calls (by last segment), bare tag references,
/// and tag constructor calls. Does not include the definition site itself.
pub fn find_references(ast: &ast::FileAst, name: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    for (expr, _) in ast.top_level_exprs() {
        collect_refs_expr(expr, name, &mut out);
    }
    for bind in ast.defs().values() {
        collect_refs_bind_value(bind.value(), name, &mut out);
    }
    out
}

fn collect_refs_expr(expr: &ast::Expr, name: &str, out: &mut Vec<std::ops::Range<usize>>) {
    use ast::Expr;
    match expr {
        Expr::FnCall(call) => {
            if call.path.segments.is_empty() {
                if call.path.root.as_str() == name {
                    let s = call.path.span.start;
                    out.push(s..s + name.len());
                }
            } else if call
                .path
                .segments
                .last()
                .is_some_and(|seg| seg.as_str() == name)
            {
                let e = call.path.span.end;
                out.push(e - name.len()..e);
            }
            if let Some(args) = &call.args {
                for arg in args {
                    collect_refs_expr(&arg.0, name, out);
                }
            }
        }
        Expr::AnonymousTag(n, span) => {
            if n.as_str() == name {
                out.push(span.start..span.end);
            }
        }
        Expr::TagCall(tc) => {
            if tc.name.as_str() == name {
                out.push(tc.span.start..tc.span.start + name.len());
            }
            for arg in &tc.args {
                collect_refs_expr(arg, name, out);
            }
        }
        Expr::Binary(bin) => {
            collect_refs_expr(&bin.lhs.0, name, out);
            collect_refs_expr(&bin.rhs.0, name, out);
        }
        Expr::Bind(bind) => collect_refs_bind_value(bind.value(), name, out),
        Expr::When(when_expr) => {
            if let Some(subject) = &when_expr.subject {
                collect_refs_expr(subject, name, out);
            }
            for arm in &when_expr.arms {
                match arm {
                    ast::WhenArm::Cond { condition, body } => {
                        collect_refs_expr(condition, name, out);
                        collect_refs_expr(body, name, out);
                    }
                    ast::WhenArm::Is { body, .. } | ast::WhenArm::Else(body) => {
                        collect_refs_expr(body, name, out);
                    }
                }
            }
        }
        Expr::If(if_expr) => {
            for e in &if_expr.body {
                collect_refs_expr(e, name, out);
            }
        }
        Expr::Loop(loop_expr) => match loop_expr {
            ast::LoopEnum::ForIn(for_loop) => {
                collect_refs_expr(&for_loop.iter, name, out);
                for e in &for_loop.exprs {
                    collect_refs_expr(e, name, out);
                }
            }
            ast::LoopEnum::While(while_loop) => {
                collect_refs_expr(&while_loop.cond, name, out);
                for e in &while_loop.exprs {
                    collect_refs_expr(e, name, out);
                }
            }
        },
        Expr::FormatString(fs) => {
            for part in &fs.parts {
                if let ast::FormatPart::Expr(e) = part {
                    collect_refs_expr(e, name, out);
                }
            }
        }
        Expr::Range(range) => {
            collect_refs_expr(&range.start.0, name, out);
            collect_refs_expr(&range.end.0, name, out);
        }
        Expr::TupleAlloc { init, .. } => collect_refs_expr(init, name, out),
        Expr::TupleGet { base, .. } => collect_refs_expr(base, name, out),
        Expr::TupleSet { base, value, .. } => {
            collect_refs_expr(base, name, out);
            collect_refs_expr(value, name, out);
        }
        Expr::Cast { expr, .. } => collect_refs_expr(expr, name, out),
        Expr::BufGet { buf, index, .. } => {
            collect_refs_expr(buf, name, out);
            collect_refs_expr(index, name, out);
        }
        Expr::BufSet {
            buf, index, value, ..
        } => {
            collect_refs_expr(buf, name, out);
            collect_refs_expr(index, name, out);
            collect_refs_expr(value, name, out);
        }
        Expr::TakePtr(inner) | Expr::TakeRef(inner) | Expr::Deref(inner) | Expr::Negate(inner) => {
            collect_refs_expr(inner, name, out);
        }
        Expr::TupleLit(elems) => {
            for e in elems {
                collect_refs_expr(e, name, out);
            }
        }
        Expr::Lit(_) | Expr::SelfRef(_) => {}
    }
}

fn collect_refs_bind_value(
    value: &ast::BindValue,
    name: &str,
    out: &mut Vec<std::ops::Range<usize>>,
) {
    match value {
        ast::BindValue::Expr(e) => collect_refs_expr(&e.0, name, out),
        ast::BindValue::Body { exprs, ret } => {
            for e in exprs {
                collect_refs_expr(&e.0, name, out);
            }
            if let Some(r) = &ret.0 {
                collect_refs_expr(&r.0, name, out);
            }
        }
        ast::BindValue::Extern => {}
    }
}
