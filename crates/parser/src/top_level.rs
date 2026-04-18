use ast::span::{SpanId, SpanTable};
use lexer::Token;
use std::collections::HashSet;

use ast::{
    Bind, Declare, DeclareValue, Expr, FileAst, ImplBlock, ParameterKind, Spanned, Tag, Variant,
    WhenArm, collapse_defs_for_platform, type_tag_expr_from_tag,
};
use indexmap::IndexMap;
use internment::Intern;

use crate::cursor::TokenCursor;
use crate::expr::ExprFn;

pub fn parse_file(cursor: &mut TokenCursor, expr_parser: ExprFn) -> FileAst {
    let imports = parse_imports(cursor);

    let mut public_elements = Vec::new();
    while let Some(el) = parse_element_line(cursor, expr_parser) {
        public_elements.push(el);
    }

    let mut private_elements = Vec::new();
    cursor.skip_newlines();
    if cursor.eat(&Token::Private) {
        cursor.skip_newlines();
        while let Some(el) = parse_element_line(cursor, expr_parser) {
            private_elements.push(el);
        }
    }

    let mut tags = ast::TagMap::new();
    let mut defs_scratch: IndexMap<Intern<String>, Vec<Bind>> = IndexMap::new();
    let mut private_defs = HashSet::new();
    let mut private_tags = HashSet::new();
    let mut exprs = Vec::new();

    for el in public_elements {
        collect_top_level(el, &mut tags, &mut defs_scratch, &mut exprs);
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
            TopLevelValue::Expr(..) => {}
        }
        collect_top_level(el, &mut tags, &mut defs_scratch, &mut exprs);
    }

    let defs = collapse_defs_for_platform(defs_scratch);
    generate_return_type_unions(&defs, &mut tags, &private_defs);

    FileAst {
        uses: imports,
        tags,
        defs,
        private_defs,
        private_tags,
        exprs,
        span_table: SpanTable::new(),
    }
}

fn parse_imports(cursor: &mut TokenCursor) -> Vec<ast::Import> {
    let mut imports = Vec::new();
    while cursor.is_at(&Token::Use) {
        if let Some(import) = crate::expr::import::parse_import(cursor) {
            imports.push(import);
        } else {
            break;
        }
    }
    imports
}

enum TopLevelValue {
    Tag(Declare),
    Bind(Box<Bind>),
    ImplBlock(ImplBlock),
    Expr(Expr, SpanId),
}

fn parse_top_level_element(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<TopLevelValue> {
    if cursor.is_eof() {
        return None;
    }

    match cursor.peek() {
        Some(Token::Private)
        | Some(Token::Dedent)
        | Some(Token::ParenClose)
        | Some(Token::Indent) => {
            return None;
        }
        _ => {}
    }

    let start_offset = skip_metadata_offset(cursor);
    let effective = cursor.peek_at(start_offset)?;

    match effective {
        Token::Tag(_) => dispatch_tag_element(cursor, expr_parser, start_offset),
        Token::Id(_) => {
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
            }
            // else: bare identifier or expression — no bind speculation needed
            let Spanned(expr, span) = expr_parser(cursor);
            Some(TopLevelValue::Expr(expr, span))
        }
        Token::Pound => {
            // #[...] always starts a bind, no speculation needed
            if let Some(bind) = crate::expr::bind::parse_bind(cursor, expr_parser) {
                return Some(TopLevelValue::Bind(Box::new(bind)));
            }
            let Spanned(expr, span) = expr_parser(cursor);
            Some(TopLevelValue::Expr(expr, span))
        }
        _ => {
            let Spanned(expr, span) = expr_parser(cursor);
            Some(TopLevelValue::Expr(expr, span))
        }
    }
}

