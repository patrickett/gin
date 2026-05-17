//! Hover formatting — colocated markdown generation from a resolved [`FileAst`] and [`TypedFileAst`].

use crate::format_type_surface;
use crate::{Bound, ConstValue, FlowAnalysis, HasSpanId, TypeConstraint};
use internment::Intern;
use std::collections::HashMap;

/// Compute hover markdown at `byte_pos` using a pre-built [`FlowAnalysis`].
pub fn hover_at_with_flow(
    source: &str,
    ast: &crate::FileAst,
    tag_types: Option<&HashMap<crate::typed::TagId, crate::ty::Ty>>,
    flow: &FlowAnalysis,
    byte_pos: usize,
    source_name: Option<&str>,
) -> Option<String> {
    let word = ast
        .word_at_byte(byte_pos, source)
        .or_else(|| word_at_byte_offset(source, byte_pos))?;

    let tag_key = Intern::<String>::from_ref(&word);
    if let Some(decl) = ast.tags().get(&tag_key) {
        let mut result = String::new();
        if let Some(sn) = source_name {
            result.push_str(&format!("`{sn}`\n\n"));
        }
        result.push_str(&format!("```gin\n{decl}\n```"));
        if let Some(doc) = decl.doc_comment() {
            result.push_str(&format!("\n\n---\n\n{}", doc.value));
        }
        // Show size/align when tag_types map is available.
        if let Some(tag_types) = tag_types {
            let tid = crate::typed::TagId(tag_key);
            if let Some(ty) = tag_types.get(&tid) {
                let size = crate::ty::ty_byte_size_static(ty);
                let align = crate::ty::ty_alignment(ty);
                result.push_str(&format!("\n\n---\n\nsize = {size}, align = {align}"));
            }
        }
        return Some(result);
    }

    // Check for variant matches by scanning all union variants.
    if let Some(variant_hover) = variant_hover_for_word(ast, &word) {
        return Some(variant_hover);
    }

    // Check function definitions.
    for (name, bind) in ast.defs() {
        if name.as_str() != word && bind.name().as_str() != word {
            continue;
        }
        let display_name = name.as_str().to_string();
        return Some(format_bind_hover(bind, &display_name, ast, flow));
    }

    // Check parameter names.
    if let Some(h) = hover_param(ast, flow, source, byte_pos, &word) {
        return Some(h);
    }

    // Check body binds with narrowing info.
    if let Some(h) = hover_body_bind(ast, flow, source, byte_pos, &word) {
        return Some(h);
    }

    // Generic narrowing / const-value.
    if let Some(h) = hover_narrowing(ast, flow, byte_pos, &word) {
        return Some(h);
    }

    Some(format!("```gin\n{word}\n```"))
}

/// If `word` is a variant name in any union, return the formatted hover.
fn variant_hover_for_word(ast: &crate::FileAst, word: &str) -> Option<String> {
    let variant_key = Intern::<String>::from_ref(word);
    for (tag_name, decl) in ast.tags() {
        use crate::DeclareValue;
        let variants = match decl.value() {
            DeclareValue::Union { variants } => variants,
            _ => continue,
        };
        for v in variants {
            if variant_name(&v.shape().value) == Some(variant_key) {
                let mut result = format!("```gin\n{tag_name}\n\n{v}\n```");
                if let crate::Variant::Local { doc_comment, .. } = v
                    && let Some(doc) = doc_comment
                {
                    result.push_str(&format!("\n\n---\n\n{}", doc.value));
                }
                return Some(result);
            }
        }
    }
    None
}

fn variant_name(expr: &crate::TypeExpr) -> Option<Intern<String>> {
    match expr {
        crate::TypeExpr::Generic { name, .. } => Some(*name),
        crate::TypeExpr::Nominal(name, _) => Some(*name),
        _ => None,
    }
}

