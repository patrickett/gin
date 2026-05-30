use ast::span::{SpanId, SpanTable};
use lexer::Token;
use std::collections::HashSet;

use ast::{
    Bind, Declare, DeclareValue, Expr, FileAst, ImplBlock, ParameterKind, Spanned, TypeExpr, Typed,
    Variant, type_surface_mangle_name,
};
use indexmap::IndexMap;
use internment::Intern;

use crate::cursor::TokenCursor;
use crate::expr::ExprFn;
use crate::expr::bind::parse_doc_comment;

pub fn parse_file(cursor: &mut TokenCursor, expr_parser: ExprFn) -> FileAst {
    let module_doc = parse_module_doc(cursor);
    let imports = parse_imports(cursor);

    let mut public_elements = Vec::new();
    loop {
        cursor.advance_push();
        match parse_element_line(cursor, expr_parser) {
            Some(el) => {
                cursor.advance_pop();
                public_elements.push(el);
            }
            None => {
                cursor.advance_drop();
                break;
            }
        }
    }

    let mut private_elements = Vec::new();
    cursor.skip_newlines();
    if cursor.eat(&Token::Private) {
        cursor.skip_newlines();
        loop {
            cursor.advance_push();
            match parse_element_line(cursor, expr_parser) {
                Some(el) => {
                    cursor.advance_pop();
                    private_elements.push(el);
                }
                None => {
                    cursor.advance_drop();
                    break;
                }
            }
        }
    }

    let mut tags_scratch: IndexMap<Intern<String>, Vec<Declare>> = IndexMap::new();
    let mut defs_scratch: IndexMap<Intern<String>, Vec<Bind>> = IndexMap::new();
    let mut private_defs = HashSet::new();
    let mut private_tags = HashSet::new();
    let mut exprs = Vec::new();
    let mut blanket_impls = Vec::new();

    for el in public_elements {
        collect_top_level(
            el,
            &mut tags_scratch,
            &mut defs_scratch,
            &mut exprs,
            &mut blanket_impls,
        );
    }

    for el in private_elements {
        match &el {
            TopLevelValue::Tag(decl) => {
                private_tags.insert(decl.name());
            }
            TopLevelValue::Bind(bind) => {
                private_defs.insert(bind.name());
            }
            TopLevelValue::ImplBlock(block) => {
                for method_name in block.methods.keys() {
                    let mangled = Intern::<String>::new(format!(
                        "{}.{}",
                        block.type_name.as_str(),
                        method_name.as_str()
                    ));
                    private_defs.insert(mangled);
                }
            }
            TopLevelValue::BlanketImpl(..) | TopLevelValue::Expr(..) => {}
        }
        collect_top_level(
            el,
            &mut tags_scratch,
            &mut defs_scratch,
            &mut exprs,
            &mut blanket_impls,
        );
    }

    let mut tags = ast::TagMap::new();
    for (name, declares) in tags_scratch {
        if let Some(decl) = declares.into_iter().next() {
            tags.insert(name, decl);
        }
    }
    let mut defs = ast::DefMap::new();
    let mut parse_warnings = Vec::new();
    for (name, binds) in defs_scratch {
        parse_warnings.extend(ast::warnings::const_bind_after_declare_warnings(
            &binds,
            cursor.span_table(),
        ));
        if let Some(bind) = binds.into_iter().last() {
            defs.insert(name, bind);
        }
    }
    generate_return_type_unions(&defs, &mut tags, &private_defs);

    FileAst {
        module_doc,
        uses: imports,
        tags,
        defs,
        private_defs,
        private_tags,
        exprs,
        symbol_aliases: Vec::new(),
        symbol_alias_spans: Vec::new(),
        span_table: SpanTable::new(),
        blanket_impls,
        parse_warnings,
    }
}

fn parse_module_doc(cursor: &mut TokenCursor) -> Option<ast::DocComment> {
    cursor.skip_newlines();

    let first = match cursor.peek()? {
        Token::ModuleDocComment(text) => {
            let stripped = text
                .strip_prefix("--|")
                .map(|s| s.trim_start())
                .unwrap_or(text)
                .to_owned();
            cursor.advance();
            stripped
        }
        _ => return None,
    };

    // Fast path: single-line module doc
    if !matches!(cursor.peek(), Some(Token::ModuleDocComment(_))) {
        let doc = ast::DocComment { value: first };
        return if doc.is_empty() { None } else { Some(doc) };
    }

    let mut lines = vec![first];
    while let Some(Token::ModuleDocComment(text)) = cursor.peek() {
        let stripped = text
            .strip_prefix("--|")
            .map(|s| s.trim_start())
            .unwrap_or(text)
            .to_owned();
        cursor.advance();
        lines.push(stripped);
    }

    let doc = ast::DocComment {
        value: lines.join("\n"),
    };
    if doc.is_empty() { None } else { Some(doc) }
}