fn dispatch_tag_element(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
    tag_offset: usize,
) -> Option<TopLevelValue> {
    let after_tag = tag_offset + 1;

    // Tag.Tag → impl_block (deterministic: no checkpoint/rewind needed)
    if matches!(cursor.peek_at(after_tag), Some(Token::Dot))
        && matches!(cursor.peek_at(after_tag + 1), Some(Token::Tag(_)))
    {
        return crate::impl_block::parse_impl_block(cursor, expr_parser)
            .map(TopLevelValue::ImplBlock);
    }

    // Tag.Id → method_bind (deterministic: no checkpoint/rewind needed)
    if matches!(cursor.peek_at(after_tag), Some(Token::Dot))
        && matches!(cursor.peek_at(after_tag + 1), Some(Token::Id(_)))
    {
        return parse_method_bind(cursor, expr_parser);
    }

    // Tag [params] is/has → declare (deterministic: no checkpoint/rewind needed)
    if is_declare_from_offset(cursor, tag_offset) {
        return crate::declare::parse_declare(cursor, expr_parser).map(TopLevelValue::Tag);
    }

    // fallback: expression (bare Tag, Tag(args), etc.)
    let Spanned(expr, span) = expr_parser(cursor);
    Some(TopLevelValue::Expr(expr, span))
}