fn hover_param(
    ast: &crate::FileAst,
    flow: &FlowAnalysis,
    _source: &str,
    pos: usize,
    word: &str,
) -> Option<String> {
    for bind in ast.defs().values() {
        let Some(params) = bind.params() else {
            continue;
        };
        if let Some(kind) = params.get(&Intern::<String>::from_ref(word)) {
            use crate::ParameterKind;
            let label = match kind {
                ParameterKind::Tagged(_) => format!("{word}{kind}"),
                ParameterKind::Default(expr) => format!("{word}: {expr:?}"),
                ParameterKind::Generic => word.to_string(),
            };

            let narrowed = narrowed_at_position(ast, flow, pos, word);

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
    None
}

fn hover_body_bind(
    ast: &crate::FileAst,
    flow: &FlowAnalysis,
    _source: &str,
    pos: usize,
    word: &str,
) -> Option<String> {
    let bind = find_body_bind(ast, word)?;
    let narrowed = narrowed_at_position(ast, flow, pos, word);
    let const_val = const_at_position(ast, flow, pos, word);
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
            if let Some((type_name, args)) = &bind.type_annotation {
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
    result.push_str("\n\n---\n\n");
    if let Some((type_name, _)) = &bind.type_annotation {
        result.push_str(&format!("type: {}\n", type_name.as_str()));
    }
    if let Some(ref cv) = const_val {
        result.push_str(&format!("const: {}\n", cv.to_hover_string()));
    }
    Some(result)
}

fn hover_narrowing(
    ast: &crate::FileAst,
    flow: &FlowAnalysis,
    pos: usize,
    word: &str,
) -> Option<String> {
    let narrowed = narrowed_at_position(ast, flow, pos, word);
    let const_val = const_at_position(ast, flow, pos, word);

    if let Some(TypeConstraint::Compare { op, bound }) = &narrowed {
        let bound_str = match bound {
            Bound::Variable(name) => name.as_str().to_string(),
            Bound::Constant(val) => val.to_hover_string(),
        };
        return Some(format!("```gin\n{word} {} {}\n```", op.symbol(), bound_str));
    }
    if let Some(cv) = const_val {
        return Some(format!("```gin\n{word} {}\n```", cv.to_hover_string()));
    }
    None
}

/// Find the definition span of a symbol by name.
pub fn definition_span(ast: &crate::FileAst, name: &str) -> Option<std::ops::Range<usize>> {
    let span_table = ast.span_table();
    let key = Intern::<String>::from_ref(name);
    if let Some(decl) = ast.tags().get(&key) {
        let span = span_table.get(decl.name_span);
        return Some(span.start..span.end);
    }
    ast.defs().iter().find_map(|(k, bind)| {
        let s = k.as_str();
        if s == name || (s.contains('.') && s.split('.').next_back() == Some(name)) {
            let span = span_table.get(bind.name_span);
            Some(span.start..span.end)
        } else {
            None
        }
    })
}

fn format_bind_hover(
    bind: &crate::Bind,
    display_name: &str,
    ast: &crate::FileAst,
    flow: &FlowAnalysis,
) -> String {
    let mut result = format!("```gin\n{display_name}");
    if let Some(params) = bind.params() {
        result.push_str(&format_params_pre(params).to_string());
    }
    let is_function = bind.params().is_some();
    if !is_function && bind.is_const {
        let var = Intern::<String>::from_ref(display_name);
        if let Some(const_val) = flow.final_context.get_constant(&var) {
            let is_single_variant = bind.return_tag.as_ref().is_some_and(|sp| match &sp.value {
                crate::TypeExpr::Nominal(name, _) => ast.tags().get(name).is_some_and(|decl| {
                    matches!(decl.value(), crate::DeclareValue::Union { variants } if variants.len() == 1)
                }),
                _ => false,
            });

            if is_single_variant {
                if let Some(sp) = &bind.return_tag
                    && let crate::TypeExpr::Nominal(name, _) = &sp.value
                    && let Some(decl) = ast.tags().get(name)
                    && let crate::DeclareValue::Union { variants } = decl.value()
                    && let Some(variant) = variants.first()
                {
                    result.push_str(&format!(" {variant}"));
                }
            } else {
                result.push_str(&format!(" {}", const_val.to_hover_string()));
            }
        } else if let Some(sp) = &bind.return_tag {
            result.push_str(&format!(" {}", format_type_surface(&sp.value)));
        }
    } else if let Some(sp) = &bind.return_tag {
        result.push_str(&format!(" {}", format_type_surface(&sp.value)));
    }
    result.push_str("\n```");
    if let Some(doc) = bind.doc_comment() {
        result.push_str(&format!("\n\n---\n\n{}", doc.value));
    }
    let mut meta_parts = Vec::new();
    if !is_function {
        // Size/align info not available without Analysis
    }
    if let Some(complexity) = bind.attributes().complexity.as_ref() {
        meta_parts.push(format!("complexity = {}", complexity.display_big_o()));
    }
    if !meta_parts.is_empty() {
        result.push_str(&format!("\n\n---\n\n{}", meta_parts.join(", ")));
    }
    result
}

fn format_params_pre(params: &crate::Parameters) -> String {
    use crate::ParameterKind;
    use std::fmt::Write;
    let mut s = String::from("(");
    let mut first = true;
    for (name, kind) in params.iter() {
        if !first {
            s.push_str(", ");
        }
        first = false;
        s.push_str(name.as_str());
        match kind {
            ParameterKind::Tagged(sp) => {
                s.push(' ');
                if let Some(te) = sp.value.as_type_expr() {
                    let _ = write!(s, "{}", format_type_surface(&te));
                }
            }
            ParameterKind::Default(expr) => {
                s.push_str(&format!(": {expr:?}"));
            }
            ParameterKind::Generic => {}
        }
    }
    s.push(')');
    s
}

fn format_type_annotation(type_name: &str, args: &[crate::Typed<crate::Expr>]) -> String {
    if args.is_empty() {
        return type_name.to_string();
    }
    let parts: Vec<String> = args
        .iter()
        .map(|a| match &a.value {
            crate::Expr::Lit(lit) => match lit {
                crate::Literal::Number(n) => n.to_string(),
                crate::Literal::Float(f) => f.to_string(),
                crate::Literal::Int(i) => i.to_string(),
                crate::Literal::String(s) => format!("\"{s}\""),
            },
            _ => format!("{:?}", &a.value),
        })
        .collect();
    format!("{}({})", type_name, parts.join(", "))
}

fn find_body_bind<'a>(ast: &'a crate::FileAst, word: &str) -> Option<&'a crate::Bind> {
    for bind in ast.defs().values() {
        if let crate::BindValue::Body { .. } = bind.value()
            && bind.name().as_str() == word
        {
            return Some(bind);
        }
    }
    None
}

fn innermost_expr_index(ast: &crate::FileAst, flow: &FlowAnalysis, pos: usize) -> Option<usize> {
    let sp = ast.span_table();
    let mut best: Option<usize> = None;
    let mut best_len = usize::MAX;
    for (&span_id, &idx) in &flow.expr_spans {
        let s = sp.get(span_id);
        if pos >= s.start && pos <= s.end {
            let len = s.end - s.start;
            if len < best_len {
                best_len = len;
                best = Some(idx);
            }
        }
    }
    best
}

fn narrowed_at_position(
    ast: &crate::FileAst,
    flow: &FlowAnalysis,
    pos: usize,
    word: &str,
) -> Option<TypeConstraint> {
    let idx = innermost_expr_index(ast, flow, pos)?;
    flow.narrowed_at(idx, word).cloned()
}

fn const_at_position(
    ast: &crate::FileAst,
    flow: &FlowAnalysis,
    pos: usize,
    word: &str,
) -> Option<ConstValue> {
    let idx = innermost_expr_index(ast, flow, pos)?;
    for offset in 0..3 {
        if let Some(val) = flow.value_at(idx + offset, word) {
            return Some(val.clone());
        }
    }
    None
}

/// Simple fallback to extract the identifier word at a byte offset from raw source text.
fn word_at_byte_offset(source: &str, byte_pos: usize) -> Option<String> {
    let bytes = source.as_bytes();
    if byte_pos >= bytes.len()
        || !(bytes[byte_pos] as char).is_alphanumeric() && bytes[byte_pos] != b'_'
    {
        return None;
    }
    let mut start = byte_pos;
    let mut end = byte_pos;
    while start > 0 {
        let c = bytes[start - 1] as char;
        if c.is_alphanumeric() || c == '_' {
            start -= 1;
        } else {
            break;
        }
    }
    while end < bytes.len() {
        let c = bytes[end] as char;
        if c.is_alphanumeric() || c == '_' {
            end += 1;
        } else {
            break;
        }
    }
    if start >= end {
        return None;
    }
    Some(source[start..end].to_string())
}

/// Convenience: hover using a pre-built [`TypedFileAst`] instead of a raw parse AST.
pub fn hover_at_with_source(
    source: &str,
    typed_ast: &crate::TypedFileAst,
    byte_pos: usize,
    source_name: Option<&str>,
) -> Option<String> {
    let (line, character) = crate::byte_offset_to_position(byte_pos, source);
    let hover_text = typed_ast.hover_at(source, line, character)?;
    match source_name {
        Some(name) => Some(format!("`{name}`\n\n{hover_text}")),
        None => Some(hover_text),
    }
}

/// Check whether the word at `byte_pos` is a variant name (for LSP semantic tokens).
pub fn is_variant_at(ast: &crate::FileAst, byte_pos: usize) -> Option<(String, String)> {
    if let Some((expr, _span_id)) = ast.expr_at_byte(byte_pos)
        && let crate::Expr::AnonymousTag(name) = expr
    {
        let word = name.as_str();
        for (tag_name, decl) in ast.tags() {
            use crate::DeclareValue;
            let variants = match decl.value() {
                DeclareValue::Union { variants } => variants,
                _ => continue,
            };
            for v in variants {
                if let Some(vn) = variant_name(&v.shape().value)
                    && vn.as_str() == word
                {
                    return Some((tag_name.to_string(), word.to_string()));
                }
            }
        }
    }
    None
}

/// Return the union type reachable via a dot expression at `byte_pos`.
pub fn dot_type_at(source: &str, ast: &crate::FileAst, byte_pos: usize) -> Option<crate::ty::Ty> {
    let dot_pos = byte_pos.checked_sub(1)?;
    if source.as_bytes().get(dot_pos) != Some(&b'.') {
        return None;
    }
    let name = find_name_before_dot(ast, dot_pos)?;
    resolve_dot_type(name)
}

fn resolve_dot_type(name: Intern<String>) -> Option<crate::ty::Ty> {
    // With Analysis, we would use analysis.tag_types. Without it, fall back
    // to nothing (dot_type_at users should use the Analysis-aware variant if available).
    let _ = name;
    None
}

fn find_name_before_dot(ast: &crate::FileAst, dot_pos: usize) -> Option<Intern<String>> {
    let span_table = ast.span_table();
    for bind in ast.defs().values() {
        if let crate::BindValue::Body { exprs, ret } = bind.value() {
            for expr in exprs {
                if let Some(name) = find_name_in_expr(expr, dot_pos, span_table) {
                    return Some(name);
                }
            }
            if let Some(ret_expr) = ret.value.as_ref()
                && let Some(name) = find_name_in_expr(ret_expr, dot_pos, span_table)
            {
                return Some(name);
            }
        }
    }
    None
}

fn find_name_in_expr(
    expr: &crate::Typed<crate::Expr>,
    dot_pos: usize,
    span_table: &crate::SpanTable,
) -> Option<Intern<String>> {
    use crate::Expr;
    let span = span_table.get(expr.span_id);
    if span.end < dot_pos || span.start > dot_pos {
        return None;
    }
    match &expr.value {
        Expr::AnonymousTag(name) if span_table.get(expr.span_id).end == dot_pos => Some(*name),
        Expr::FnCall(call)
            if call.args.is_none() && span_table.get(call.path.span_id()).end == dot_pos =>
        {
            Some(call.path.root)
        }
        _ => None,
    }
}

/// Byte range of the `use` path text that introduces `name` into scope.
pub fn find_import_definition_span(
    ast: &crate::FileAst,
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
            if let crate::ImportSource::Package(mod_path) = &mi.source
                && let Some(last_seg) = mod_path.segments.last()
            {
                let path_span = span_table.get(mod_path.span_id());
                let path_text = source.get(path_span.start..path_span.end)?;
                let dot_pos = path_text.rfind('.')?;
                let seg_start = path_span.start + dot_pos + 1;
                let seg_end = seg_start + last_seg.len();
                return Some(seg_start..seg_end);
            }
            if let crate::ImportSource::LocalBundle(b) = &mi.source {
                for member in &b.members {
                    let member_name = member.alias.unwrap_or(member.export);
                    if member_name == key {
                        let mspan = span_table.get(member.span);
                        return Some(mspan.start..mspan.end);
                    }
                }
            }
            if let crate::ImportSource::CurrentModule { member } = &mi.source
                && member.alias.unwrap_or(member.export) == key
            {
                let mspan = span_table.get(member.span);
                return Some(mspan.start..mspan.end);
            }
            let span = span_table.get(mi.source.span_id());
            return Some(span.start..span.end);
        }
    }
    None
}

