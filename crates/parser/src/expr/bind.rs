use indexmap::IndexMap;
use internment::Intern;
use lexer::Token;

use ast::{
    AttributeItem, Bind, BindAttributes, BindValue, DocComment, Expr, ModPath, ParamConvention,
    ParameterKind, Parameters, Return, Spanned, TypeExpr, type_surface_mangle_name,
};

use super::ExprFn;
use super::control::parse_return;
use crate::cursor::TokenCursor;
use crate::path::{parse_id, parse_tag_variant_path};
use crate::tag::{parse_tag_type_params, parse_type_expr};

type ReturnTypePart = (
    Option<Intern<String>>,
    Option<Box<Spanned<TypeExpr>>>,
    Option<(Intern<String>, Vec<Spanned<Expr>>)>,
    Option<Spanned<ModPath>>,
);

pub fn parse_bind(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<Bind> {
    cursor.skip_newlines();

    // Doc comments may appear before or after attributes; try both positions.
    let doc_before_attrs = parse_doc_comment(cursor);
    let attrs = parse_bind_attributes(cursor);
    let doc_before = parse_doc_comment(cursor).or(doc_before_attrs);

    cursor.eat(&Token::Indent);

    let (name, name_span) = match cursor.peek() {
        Some(Token::Id(n)) => {
            let name = cursor.intern(n);
            let span = cursor.peek_span()?;
            cursor.advance();
            (name, span)
        }
        _ => return None,
    };

    let (params, conventions) = parse_params(cursor, expr_parser);

    let (return_type_name, return_tag, type_annotation, type_annotation_qual) =
        parse_return_type_part(cursor, expr_parser);

    // Handle `extern` binds: `name(params) extern` without `:` or `:=`
    let (value, postfix_doc, is_const) = if cursor.eat(&Token::Extern) {
        (BindValue::Extern, None, false)
    } else {
        let is_const = if cursor.eat(&Token::ColonEq) {
            true
        } else if cursor.eat(&Token::Colon) {
            false
        } else {
            cursor.error("expected ':=', ':', or 'extern'", cursor.current_span());
            return None;
        };

        // For `:=` (const) binds, Indent means expression continuation,
        // not block body. For `:` (function) binds, Indent starts a block.
        if is_const && cursor.is_at(&Token::Indent) {
            // Const bind with Indent: skip Indent, parse as continuation expr.
            cursor.skip_indents();
            let expr = expr_parser(cursor);
            cursor.eat(&Token::Dedent);
            (BindValue::Expr(Box::new(expr)), None, true)
        } else {
            let (value, postfix_doc) = parse_bind_value(cursor, expr_parser);
            (value, postfix_doc, is_const)
        }
    };

    let doc = postfix_doc.or(doc_before);

    let mut bind = Bind::new(name, name_span, value, is_const)
        .with_params(params)
        .with_return_type_name(return_type_name)
        .with_doc(doc);

    if let Some(attrs) = attrs {
        bind = bind.with_attributes(attrs);
    }
    bind.param_conventions = conventions;
    bind.return_tag = return_tag;
    bind.type_annotation = type_annotation;
    bind.type_annotation_qual = type_annotation_qual;

    Some(bind)
}

pub fn parse_bind_attributes(cursor: &mut TokenCursor) -> Option<BindAttributes> {
    if !cursor.is_at(&Token::Pound) {
        return None;
    }
    cursor.advance();

    cursor.expect(&Token::BracketOpen)?;

    let mut items: Vec<AttributeItem> = Vec::new();

    if !cursor.is_at(&Token::BracketClose) {
        loop {
            if let Some(item) = parse_one_attribute_item(cursor) {
                items.push(item);
            }
            if cursor.eat(&Token::Comma) {
                continue;
            }
            break;
        }
    }

    cursor.expect(&Token::BracketClose);

    let mut attrs = BindAttributes {
        raw_attributes: Some(items),
        ..Default::default()
    };
    attrs.extract_intrinsic_attributes();
    Some(attrs)
}

pub(crate) fn parse_one_attribute_item(cursor: &mut TokenCursor) -> Option<AttributeItem> {
    match cursor.peek()? {
        Token::Id(name) => {
            let name_interned = cursor.intern(name);
            let name_span = cursor.peek_span()?;
            cursor.advance();

            // Id(...) → function call attribute
            if cursor.is_at(&Token::ParenOpen) {
                let args = crate::expr::parse_paren_args(cursor).unwrap_or_default();
                Some(AttributeItem::Call {
                    name: name_interned,
                    name_span,
                    args,
                })
            } else {
                // bare Id → flag attribute
                Some(AttributeItem::Flag {
                    name: name_interned,
                    span: name_span,
                })
            }
        }
        _ => None,
    }
}

fn parse_return_type_part(cursor: &mut TokenCursor, expr_parser: ExprFn) -> ReturnTypePart {
    // lowercase id → named union return type (e.g., `print(a) result:`)
    if let Some(name) = parse_id(cursor) {
        return (Some(name), None, None, None);
    }

    // Tag-based type annotations
    if !matches!(cursor.peek(), Some(Token::Tag(_))) {
        return (None, None, None, None);
    }

    // Qualified path: Tag.Tag[.Tag...][(args)] (e.g., `Maybe.Some(3)`, `Bool.True`)
    // Guard: a qualified path requires Tag followed by Dot. Skip the speculative
    // checkpoint + allocation when this pattern is absent (e.g., simple `Str`, `Int`).
    if matches!(cursor.peek_at(1), Some(Token::Dot)) {
        let checkpoint = cursor.checkpoint();
        if let Some(path) = parse_tag_variant_path(cursor) {
            if cursor.is_at(&Token::ParenOpen) {
                let args = parse_type_annotation_args(cursor, expr_parser);
                if !args.is_empty() {
                    let variant_name = *path.segments.last().unwrap_or(&path.root);
                    return (None, None, Some((variant_name, args)), Some(path));
                }
            }
            let span = path.span_id;
            return (
                None,
                Some(Box::new(Spanned {
                    value: TypeExpr::Qualified(path),
                    span_id: span,
                })),
                None,
                None,
            );
        }
        cursor.rewind(checkpoint);
    }

    // Simple Tag or Tag(args) (e.g., `Str`, `Maybe(3)`)
    let (name, name_span) = match cursor.peek() {
        Some(Token::Tag(n)) => {
            let name = cursor.intern(n);
            let span = cursor
                .peek_span()
                .expect("peek confirmed Tag token, peek_span should succeed");
            cursor.advance();
            (name, span)
        }
        _ => return (None, None, None, None),
    };

    if cursor.is_at(&Token::BracketOpen) {
        let params = parse_tag_type_params(cursor, expr_parser);
        if !params.is_empty() {
            let end_span = cursor.last_consumed_span();
            let span = cursor.merge_span(name_span, end_span);
            return (
                None,
                Some(Box::new(Spanned {
                    value: TypeExpr::Generic {
                        name,
                        params: params.into_iter().collect(),
                        span,
                    },
                    span_id: span,
                })),
                None,
                None,
            );
        }
    }

    if cursor.is_at(&Token::ParenOpen) {
        let args = parse_type_annotation_args(cursor, expr_parser);
        if !args.is_empty() {
            // If the args are all type-like (bare identifiers or tags), promote
            // to a `TypeGeneric` return tag so the typechecker sees a uniform
            // type-surface shape (matching how receivers are stored). Examples:
            //   `Range[x]` → return_tag = TypeGeneric { Range, [x: Generic] }
            //   `Maybe(3)` → falls through to `type_annotation` (value annotation)
            if let Some(type_params) = try_args_as_type_params(&args) {
                return (
                    None,
                    Some(Box::new(Spanned {
                        value: TypeExpr::Generic {
                            name,
                            params: type_params,
                            span: name_span,
                        },
                        span_id: name_span,
                    })),
                    None,
                    None,
                );
            }
            return (None, None, Some((name, args)), None);
        }
    }

    (
        None,
        Some(Box::new(Spanned {
            value: TypeExpr::Nominal(name, name_span),
            span_id: name_span,
        })),
        None,
        None,
    )
}

/// If every arg is a "type-like" expression (bare lowercase identifier or bare
/// tag), convert to declaration parameters suitable for `TypeExpr::Generic`.
/// Otherwise return `None` so the caller can fall back to the value-annotation
/// path (e.g., `Maybe(3)`).
fn try_args_as_type_params(args: &[Spanned<Expr>]) -> Option<Vec<(Intern<String>, ParameterKind)>> {
    let mut out = Vec::with_capacity(args.len());
    for Spanned {
        value: arg,
        span_id: span,
    } in args
    {
        match arg {
            // Bare lowercase identifier, e.g. `x` in `Range[x]` → type-variable
            // *introduction* (matches how `Range[x]` parses in receiver and
            // declaration positions). Stored as `Generic` so the typechecker
            // doesn't report `x` as an undeclared tag.
            Expr::FnCall(call) if call.path.segments.is_empty() && call.args.is_none() => {
                out.push((call.path.root, ParameterKind::Generic));
            }
            // Bare capitalized tag, e.g. `Int` in `Range(Int)` → concrete
            // instantiation. Tagged so the typechecker resolves and validates it.
            Expr::AnonymousTag(n, _) => {
                let sp = Spanned {
                    value: Expr::TypeNominal(*n, *span),
                    span_id: *span,
                };
                out.push((*n, ParameterKind::Tagged(Box::new(sp))));
            }
            _ => return None,
        }
    }
    Some(out)
}

fn parse_type_annotation_args(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Vec<Spanned<Expr>> {
    if !cursor.eat(&Token::ParenOpen) {
        return Vec::new();
    }

    let mut args = Vec::new();

    if !cursor.is_at(&Token::ParenClose) {
        args.push(expr_parser(cursor));
        while cursor.eat(&Token::Comma) {
            if cursor.is_at(&Token::ParenClose) {
                break;
            }
            args.push(expr_parser(cursor));
        }
    }

    cursor.expect(&Token::ParenClose);
    args
}

fn parse_bind_value(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
) -> (BindValue, Option<DocComment>) {
    // extern → BindValue::Extern
    if cursor.eat(&Token::Extern) {
        return (BindValue::Extern, None);
    }

    // Indent → multi-line body
    if cursor.is_at(&Token::Indent) {
        let _has_indent = cursor.eat(&Token::Indent);

        let exprs = parse_body_exprs(cursor, expr_parser);

        // Dedent is optional — return may appear inside or outside the indented block
        cursor.eat(&Token::Dedent);

        let ret = parse_return(cursor, expr_parser).unwrap_or_else(|| {
            cursor.error("expected 'return' after bind body", cursor.current_span());
            Return {
                value: None,
                span_id: cursor.current_span(),
            }
        });

        return (BindValue::Body { exprs, ret }, None);
    }

    // Otherwise → single expression with optional postfix doc comment
    let expr = expr_parser(cursor);
    // Only consume a doc comment if it's on the same line (no intervening newline),
    // otherwise it belongs to the next top-level declaration.
    let doc = if matches!(cursor.peek_at(0), Some(Token::DocComment(_))) {
        parse_doc_comment(cursor)
    } else {
        None
    };

    (BindValue::Expr(Box::new(expr)), doc)
}

fn parse_params(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
) -> (
    Option<Parameters>,
    IndexMap<Intern<String>, ParamConvention>,
) {
    if !cursor.is_at(&Token::ParenOpen) {
        return (None, IndexMap::new());
    }

    let mut params = Parameters::new();
    let mut conventions = IndexMap::new();
    cursor.advance();

    if cursor.is_at(&Token::ParenClose) {
        cursor.advance();
        return (Some(params), conventions);
    }

    if let Some((key, kind, conv)) = parse_one_param(cursor, expr_parser) {
        params.insert(key, kind);
        if conv != ParamConvention::Ref {
            conventions.insert(key, conv);
        }
    }

    while cursor.eat(&Token::Comma) {
        if cursor.is_at(&Token::ParenClose) {
            break;
        }
        if let Some((key, kind, conv)) = parse_one_param(cursor, expr_parser) {
            params.insert(key, kind);
            if conv != ParamConvention::Ref {
                conventions.insert(key, conv);
            }
        }
    }

    cursor.expect(&Token::ParenClose);
    (Some(params), conventions)
}

fn parse_one_param(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
) -> Option<(Intern<String>, ParameterKind, ParamConvention)> {
    // Positional: bare Tag → (tag_name, ParameterKind::Tagged(tag))
    if matches!(cursor.peek(), Some(Token::Tag(_))) {
        let sp = parse_type_expr(cursor, expr_parser)?;
        let key = Intern::<String>::from_ref(type_surface_mangle_name(&sp.value));
        return Some((
            key,
            ParameterKind::Tagged(Box::new(Spanned {
                value: sp.value.into(),
                span_id: sp.span_id,
            })),
            ParamConvention::Ref,
        ));
    }

    // Convention keyword before the parameter name:
    // `mut name Type` → Mutable
    // `own name Type` → Own
    let convention = if cursor.eat(&Token::Mut) {
        ParamConvention::Mut
    } else if cursor.eat(&Token::Own) {
        ParamConvention::Own
    } else {
        ParamConvention::Ref
    };

    // Named: id [Tag | id | : expr]
    let name = match cursor.peek() {
        Some(Token::Id(n)) => {
            let id = cursor.intern(n);
            cursor.advance();
            id
        }
        _ => return None,
    };

    let (name, kind) = crate::params::parse_param_after_name(cursor, expr_parser, name)?;
    Some((name, kind, convention))
}

pub(crate) fn parse_doc_comment(cursor: &mut TokenCursor) -> Option<DocComment> {
    let first = match cursor.peek()? {
        Token::DocComment(text) => {
            let stripped = text
                .strip_prefix("---")
                .map(|s| s.trim_start())
                .unwrap_or(text)
                .to_owned();
            cursor.advance();
            stripped
        }
        _ => return None,
    };

    // Fast path: single-line doc comment (most common)
    if !matches!(cursor.peek(), Some(Token::DocComment(_))) {
        let doc = DocComment { value: first };
        return if doc.is_empty() { None } else { Some(doc) };
    }

    let mut lines = vec![first];

    while let Some(Token::DocComment(text)) = cursor.peek() {
        let stripped = text
            .strip_prefix("---")
            .map(|s| s.trim_start())
            .unwrap_or(text)
            .to_owned();
        cursor.advance();
        lines.push(stripped);
    }

    let doc = DocComment {
        value: lines.join("\n"),
    };
    if doc.is_empty() { None } else { Some(doc) }
}

fn parse_body_exprs(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Vec<Spanned<Expr>> {
    let mut exprs = Vec::new();
    loop {
        super::body_trivia::skip_expr_body_trivia(cursor);
        match cursor.peek() {
            Some(t) if can_start_expr(t) => {
                // Fast path: in body context, Id followed by : or := is always a bind.
                // This bypasses the speculative looks_like_bind + checkpoint/rewind in parse_atom,
                // eliminating ~100 speculative parse_bind calls for large_mixed (40 functions × ~2-3 body binds).
                if matches!(t, Token::Id(_)) {
                    cursor.skip_newlines();
                    if matches!(cursor.peek_at(1), Some(Token::Colon) | Some(Token::ColonEq)) {
                        let start_pos = cursor.pos();
                        if let Some(bind) = parse_bind(cursor, expr_parser) {
                            let start_span = cursor.span_at(start_pos);
                            cursor.consume_trailing_newline();
                            let end_span = cursor.last_consumed_span();
                            exprs.push(Spanned {
                                value: Expr::Bind(Box::new(bind)),
                                span_id: cursor.merge_span(start_span, end_span),
                            });
                            continue;
                        }
                        // parse_bind failed on Id: — extremely rare, rewind and fall through
                        cursor.rewind(start_pos);
                    }
                }

                let pos_before = cursor.pos();
                let expr = expr_parser(cursor);
                exprs.push(expr);
                if cursor.pos() == pos_before {
                    cursor.error("expression parser made no progress", cursor.current_span());
                    cursor.advance();
                }
            }
            _ => break,
        }
    }
    exprs
}

fn can_start_expr(token: &Token) -> bool {
    matches!(
        token,
        Token::Id(_)
            | Token::Tag(_)
            | Token::Int(_)
            | Token::Float(_)
            | Token::String(_)
            | Token::SelfInstance
            | Token::Minus
            | Token::At
            | Token::Caret
            | Token::Star
            | Token::ParenOpen
            | Token::If
            | Token::When
            | Token::For
            | Token::While
            | Token::FormatStringDelim
    )
}
