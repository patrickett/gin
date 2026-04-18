//! Hover functionality for the Gin language server.
//!
//! This module provides hover information and dot type completion
//! for the LSP.

// TODO: collapse common retype in hover messages
// some_func(request Request) should show some_func(r Request)
//
// TODO: return type inference from early-returning blocks
// - `main` → show `main 4 or Nothing` via return type inference from early-returning blocks

/// Return markdown hover text for the word at `byte_pos` in the given AST.
/// Returns `None` if there is nothing hover-able at that position.
pub fn hover_at(source: &str, ast: &ast::FileAst, byte_pos: usize) -> Option<String> {
    let word = crate::word_at_byte_offset(source, byte_pos)?;
    let ty_env = typeck::TyEnv::from_file_ast(ast);

    // Look for tag definitions
    for (name, decl) in ast.tags() {
        if name.as_str() == word {
            let mut result = format!("```gin\n{decl}\n```");
            if let Some(doc) = decl.doc_comment() {
                result.push_str(&format!("\n\n---\n\n{}", doc.0));
            }
            if let Some(ty) = ty_env.lookup_tag(*name) {
                result.push_str(&format!(
                    "\n\n---\n\nsize = {}, align = {}",
                    typeck::ty_byte_size_static(ty),
                    typeck::ty_alignment(ty),
                ));
            }
            return Some(result);
        }
    }

    // Look for function definitions
    for (name, bind) in ast.defs() {
        if name.as_str() != word {
            continue;
        }
        let mut result = format!("```gin\n{}", name.as_str());
        if let Some(params) = bind.params() {
            result.push_str(&crate::format_params(params));
        }
        if let Some(sp) = &bind.return_tag {
            if let Some(tag) = ast::type_tag_as_tag(&sp.0) {
                result.push_str(&format!(" {tag}"));
            }
        }
        result.push_str("\n```");
        if let Some(doc) = bind.doc_comment() {
            result.push_str(&format!("\n\n---\n\n{}", doc.0));
        }
        let mut meta_parts = Vec::new();
        let is_function = bind.params().is_some();
        if !is_function && let Some(ty) = ty_env.fn_return_ty(name) {
            meta_parts.push(format!("size = {}", typeck::ty_byte_size_static(ty)));
            meta_parts.push(format!("align = {}", typeck::ty_alignment(ty)));
        }
        if let Some(complexity) = bind.attributes().complexity.as_ref() {
            meta_parts.push(format!("complexity = {}", complexity.display_big_o()));
        }
        if !meta_parts.is_empty() {
            result.push_str(&format!("\n\n---\n\n{}", meta_parts.join(", ")));
        }
        return Some(result);
    }

    // Look for parameter names across all defs
    for bind in ast.defs().values() {
        if let Some(params) = bind.params()
            && let Some(kind) = params.get(&internment::Intern::<String>::from_ref(&word))
        {
            let label = match kind {
                ast::ParameterKind::Tagged(tag) => format!("{word} {tag}"),
                ast::ParameterKind::Default(expr) => format!("{word}: {expr:?}"),
                ast::ParameterKind::Generic => word.clone(),
            };

            // Check for flow narrowing at the cursor position (e.g. `num < 10` inside if body)
            let mut analyzer = typeck::FlowAnalyzer::new(&ty_env);
            analyzer.analyze_file(ast);
            let flow = analyzer.into_result();
            let narrowed = narrowed_at_position(ast, &flow, byte_pos, &word);

            if let Some(constraint) = &narrowed {
                let suffix = match constraint {
                    typeck::TypeConstraint::IsVariant(_, variant) => {
                        Some(variant.as_str().to_string())
                    }
                    typeck::TypeConstraint::IsNotVariant(union, excluded) => {
                        // Resolve to remaining variant(s), same as body binds
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
                    typeck::TypeConstraint::Compare { op, bound } => {
                        let bound_str = match bound {
                            typeck::Bound::Variable(name) => name.as_str().to_string(),
                            typeck::Bound::Constant(val) => val.to_hover_string(),
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
        // Run flow analysis to check for type narrowing and constant propagation
        let mut analyzer = typeck::FlowAnalyzer::new(&ty_env);
        analyzer.analyze_file(ast);
        let flow = analyzer.into_result();
        let narrowed = narrowed_at_position(ast, &flow, byte_pos, &word);
        let const_val = const_at_position(ast, &flow, byte_pos, &word);

        let mut result = format!("```gin\n{word}");
        match &narrowed {
            Some(typeck::TypeConstraint::IsVariant(_, variant)) => {
                // Show narrowed variant with payload from constant propagation
                // e.g. `val Some(3)` instead of just `val Some`
                match &const_val {
                    Some(typeck::ConstValue::Tag { name, .. }) if name == variant => {
                        result.push_str(&format!(
                            " {}",
                            const_val.as_ref().unwrap().to_hover_string()
                        ));
                    }
                    _ => {
                        result.push_str(&format!(" {}", variant.as_str()));
                    }
                }
            }
            Some(typeck::TypeConstraint::IsNotVariant(union, excluded)) => {
                // After an early-returning if, show the remaining variant(s)
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
            Some(typeck::TypeConstraint::Compare { op, bound }) => {
                // Show comparison constraint from while/if condition, e.g. `i < len`
                let bound_str = match bound {
                    typeck::Bound::Variable(name) => name.as_str().to_string(),
                    typeck::Bound::Constant(val) => val.to_hover_string(),
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
                    // No narrowing and no type annotation, but we know the value
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
                typeck::ty_byte_size_static(ty),
                typeck::ty_alignment(ty),
            ));
        }
        return Some(result);
    }

    // Look for in-scope variables with known constant values or comparison constraints
    // (e.g., pattern-extracted variables like `v` in `is Some(v)`,
    //  constant-folded values like `four` from `four: v + 1`,
    //  or comparison constraints like `i < len` from while conditions)
    {
        let mut analyzer = typeck::FlowAnalyzer::new(&ty_env);
        analyzer.analyze_file(ast);
        let flow = analyzer.into_result();
        let narrowed = narrowed_at_position(ast, &flow, byte_pos, &word);
        let const_val = const_at_position(ast, &flow, byte_pos, &word);

        // Comparison constraints take priority (e.g. `i < len` inside while)
        if let Some(typeck::TypeConstraint::Compare { op, bound }) = &narrowed {
            let bound_str = match bound {
                typeck::Bound::Variable(name) => name.as_str().to_string(),
                typeck::Bound::Constant(val) => val.to_hover_string(),
            };
            return Some(format!("```gin\n{word} {} {}\n```", op.symbol(), bound_str,));
        }

        // Then constant values (e.g. `v 3`)
        if let Some(const_val) = const_val {
            return Some(format!(
                "```gin\n{word} {}\n```",
                const_val.to_hover_string()
            ));
        }
    }

    Some(format!("```gin\n{word}\n```"))
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
) -> Option<internment::Intern<String>> {
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
            if call.args.is_none() && span_table.get(call.path.span).end == dot_pos =>
        {
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
    let span_table = ast.span_table();
    let key = internment::Intern::<String>::from_ref(name);
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

fn collect_refs_tag_pattern(
    tag: &ast::Tag,
    name: &str,
    span_table: &ast::SpanTable,
    out: &mut Vec<std::ops::Range<usize>>,
) {
    use ast::{ParameterKind, Tag};
    let Tag::Generic(_, params, _) = tag else {
        return;
    };
    for pk in params.values() {
        match pk {
            ParameterKind::Default(e) => collect_refs_expr(&e.0, name, span_table, out),
            ParameterKind::Tagged(inner) => collect_refs_tag_pattern(inner, name, span_table, out),
            ParameterKind::Generic => {}
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
            let call_span = span_table.get(call.path.span);
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
                let tc_span = span_table.get(tc.span);
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
                    if let Some(t) = ast::is_pattern_as_tag(&pattern.0) {
                        collect_refs_tag_pattern(t, name, span_table, out);
                    }
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
        Expr::IsPattern(t) | Expr::TypeTag(t) => collect_refs_tag_pattern(t, name, span_table, out),
        Expr::Lit(_) | Expr::SelfRef(_) | Expr::Asm(_) => {}
    }
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

/// Search all def bodies for a local bind matching `word`.
fn find_body_bind<'a>(ast: &'a ast::FileAst, word: &str) -> Option<&'a ast::Bind> {
    let key = internment::Intern::<String>::from_ref(word);
    for bind in ast.defs().values() {
        if let Some(found) = search_bind_value(bind.value(), key) {
            return Some(found);
        }
    }
    None
}

fn search_bind_value(
    value: &ast::BindValue,
    name: internment::Intern<String>,
) -> Option<&ast::Bind> {
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

fn search_expr(expr: &ast::Expr, name: internment::Intern<String>) -> Option<&ast::Bind> {
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

/// Find the innermost (smallest) expression index whose span contains `byte_pos`.
fn innermost_expr_index(
    ast: &ast::FileAst,
    analysis: &typeck::FlowAnalysis,
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
///
/// Searches all recorded expression spans and returns the constraint from
/// the smallest (most specific) span that contains `byte_pos`.
fn narrowed_at_position(
    ast: &ast::FileAst,
    analysis: &typeck::FlowAnalysis,
    byte_pos: usize,
    var_name: &str,
) -> Option<typeck::TypeConstraint> {
    let idx = innermost_expr_index(ast, analysis, byte_pos)?;
    analysis.narrowed_at(idx, var_name).cloned()
}

/// Find the known constant value for `var_name` at a given byte position.
///
/// Contexts are saved *before* expression analysis, so constants defined by a
/// bind expression (e.g. `four: v + 1`) only appear in later indices.  We check
/// the innermost span's index and the next two indices to cover this gap.
fn const_at_position(
    ast: &ast::FileAst,
    analysis: &typeck::FlowAnalysis,
    byte_pos: usize,
    var_name: &str,
) -> Option<typeck::ConstValue> {
    let idx = innermost_expr_index(ast, analysis, byte_pos)?;
    // The context at `idx` is saved before the expression is analysed.
    // For binds, the constant is recorded during analysis, so it only
    // appears in contexts saved for sub-expressions (idx+1, idx+2, …).
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