/// Find all use-sites of `name` in the AST.
pub fn find_references(ast: &crate::FileAst, name: &str) -> Vec<std::ops::Range<usize>> {
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
    expr: &crate::TypeExpr,
    name: &str,
    span_table: &crate::SpanTable,
    out: &mut Vec<std::ops::Range<usize>>,
) {
    if let crate::TypeExpr::Generic { params, .. } = expr {
        for (_, pk) in params {
            match pk {
                crate::ParameterKind::Default(e) => {
                    collect_refs_expr(&e.value, name, span_table, out)
                }
                crate::ParameterKind::Tagged(sp) => {
                    if let Some(te) = sp.value.as_type_expr() {
                        collect_refs_type_surface(&te, name, span_table, out);
                    }
                }
                crate::ParameterKind::Generic => {}
            }
        }
    }
}

fn collect_refs_expr(
    expr: &crate::Expr,
    name: &str,
    span_table: &crate::SpanTable,
    out: &mut Vec<std::ops::Range<usize>>,
) {
    use crate::Expr;
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
                    collect_refs_expr(arg, name, span_table, out);
                }
            }
        }
        Expr::AnonymousTag(n) => {
            if n.as_str() == name {
                // Span not available without the wrapper; skip highlight for
                // bare tags in this context.
            }
        }
        Expr::TagCall(tc) => {
            // Span not available without the wrapper; skip highlight for
            // tag calls in this context.
            for arg in &tc.args {
                collect_refs_expr(arg, name, span_table, out);
            }
        }
        Expr::Binary(bin) => {
            collect_refs_expr(&bin.lhs.value, name, span_table, out);
            collect_refs_expr(&bin.rhs.value, name, span_table, out);
        }
        Expr::Bind(bind) => collect_refs_bind_value(bind.value(), name, span_table, out),
        Expr::When(when_expr) => {
            if let Some(subject) = &when_expr.subject {
                collect_refs_expr(subject, name, span_table, out);
            }
            for arm in &when_expr.arms {
                match arm {
                    crate::WhenArm::Cond {
                        condition, body, ..
                    } => {
                        collect_refs_expr(condition, name, span_table, out);
                        collect_refs_expr(body, name, span_table, out);
                    }
                    crate::WhenArm::Is { pattern, body, .. } => {
                        collect_refs_type_surface(&pattern.value, name, span_table, out);
                        collect_refs_expr(body, name, span_table, out);
                    }
                    crate::WhenArm::Else(body, _) => {
                        collect_refs_expr(body, name, span_table, out);
                    }
                }
            }
        }
        Expr::If(if_expr) => {
            match &if_expr.condition {
                crate::IfCondition::Bool(e) => collect_refs_expr(&e.value, name, span_table, out),
                crate::IfCondition::Pattern { subject, .. } => {
                    collect_refs_expr(&subject.value, name, span_table, out);
                }
            }
            for e in &if_expr.body {
                collect_refs_expr(e, name, span_table, out);
            }
        }
        Expr::Loop(loop_expr) => match loop_expr {
            crate::LoopEnum::ForIn(for_loop) => {
                collect_refs_expr(&for_loop.iter, name, span_table, out);
                for e in &for_loop.exprs {
                    collect_refs_expr(e, name, span_table, out);
                }
            }
            crate::LoopEnum::While(while_loop) => {
                collect_refs_expr(&while_loop.cond, name, span_table, out);
                for e in &while_loop.exprs {
                    collect_refs_expr(e, name, span_table, out);
                }
            }
        },
        Expr::FormatString(fs) => {
            for part in &fs.parts {
                if let crate::FormatPart::Expr(e, _) = part {
                    collect_refs_expr(e, name, span_table, out);
                }
            }
        }
        Expr::Range(range) => {
            collect_refs_expr(&range.start.value, name, span_table, out);
            collect_refs_expr(&range.end.value, name, span_table, out);
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
        Expr::TakePtr(inner)
        | Expr::TakeRef(inner)
        | Expr::Deref(inner)
        | Expr::Negate(inner)
        | Expr::MutArg(inner)
        | Expr::OwnArg(inner) => {
            collect_refs_expr(inner, name, span_table, out);
        }
        Expr::TupleLit(elems) | Expr::List(elems) => {
            for e in elems {
                collect_refs_expr(e, name, span_table, out);
            }
        }
        Expr::Lit(_)
        | Expr::SelfRef
        | Expr::Asm(_)
        | Expr::TypeNominal(..)
        | Expr::TypeQualified(_)
        | Expr::TypeGeneric { .. } => {}
    }
}

fn collect_refs_bind_value(
    value: &crate::BindValue,
    name: &str,
    span_table: &crate::SpanTable,
    out: &mut Vec<std::ops::Range<usize>>,
) {
    match value {
        crate::BindValue::Expr(e) => collect_refs_expr(&e.value, name, span_table, out),
        crate::BindValue::Body { exprs, ret } => {
            for e in exprs {
                collect_refs_expr(&e.value, name, span_table, out);
            }
            if let Some(r) = &ret.value {
                collect_refs_expr(&r.value, name, span_table, out);
            }
        }
        crate::BindValue::Extern => {}
    }
}
