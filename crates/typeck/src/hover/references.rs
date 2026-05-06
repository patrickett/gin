use ast::HasSpanId;

/// Find all use-sites of `name` in the AST, returning byte ranges suitable for LSP locations.
///
/// Matches plain function calls, method calls (by last segment), bare tag references,
/// and tag constructor calls. Does not include the definition site itself.
pub fn find_references(ast: &ast::FileAst, name: &str) -> Vec<std::ops::Range<usize>> {
    let span_table = ast.span_table();
    let mut out = Vec::new();
    for (expr, _) in ast.top_level_exprs() {
        collect_refs_expr(expr, name, span_table, &mut out);
    }
    for bind in ast.defs().values() {
        collect_refs_bind_value(bind.value(), name, span_table, &mut out);
    }
    out
}

fn collect_refs_type_surface(
    expr: &ast::Expr,
    name: &str,
    span_table: &ast::SpanTable,
    out: &mut Vec<std::ops::Range<usize>>,
) {
    if let ast::Expr::TypeGeneric { params, .. } = expr {
        for (_, pk) in params {
            match pk {
                ast::ParameterKind::Default(e) => collect_refs_expr(&e.0, name, span_table, out),
                ast::ParameterKind::Tagged(sp) => {
                    collect_refs_type_surface(&sp.0, name, span_table, out);
                }
                ast::ParameterKind::Generic => {}
            }
        }
    }
}

fn collect_refs_expr(
    expr: &ast::Expr,
    name: &str,
    span_table: &ast::SpanTable,
    out: &mut Vec<std::ops::Range<usize>>,
) {
    use ast::Expr;
    match expr {
        Expr::FnCall(call) => {
            let call_span = span_table.get(call.path.span_id());
            if call.path.segments.is_empty() {
                if call.path.root.as_str() == name {
                    let s = call_span.start;
                    out.push(s..s + name.len());
                }
            } else if call
                .path
                .segments
                .last()
                .is_some_and(|seg| seg.as_str() == name)
            {
                let e = call_span.end;
                out.push(e - name.len()..e);
            }
            if let Some(args) = &call.args {
                for arg in args {
                    collect_refs_expr(&arg.0, name, span_table, out);
                }
            }
        }
        Expr::AnonymousTag(n, span_id) => {
            if n.as_str() == name {
                let span = span_table.get(*span_id);
                out.push(span.start..span.end);
            }
        }
        Expr::TagCall(tc) => {
            if tc.name.as_str() == name {
                let tc_span = span_table.get(tc.span_id());
                out.push(tc_span.start..tc_span.start + name.len());
            }
            for arg in &tc.args {
                collect_refs_expr(arg, name, span_table, out);
            }
        }
        Expr::Binary(bin) => {
            collect_refs_expr(&bin.lhs.0, name, span_table, out);
            collect_refs_expr(&bin.rhs.0, name, span_table, out);
        }
        Expr::Bind(bind) => collect_refs_bind_value(bind.value(), name, span_table, out),
        Expr::When(when_expr) => {
            if let Some(subject) = &when_expr.subject {
                collect_refs_expr(subject, name, span_table, out);
            }
            for arm in &when_expr.arms {
                match arm {
                    ast::WhenArm::Cond { condition, body } => {
                        collect_refs_expr(condition, name, span_table, out);
                        collect_refs_expr(body, name, span_table, out);
                    }
                    ast::WhenArm::Is { pattern, body } => {
                        collect_refs_expr(&pattern.0, name, span_table, out);
                        collect_refs_expr(body, name, span_table, out);
                    }
                    ast::WhenArm::Else(body) => {
                        collect_refs_expr(body, name, span_table, out);
                    }
                }
            }
        }
        Expr::If(if_expr) => {
            match &if_expr.condition {
                ast::IfCondition::Bool(e) => collect_refs_expr(&e.0, name, span_table, out),
                ast::IfCondition::Pattern { subject, pattern } => {
                    collect_refs_expr(&subject.0, name, span_table, out);
                    collect_refs_expr_type_surface(&pattern.0, name, span_table, out);
                }
            }
            for e in &if_expr.body {
                collect_refs_expr(e, name, span_table, out);
            }
        }
        Expr::Loop(loop_expr) => match loop_expr {
            ast::LoopEnum::ForIn(for_loop) => {
                collect_refs_expr(&for_loop.iter, name, span_table, out);
                for e in &for_loop.exprs {
                    collect_refs_expr(e, name, span_table, out);
                }
            }
            ast::LoopEnum::While(while_loop) => {
                collect_refs_expr(&while_loop.cond, name, span_table, out);
                for e in &while_loop.exprs {
                    collect_refs_expr(e, name, span_table, out);
                }
            }
        },
        Expr::FormatString(fs) => {
            for part in &fs.parts {
                if let ast::FormatPart::Expr(e) = part {
                    collect_refs_expr(e, name, span_table, out);
                }
            }
        }
        Expr::Range(range) => {
            collect_refs_expr(&range.start.0, name, span_table, out);
            collect_refs_expr(&range.end.0, name, span_table, out);
        }
        Expr::TupleAlloc { init, .. } => collect_refs_expr(init, name, span_table, out),
        Expr::TupleGet { base, .. } => collect_refs_expr(base, name, span_table, out),
        Expr::TupleSet { base, value, .. } => {
            collect_refs_expr(base, name, span_table, out);
            collect_refs_expr(value, name, span_table, out);
        }
        Expr::Cast { expr, .. } => collect_refs_expr(expr, name, span_table, out),
        Expr::BufGet { buf, index, .. } => {
            collect_refs_expr(buf, name, span_table, out);
            collect_refs_expr(index, name, span_table, out);
        }
        Expr::BufSet {
            buf, index, value, ..
        } => {
            collect_refs_expr(buf, name, span_table, out);
            collect_refs_expr(index, name, span_table, out);
            collect_refs_expr(value, name, span_table, out);
        }
        Expr::TakePtr(inner) | Expr::TakeRef(inner) | Expr::Deref(inner) | Expr::Negate(inner) => {
            collect_refs_expr(inner, name, span_table, out);
        }
        Expr::TupleLit(elems) => {
            for e in elems {
                collect_refs_expr(e, name, span_table, out);
            }
        }
        Expr::TypeGeneric { .. } => collect_refs_type_surface(expr, name, span_table, out),
        Expr::TypeNominal(..) | Expr::TypeQualified(_) => {}
        Expr::Lit(_) | Expr::SelfRef(_) | Expr::Asm(_) => {}
    }
}

/// Helper for If pattern case -- type surface doesn't have refs_expr, use type_surface version
fn collect_refs_expr_type_surface(
    expr: &ast::Expr,
    name: &str,
    span_table: &ast::SpanTable,
    out: &mut Vec<std::ops::Range<usize>>,
) {
    collect_refs_type_surface(expr, name, span_table, out);
}

fn collect_refs_bind_value(
    value: &ast::BindValue,
    name: &str,
    span_table: &ast::SpanTable,
    out: &mut Vec<std::ops::Range<usize>>,
) {
    match value {
        ast::BindValue::Expr(e) => collect_refs_expr(&e.0, name, span_table, out),
        ast::BindValue::Body { exprs, ret } => {
            for e in exprs {
                collect_refs_expr(&e.0, name, span_table, out);
            }
            if let Some(r) = &ret.0 {
                collect_refs_expr(&r.0, name, span_table, out);
            }
        }
        ast::BindValue::Extern => {}
    }
}