fn parse_imports(cursor: &mut TokenCursor) -> Vec<ast::Import> {
    let mut imports = Vec::new();
    while cursor.is_at(&Token::Use) {
        cursor.advance_push();
        match crate::expr::import::parse_import(cursor) {
            Some(import) => {
                cursor.advance_pop();
                imports.push(import);
            }
            None => {
                cursor.advance_drop();
                break;
            }
        }
    }
    imports
}

fn can_start_top_level_after_dedent(cursor: &TokenCursor) -> bool {
    let start = skip_metadata_offset(cursor);
    match cursor.peek_at(start) {
        Some(Token::Tag(_)) => true,
        Some(Token::Id(_)) => matches!(
            cursor.peek_at(start + 1),
            Some(Token::Colon) | Some(Token::ColonEq) | Some(Token::ParenOpen) | Some(Token::Has)
        ),
        _ => false,
    }
}

enum TopLevelValue {
    Tag(Declare),
    Bind(Box<Bind>),
    ImplBlock(ImplBlock),
    BlanketImpl(ast::BlanketImpl),
    Expr(Typed<Expr>),
}

fn parse_top_level_element(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<TopLevelValue> {
    if cursor.is_eof() {
        return None;
    }

    if matches!(cursor.peek(), Some(Token::ModuleDocComment(_))) {
        cursor.error(
            "module doc comments (--|) are only allowed at the start of the file",
            cursor.peek_span().unwrap_or(SpanId::INVALID),
        );
        cursor.advance();
        cursor.consume_trailing_newline();
        return None;
    }

    if cursor.is_at(&Token::Dedent) {
        cursor.advance();
        if can_start_top_level_after_dedent(cursor) {
            return parse_top_level_element(cursor, expr_parser);
        }
        return None;
    }

    match cursor.peek() {
        Some(Token::Private) | Some(Token::ParenClose) | Some(Token::Indent) => {
            return None;
        }
        _ => {}
    }

    let start_offset = skip_metadata_offset(cursor);
    let effective = cursor.peek_at(start_offset)?;

    match effective {
        Token::Tag(_) => dispatch_tag_element(cursor, expr_parser, start_offset),
        Token::Id(_) => {
            if crate::declare::is_blanket_impl_start(cursor) {
                if let Some(blanket) = crate::declare::parse_blanket_impl(cursor, expr_parser) {
                    return Some(TopLevelValue::BlanketImpl(blanket));
                }
                return None;
            }
            // Deterministic dispatch: if next token after id is : or :=, it's definitely a bind.
            // No checkpoint/rewind needed for the common case (x: expr, x := expr).
            // For id(...) and id Tag, use speculative parsing only for the truly ambiguous cases.
            if matches!(
                cursor.peek_at(start_offset + 1),
                Some(Token::Colon) | Some(Token::ColonEq)
            ) {
                // id: or id:= → bind, no speculation needed
                if let Some(bind) = crate::expr::bind::parse_bind(cursor, expr_parser) {
                    return Some(TopLevelValue::Bind(Box::new(bind)));
                }
            } else if matches!(cursor.peek_at(start_offset + 1), Some(Token::ParenOpen)) {
                // Could be function def (id(...) RetType:) — speculative
                let checkpoint = cursor.checkpoint();
                if let Some(bind) = crate::expr::bind::parse_bind(cursor, expr_parser) {
                    return Some(TopLevelValue::Bind(Box::new(bind)));
                }
                cursor.rewind(checkpoint);
            } else if matches!(cursor.peek_at(start_offset + 1), Some(Token::BracketOpen)) {
                // id[...](...) or id[...] Tag: — group annotations + optional params
                let checkpoint = cursor.checkpoint();
                if let Some(bind) = crate::expr::bind::parse_bind(cursor, expr_parser) {
                    return Some(TopLevelValue::Bind(Box::new(bind)));
                }
                cursor.rewind(checkpoint);
            } else if matches!(cursor.peek_at(start_offset + 1), Some(Token::Tag(_))) {
                // `arch Architecture` — declare without value (same as unassigned bind).
                let is_type_only_declare = !matches!(
                    cursor.peek_at(start_offset + 2),
                    Some(Token::Colon) | Some(Token::ColonEq)
                );
                if is_type_only_declare {
                    if let Some(bind) = crate::expr::bind::parse_bind(cursor, expr_parser) {
                        return Some(TopLevelValue::Bind(Box::new(bind)));
                    }
                } else {
                    // `id Tag: value` — typed bind with inline type annotation
                    let checkpoint = cursor.checkpoint();
                    if let Some(bind) = crate::expr::bind::parse_bind(cursor, expr_parser) {
                        return Some(TopLevelValue::Bind(Box::new(bind)));
                    }
                    cursor.rewind(checkpoint);
                }
            }
            // else: bare identifier or expression — no bind speculation needed
            let expr = expr_parser(cursor);
            // Feature 3: Detect `{expr} {expr}` on the same line without an operator.
            if let Some(next_tok) = cursor.peek_at(0)
                && crate::expr::body::can_start_expr(next_tok)
            {
                cursor.error(
                    format!("expected operator between expressions, found {next_tok:?}"),
                    cursor.peek_span().unwrap_or(expr.span_id),
                );
            }
            Some(TopLevelValue::Expr(expr))
        }
        Token::Pound => {
            // #[...] always starts a bind, no speculation needed
            if let Some(bind) = crate::expr::bind::parse_bind(cursor, expr_parser) {
                return Some(TopLevelValue::Bind(Box::new(bind)));
            }
            let expr = expr_parser(cursor);
            Some(TopLevelValue::Expr(expr))
        }
        _ => {
            let expr = expr_parser(cursor);
            Some(TopLevelValue::Expr(expr))
        }
    }
}

fn dispatch_tag_element(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
    tag_offset: usize,
) -> Option<TopLevelValue> {
    let after_tag = tag_offset + 1;

    // Helper: given an offset that may be at a method separator (. or ::),
    // return the offset just past it if found.
    let sep_past = |cursor: &TokenCursor, offset: usize| -> Option<usize> {
        if cursor.peek_at(offset) == Some(&Token::Dot) {
            Some(offset + 1)
        } else if matches!(cursor.peek_at(offset), Some(Token::Colon))
            && cursor.peek_at(offset + 1) == Some(&Token::Colon)
        {
            Some(offset + 2)
        } else {
            None
        }
    };

    // Tag.Tag → impl_block (deterministic: no checkpoint/rewind needed)
    if matches!(cursor.peek_at(after_tag), Some(Token::Dot))
        && matches!(cursor.peek_at(after_tag + 1), Some(Token::Tag(_)))
    {
        return crate::impl_block::parse_impl_block(cursor, expr_parser)
            .map(TopLevelValue::ImplBlock);
    }

    // Tag.Id or Tag::Id → method_bind (deterministic: no checkpoint/rewind needed)
    if let Some(past_sep) = sep_past(cursor, after_tag)
        && matches!(
            cursor.peek_at(past_sep),
            Some(Token::Id(_)) | Some(Token::Tag(_))
        )
    {
        return parse_method_bind(cursor, expr_parser);
    }

    // Tag(...).Id / Tag(...)::Id or Tag[...].Id / Tag[...]::Id → generic-receiver method_bind
    if let Some(after_parens) = skip_balanced_parens_offset(cursor, after_tag)
        && let Some(past_sep) = sep_past(cursor, after_parens)
        && matches!(
            cursor.peek_at(past_sep),
            Some(Token::Id(_)) | Some(Token::Tag(_))
        )
    {
        return parse_method_bind(cursor, expr_parser);
    }

    if let Some(after_brackets) = skip_balanced_brackets_offset(cursor, after_tag)
        && let Some(past_sep) = sep_past(cursor, after_brackets)
        && matches!(
            cursor.peek_at(past_sep),
            Some(Token::Id(_)) | Some(Token::Tag(_))
        )
    {
        return parse_method_bind(cursor, expr_parser);
    }

    // Tag [params] is/has → declare (deterministic: no checkpoint/rewind needed)
    if is_declare_from_offset(cursor, tag_offset) {
        return crate::declare::parse_declare(cursor, expr_parser).map(TopLevelValue::Tag);
    }

    // fallback: expression (bare Tag, Tag(args), etc.)
    let expr = expr_parser(cursor);
    Some(TopLevelValue::Expr(expr))
}

/// If the token at `offset` is `(`, return the offset just past the matching `)`.
/// Returns `None` if there is no `(` at `offset` or the parens are unbalanced.
fn skip_balanced_parens_offset(cursor: &TokenCursor, offset: usize) -> Option<usize> {
    if !matches!(cursor.peek_at(offset), Some(Token::ParenOpen)) {
        return None;
    }
    let mut o = offset + 1;
    let mut depth = 1;
    while depth > 0 {
        match cursor.peek_at(o) {
            Some(Token::ParenOpen) => {
                depth += 1;
                o += 1;
            }
            Some(Token::ParenClose) => {
                depth -= 1;
                o += 1;
            }
            None => return None,
            _ => {
                o += 1;
            }
        }
    }
    Some(o)
}

fn skip_balanced_brackets_offset(cursor: &TokenCursor, offset: usize) -> Option<usize> {
    if !matches!(cursor.peek_at(offset), Some(Token::BracketOpen)) {
        return None;
    }
    let mut o = offset + 1;
    let mut depth = 1;
    while depth > 0 {
        match cursor.peek_at(o) {
            Some(Token::BracketOpen) => {
                depth += 1;
                o += 1;
            }
            Some(Token::BracketClose) => {
                depth -= 1;
                o += 1;
            }
            None => return None,
            _ => {
                o += 1;
            }
        }
    }
    Some(o)
}

fn parse_element_line(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<TopLevelValue> {
    let el = parse_top_level_element(cursor, expr_parser)?;
    Some(el)
}

fn parse_method_bind(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<TopLevelValue> {
    // skip past any metadata (indent, doc comments, attributes) to reach the Tag
    let mut doc_before = None;
    loop {
        match cursor.peek() {
            Some(Token::Indent) => {
                cursor.advance();
            }
            Some(Token::DocComment(_)) => {
                if let Some(doc) = parse_doc_comment(cursor) {
                    doc_before = Some(doc);
                }
            }
            Some(Token::Pound) => {
                cursor.advance();
                loop {
                    match cursor.peek() {
                        Some(Token::BracketClose) => {
                            cursor.advance();
                            break;
                        }
                        None => return None,
                        _ => {
                            cursor.advance();
                        }
                    }
                }
            }
            _ => break,
        }
    }

    // Receiver may be a bare Tag, a generic Tag(args), or a qualified Mod.Tag.
    let recv = crate::tag::parse_type_expr(cursor, expr_parser)?;

    // Accept `.` (Type.method) or `::` (Type::method) as the separator.
    let has_sep =
        cursor.eat(&Token::Dot) || (cursor.eat(&Token::Colon) && cursor.eat(&Token::Colon));
    if !has_sep {
        return None;
    }

    let mut bind = crate::expr::bind::parse_bind(cursor, expr_parser)?;
    let doc = bind.doc_comment().cloned().or(doc_before);
    bind = bind.with_doc(doc);
    let bind = bind.with_receiver_type(Some(Box::new(recv)));

    Some(TopLevelValue::Bind(Box::new(bind)))
}

fn collect_top_level(
    el: TopLevelValue,
    tags: &mut IndexMap<Intern<String>, Vec<Declare>>,
    defs: &mut IndexMap<Intern<String>, Vec<Bind>>,
    exprs: &mut Vec<(Expr, SpanId)>,
    blanket_impls: &mut Vec<ast::BlanketImpl>,
) {
    match el {
        TopLevelValue::BlanketImpl(b) => blanket_impls.push(b),
        TopLevelValue::Tag(decl) => {
            let name = decl.name();
            tags.entry(name).or_default().push(decl);
        }
        TopLevelValue::Bind(bind) => {
            let name = if let Some(sp) = bind.receiver_type_surface() {
                Intern::<String>::new(format!(
                    "{}.{}",
                    type_surface_mangle_name(&sp.value),
                    bind.name()
                ))
            } else {
                bind.name()
            };
            defs.entry(name).or_default().push(*bind);
        }
        TopLevelValue::ImplBlock(block) => {
            let recv = Box::new(Spanned {
                value: TypeExpr::Nominal(block.type_name, block.type_name_span),
                span_id: block.type_name_span,
            });
            for (method_name, bind) in block.methods {
                let bind = bind.with_receiver_type(Some(recv.clone()));
                let mangled = Intern::<String>::new(format!(
                    "{}.{}",
                    block.type_name.as_str(),
                    method_name.as_str()
                ));
                defs.entry(mangled).or_default().push(bind);
            }
        }
        TopLevelValue::Expr(expr) => {
            exprs.push((expr.value, expr.span_id));
        }
    }
}

fn generate_return_type_unions(
    defs: &ast::DefMap,
    tags: &mut ast::TagMap,
    _private_defs: &HashSet<Intern<String>>,
) {
    // Only adds new tags from bind return types; existing tag map is unchanged.
    let mut tag_buffer = Vec::new(); // reused across iterations
    for bind in defs.values() {
        tag_buffer.clear();
        extract_anonymous_tags_from_bind(bind, &mut tag_buffer);
        if tag_buffer.is_empty() {
            continue;
        }

        let unique_tags: HashSet<_> = tag_buffer.drain(..).collect();
        let variants: Vec<Variant> = unique_tags
            .into_iter()
            .map(|(name, span)| {
                Variant::External(Box::new(Spanned {
                    value: TypeExpr::Nominal(name, span),
                    span_id: span,
                }))
            })
            .collect();

        if let Some(name) = bind.return_type_name() {
            let decl = Declare::new(*name, SpanId::INVALID, DeclareValue::Union { variants });
            tags.insert(decl.name(), decl);
        }
    }
}

fn skip_metadata_offset(cursor: &TokenCursor) -> usize {
    let mut offset = 0;
    loop {
        match cursor.peek_at(offset) {
            Some(Token::DocComment(_)) | Some(Token::Newline) | Some(Token::Indent) => {
                offset += 1;
            }
            Some(Token::Pound) => {
                offset += 1;
                // Track bracket nesting to handle `#[attr([inner])]` correctly.
                // Depth starts at 0; the `BracketOpen` immediately after `#`
                // increments to 1, and we break when it returns to 0.
                let mut depth = 0u32;
                loop {
                    match cursor.peek_at(offset) {
                        Some(Token::BracketOpen) => {
                            depth += 1;
                            offset += 1;
                        }
                        Some(Token::BracketClose) => {
                            depth -= 1;
                            offset += 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        None => return offset,
                        _ => {
                            offset += 1;
                        }
                    }
                }
            }
            _ => return offset,
        }
    }
}

fn is_declare_from_offset(cursor: &TokenCursor, tag_offset: usize) -> bool {
    let mut offset = tag_offset + 1;
    if let Some(next) = skip_balanced_parens_offset(cursor, offset) {
        offset = next;
    } else if let Some(next) = skip_balanced_brackets_offset(cursor, offset) {
        offset = next;
    }

    while matches!(
        cursor.peek_at(offset),
        Some(Token::Newline) | Some(Token::Indent) | Some(Token::Whitespace)
    ) {
        offset += 1;
    }

    matches!(cursor.peek_at(offset), Some(Token::Is) | Some(Token::Has))
}

fn extract_anonymous_tags_from_bind(bind: &Bind, tags: &mut Vec<(Intern<String>, SpanId)>) {
    if let Some(sp) = bind.receiver_type_surface() {
        collect_type_surface_tags(&sp.value, tags);
    }
    if let Some(sp) = &bind.return_tag {
        collect_type_surface_tags(&sp.value, tags);
    }
}

fn collect_type_surface_tags(expr: &TypeExpr, tags: &mut Vec<(Intern<String>, SpanId)>) {
    match expr {
        TypeExpr::Nominal(name, span) => {
            tags.push((*name, *span));
        }
        TypeExpr::Qualified(_) => {}
        TypeExpr::Literal(..) => {}
        TypeExpr::Pointer(_) | TypeExpr::Unit | TypeExpr::ListEmpty => {}
        TypeExpr::ListCons { head, tail } => {
            collect_type_surface_tags(&head.value, tags);
            collect_type_surface_tags(&tail.value, tags);
        }
        TypeExpr::Tuple(elems) => {
            for e in elems {
                collect_type_surface_tags(&e.value, tags);
            }
        }
        TypeExpr::InRange { bounds, span } => {
            if let ast::InRangeBounds::Tag(name) = bounds {
                tags.push((*name, *span));
            }
        }
        TypeExpr::Ref { inner, .. } => collect_type_surface_tags(&inner.value, tags),
        TypeExpr::Generic { params, .. } => {
            for (_, pk) in params {
                match pk {
                    ParameterKind::Default(_e) => {
                        // Anonymous tags in default expressions are handled
                        // by collect_type_surface_tags elsewhere.
                    }
                    ParameterKind::Tagged(sp) => {
                        let te = &sp.value;
                        collect_type_surface_tags(te, tags);
                    }
                    ParameterKind::Generic => {}
                }
            }
        }
    }
}