fn parse_element_line(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<TopLevelValue> {
    let el = parse_top_level_element(cursor, expr_parser)?;
    Some(el)
}

fn parse_method_bind(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<TopLevelValue> {
    // skip past any metadata (indent, doc comments, attributes) to reach the Tag
    loop {
        match cursor.peek() {
            Some(Token::Indent) => {
                cursor.advance();
            }
            Some(Token::DocComment(_)) => {
                cursor.advance();
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

    let receiver_tag = match cursor.peek() {
        Some(Token::Tag(n)) => {
            let name = cursor.intern(n);
            let span = cursor.peek_span()?;
            cursor.advance();
            Tag::Nominal(name, span)
        }
        _ => return None,
    };

    if !cursor.eat(&Token::Dot) {
        return None;
    }

    let bind = crate::expr::bind::parse_bind(cursor, expr_parser)?;
    let bind = bind.with_receiver_type(Some(Box::new(type_tag_expr_from_tag(receiver_tag))));

    Some(TopLevelValue::Bind(Box::new(bind)))
}

fn collect_top_level(
    el: TopLevelValue,
    tags: &mut ast::TagMap,
    defs: &mut IndexMap<Intern<String>, Vec<Bind>>,
    exprs: &mut Vec<(Expr, SpanId)>,
) {
    match el {
        TopLevelValue::Tag(decl) => {
            let name = decl.name();
            tags.insert(name, decl);
        }
        TopLevelValue::Bind(bind) => {
            let name = if let Some(recv) = bind.receiver_type() {
                Intern::<String>::new(format!("{}.{}", recv.name(), bind.name()))
            } else {
                bind.name()
            };
            defs.entry(name).or_insert_with(Vec::new).push(*bind);
        }
        TopLevelValue::ImplBlock(block) => {
            let recv = Box::new(type_tag_expr_from_tag(Tag::Nominal(
                block.type_name,
                block.type_name_span,
            )));
            for (method_name, bind) in block.methods {
                let bind = bind.with_receiver_type(Some(recv.clone()));
                let mangled = Intern::<String>::new(format!(
                    "{}.{}",
                    block.type_name.as_str(),
                    method_name.as_str()
                ));
                defs.entry(mangled).or_insert_with(Vec::new).push(bind);
            }
        }
        TopLevelValue::Expr(expr, span) => {
            exprs.push((expr, span));
        }
    }
}

fn generate_return_type_unions(
    defs: &ast::DefMap,
    tags: &mut ast::TagMap,
    _private_defs: &HashSet<Intern<String>>,
) {
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
            .map(|(name, span)| Variant::External(Tag::Nominal(name, span)))
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
                loop {
                    match cursor.peek_at(offset) {
                        Some(Token::BracketClose) => {
                            offset += 1;
                            break;
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

    if matches!(cursor.peek_at(offset), Some(Token::ParenOpen)) {
        offset += 1;
        let mut depth = 1;
        while depth > 0 {
            match cursor.peek_at(offset) {
                Some(Token::ParenOpen) => {
                    depth += 1;
                    offset += 1;
                }
                Some(Token::ParenClose) => {
                    depth -= 1;
                    offset += 1;
                }
                None => return false,
                _ => {
                    offset += 1;
                }
            }
        }
    }

    while matches!(
        cursor.peek_at(offset),
        Some(Token::Newline) | Some(Token::Indent)
    ) {
        offset += 1;
    }

    matches!(cursor.peek_at(offset), Some(Token::Is) | Some(Token::Has))
}

fn extract_anonymous_tags_from_bind(bind: &Bind, tags: &mut Vec<(Intern<String>, SpanId)>) {
    use ast::{BindValue, type_tag_as_tag};

    if let Some(t) = bind.receiver_type() {
        extract_anonymous_tags_from_is_pattern_tag(t, tags);
    }
    if let Some(sp) = &bind.return_tag {
        if let Some(t) = type_tag_as_tag(&sp.0) {
            extract_anonymous_tags_from_is_pattern_tag(t, tags);
        }
    }

    match bind.value() {
        BindValue::Expr(expr) => {
            extract_anonymous_tags_from_expr(expr, tags);
        }
        BindValue::Body { exprs, ret } => {
            for expr in exprs {
                extract_anonymous_tags_from_expr(expr, tags);
            }
            if let Some(expr) = &ret.0 {
                extract_anonymous_tags_from_expr(expr, tags);
            }
        }
        BindValue::Extern => {}
    }
}

fn extract_anonymous_tags_from_is_pattern_tag(tag: &Tag, tags: &mut Vec<(Intern<String>, SpanId)>) {
    match tag {
        Tag::Nominal(name, span) => {
            tags.push((*name, *span));
        }
        Tag::Qualified(_) => {}
        Tag::Generic(_, params, _) => {
            for pk in params.values() {
                match pk {
                    ParameterKind::Default(e) => extract_anonymous_tags_from_expr(e, tags),
                    ParameterKind::Tagged(inner) => {
                        extract_anonymous_tags_from_is_pattern_tag(inner, tags);
                    }
                    ParameterKind::Generic => {}
                }
            }
        }
    }
}

fn extract_anonymous_tags_from_expr(expr: &Expr, tags: &mut Vec<(Intern<String>, SpanId)>) {
    match expr {
        Expr::AnonymousTag(name, span) => {
            tags.push((*name, *span));
        }
        Expr::FnCall(call) => {
            if let Some(args) = &call.args {
                for arg in args {
                    extract_anonymous_tags_from_expr(arg, tags);
                }
            }
        }
        Expr::Binary(bin) => {
            extract_anonymous_tags_from_expr(&bin.lhs, tags);
            extract_anonymous_tags_from_expr(&bin.rhs, tags);
        }
        Expr::Loop(loop_expr) => {
            use ast::Loop;
            match loop_expr {
                Loop::ForIn(for_loop) => {
                    for e in &for_loop.exprs {
                        extract_anonymous_tags_from_expr(e, tags);
                    }
                    extract_anonymous_tags_from_expr(&for_loop.iter, tags);
                }
                Loop::While(while_loop) => {
                    for e in &while_loop.exprs {
                        extract_anonymous_tags_from_expr(e, tags);
                    }
                    extract_anonymous_tags_from_expr(&while_loop.cond, tags);
                }
            }
        }
        Expr::When(when_expr) => {
            if let Some(subject) = &when_expr.subject {
                extract_anonymous_tags_from_expr(subject, tags);
            }
            for arm in &when_expr.arms {
                match arm {
                    WhenArm::Cond { condition, body } => {
                        extract_anonymous_tags_from_expr(condition, tags);
                        extract_anonymous_tags_from_expr(body, tags);
                    }
                    WhenArm::Is { body, .. } => {
                        extract_anonymous_tags_from_expr(body, tags);
                    }
                    WhenArm::Else(body) => {
                        extract_anonymous_tags_from_expr(body, tags);
                    }
                }
            }
        }
        Expr::Bind(bind) => {
            use ast::expr::BindValue;
            match bind.value() {
                BindValue::Expr(e) => {
                    extract_anonymous_tags_from_expr(e, tags);
                }
                BindValue::Body { exprs, ret } => {
                    for e in exprs {
                        extract_anonymous_tags_from_expr(e, tags);
                    }
                    if let Some(e) = &ret.0 {
                        extract_anonymous_tags_from_expr(e, tags);
                    }
                }
                BindValue::Extern => {}
            }
        }
        Expr::IsPattern(t) | Expr::TypeTag(t) => {
            extract_anonymous_tags_from_is_pattern_tag(t, tags)
        }
        _ => {}
    }
}
