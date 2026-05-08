use ast::HasSpanId;
use internment::Intern;

/// Return the byte range of `name` at its definition site, using the parsed AST.
///
/// For defs, uses the `name_span` recorded during parsing.
/// For tags, uses the `name_span` recorded during parsing.
/// Method defs are matched by their unqualified name (e.g. `"foo"` matches `"Point.foo"`).
pub fn find_definition_span(ast: &ast::FileAst, name: &str) -> Option<std::ops::Range<usize>> {
    let span_table = ast.span_table();
    let key = Intern::<String>::from_ref(name);
    if let Some(decl) = ast.tags().get(&key) {
        let span = span_table.get(decl.name_span);
        return Some(span.start..span.end);
    }
    ast.defs()
        .iter()
        .find(|(k, _)| {
            let s = k.as_str();
            s == name || (s.contains('.') && s.split('.').next_back() == Some(name))
        })
        .map(|(_, bind)| {
            let span = span_table.get(bind.name_span);
            span.start..span.end
        })
}

/// Byte range of the `use` path text that introduces `name` into scope (package last segment /
/// root-only path, local basename, bundle root, or `as` alias), for goto-definition before jumping
/// off-file to the dependency.
///
/// For package imports with segments (e.g. `use core.println`), returns the span of just the
/// last segment (`println`) rather than the whole dotted path.
///
/// Returns `None` when `name` is not introduced by any `use` in this file.
pub fn find_import_definition_span(
    ast: &ast::FileAst,
    name: &str,
    source: &str,
) -> Option<std::ops::Range<usize>> {
    let span_table = ast.span_table();
    let key = Intern::<String>::from_ref(name);
    for imp in ast.uses() {
        for mi in &imp.0 {
            let imported = mi
                .alias
                .unwrap_or_else(|| Intern::<String>::new(mi.effective_name()));
            if imported != key {
                continue;
            }

            // For Package imports with segments, return the span of the last segment only.
            if let ast::ImportSource::Package(mod_path) = &mi.source
                && let Some(last_seg) = mod_path.segments.last()
            {
                let path_span = span_table.get(mod_path.span_id());
                let path_text = source.get(path_span.start..path_span.end)?;
                // Find the last '.' in the path text; the last segment starts after it.
                let dot_pos = path_text.rfind('.')?;
                let seg_start = path_span.start + dot_pos + 1;
                let seg_end = seg_start + last_seg.len();
                return Some(seg_start..seg_end);
            }

            // Non-package or root-only: use the full source span.
            let span = span_table.get(mi.source.span_id());
            return Some(span.start..span.end);
        }
    }
    None
}

/// Return the union type reachable via a dot expression at `byte_pos`.
pub fn dot_type_at(
    source: &str,
    ast: &ast::FileAst,
    ty_env: &crate::TyEnv,
    byte_pos: usize,
) -> Option<crate::Ty> {
    // Check if cursor is right after a dot
    let dot_pos = byte_pos.checked_sub(1)?;
    if source.as_bytes().get(dot_pos) != Some(&b'.') {
        return None;
    }

    // Find the name before the dot using the AST
    let name = find_name_before_dot(ast, dot_pos)?;
    ty_env.resolve_dot_type(ast, name)
}

fn find_name_before_dot(ast: &ast::FileAst, dot_pos: usize) -> Option<Intern<String>> {
    let span_table = ast.span_table();
    for bind in ast.defs().values() {
        if let ast::BindValue::Body { exprs, ret } = bind.value() {
            for expr in exprs {
                if let Some(name) = find_name_in_expr(expr, dot_pos, span_table) {
                    return Some(name);
                }
            }
            if let Some(ret_expr) = ret.0.as_ref()
                && let Some(name) = find_name_in_expr(ret_expr, dot_pos, span_table)
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
    span_table: &ast::SpanTable,
) -> Option<Intern<String>> {
    use ast::Expr;

    let span = span_table.get(expr.1);
    if span.end < dot_pos || span.start > dot_pos {
        return None;
    }

    match &expr.0 {
        Expr::AnonymousTag(name, inner_span) if span_table.get(*inner_span).end == dot_pos => {
            Some(*name)
        }
        Expr::FnCall(call)
            if call.args.is_none() && span_table.get(call.path.span_id()).end == dot_pos =>
        {
            Some(call.path.root)
        }
        _ => None,
    }
}
