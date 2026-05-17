use internment::Intern;
use lexer::Token;

use ast::{
    Expr, ParameterKind, Parameters, Spanned, TagCall, TypeExpr, Typed, type_surface_mangle_name,
};

use crate::cursor::TokenCursor;
use crate::expr::ExprFn;
use crate::path::parse_tag_variant_path;

/// Type surface after `is` in `if … is …` / `when … is …` — structural [`TypeExpr`].
#[inline]
pub fn parse_is_pattern_tag(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
) -> Option<Spanned<TypeExpr>> {
    parse_type_expr_with(cursor, expr_parser)
}

/// Parse a capitalized type path (`Str`, `Maybe(T)`, `Mod.Item`) into structural type [`TypeExpr`].
pub fn parse_type_expr(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<Spanned<TypeExpr>> {
    parse_type_expr_with(cursor, expr_parser)
}

/// Parse a type-shaped variant/pattern surface.
///
/// Type arguments use parentheses (`Maybe(x)`), as do variant and `is`
/// pattern payloads (`Some(x)`).
pub fn parse_pattern_type_expr(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
) -> Option<Spanned<TypeExpr>> {
    parse_type_expr_with(cursor, expr_parser)
}

fn parse_type_expr_with(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
) -> Option<Spanned<TypeExpr>> {
    let start_span = cursor.current_span();

    // Pointer type: `@TypeExpr`
    if cursor.eat(&Token::At) {
        let inner = parse_type_expr_with(cursor, expr_parser)?;
        let end_span = inner.span_id;
        let span = cursor.merge_span(start_span, end_span);
        return Some(Spanned {
            value: TypeExpr::Pointer(Box::new(inner)),
            span_id: span,
        });
    }

    // Unit type: `()`
    if cursor.is_at(&Token::ParenOpen) && cursor.peek_at(1) == Some(&Token::ParenClose) {
        let open_span = cursor.peek_span()?;
        cursor.advance(); // eat (
        cursor.advance(); // eat )
        let close_span = cursor.last_consumed_span();
        let span = cursor.merge_span(open_span, close_span);
        return Some(Spanned {
            value: TypeExpr::Unit,
            span_id: span,
        });
    }

    if cursor.peek_at(1) == Some(&Token::Dot) {
        let qual_checkpoint = cursor.checkpoint();
        if let Some(path) = parse_tag_variant_path(cursor) {
            if cursor.is_at(&Token::ParenOpen) {
                let params = parse_tag_type_params_delimited(
                    cursor,
                    expr_parser,
                    Token::ParenOpen,
                    Token::ParenClose,
                );
                let end_span = cursor.last_consumed_span();
                let span = cursor.merge_span(start_span, end_span);
                if !params.is_empty() {
                    let name = *path.segments.last().unwrap_or(&path.root);
                    return Some(Spanned {
                        value: TypeExpr::Generic {
                            name,
                            params: params.into_iter().collect(),
                            span,
                        },
                        span_id: span,
                    });
                }
                return Some(Spanned {
                    value: TypeExpr::Qualified(path),
                    span_id: span,
                });
            }
            let span = path.span_id;
            return Some(Spanned {
                value: TypeExpr::Qualified(path),
                span_id: span,
            });
        }
        cursor.rewind(qual_checkpoint);
    }

    let (name, name_span) = match cursor.peek()? {
        &Token::Tag(n) => {
            let name = cursor.intern(n);
            let span = cursor.peek_span()?;
            cursor.advance();
            (name, span)
        }
        &Token::Id(n) => {
            // Lowercase identifiers in type position are type variables
            // (e.g., `x` in `@x` or `x` in `Range[x]`).
            let name = cursor.intern(n);
            let span = cursor.peek_span()?;
            cursor.advance();
            (name, span)
        }
        _ => return None,
    };

    if cursor.is_at(&Token::ParenOpen) {
        let params = parse_tag_type_params_delimited(
            cursor,
            expr_parser,
            Token::ParenOpen,
            Token::ParenClose,
        );
        let end_span = cursor.last_consumed_span();
        let span = cursor.merge_span(name_span, end_span);
        if !params.is_empty() {
            return Some(Spanned {
                value: TypeExpr::Generic {
                    name,
                    params: params.into_iter().collect(),
                    span,
                },
                span_id: span,
            });
        }
        return Some(Spanned {
            value: TypeExpr::Nominal(name, span),
            span_id: span,
        });
    }

    Some(Spanned {
        value: TypeExpr::Nominal(name, name_span),
        span_id: name_span,
    })
}

fn parse_tag_type_params_delimited(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
    open_token: Token<'static>,
    close_token: Token<'static>,
) -> Parameters {
    if cursor.expect(&open_token).is_none() {
        return Parameters::new();
    }

    let mut params = Parameters::new();
    let mut seen_default = false;

    if !cursor.is_at(&close_token) {
        if let Some((name, kind)) = parse_one_tag_param(cursor, expr_parser) {
            seen_default = matches!(kind, ParameterKind::Default(_));
            params.insert(name, kind);
        }
        while cursor.eat(&Token::Comma) {
            if cursor.is_at(&close_token) {
                break;
            }
            if let Some((name, kind)) = parse_one_tag_param(cursor, expr_parser) {
                // NOTE: Named type arguments (e.g. `Box(a: BumpAllocator)` to skip
                // a defaulted positional param) would allow omitting defaults without
                // triggering this flaw. Currently only positional type args are supported.
                if seen_default && !matches!(kind, ParameterKind::Default(_)) {
                    cursor.error(
                        format!(
                            "positional type parameter `{}` appears after a default parameter",
                            name.as_str()
                        ),
                        cursor.current_span(),
                    );
                }
                if matches!(kind, ParameterKind::Default(_)) {
                    seen_default = true;
                }
                params.insert(name, kind);
            }
        }
    }

    cursor.expect(&close_token);
    params
}

fn parse_one_tag_param(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
) -> Option<(Intern<String>, ParameterKind)> {
    if matches!(cursor.peek(), Some(&Token::Tag(_))) {
        let sp = parse_type_expr(cursor, expr_parser)?;
        let key = Intern::<String>::from_ref(type_surface_mangle_name(&sp.value));
        return Some((
            key,
            ParameterKind::Tagged(Box::new(Spanned {
                value: sp.value.into(),
                span_id: sp.span_id,
            })),
        ));
    }

    let name = match cursor.peek()? {
        &Token::Id(n) => {
            let id = cursor.intern(n);
            cursor.advance();
            id
        }
        _ => return None,
    };

    if matches!(cursor.peek(), Some(&Token::Tag(_))) {
        let sp = parse_type_expr(cursor, expr_parser)?;
        return Some((
            name,
            ParameterKind::Tagged(Box::new(Spanned {
                value: sp.value.into(),
                span_id: sp.span_id,
            })),
        ));
    }

    if cursor.eat(&Token::Colon) {
        let expr = expr_parser(cursor);
        return Some((name, ParameterKind::Default(Box::new(expr))));
    }

    Some((name, ParameterKind::Generic))
}

#[allow(dead_code)]
pub fn parse_tag_call(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<TagCall> {
    if cursor.peek_at(1) == Some(&Token::Dot) {
        let qual_checkpoint = cursor.checkpoint();
        if let Some(path) = parse_tag_variant_path(cursor) {
            let variant_name = *path.segments.last().unwrap_or(&path.root);
            if cursor.is_at(&Token::ParenOpen) {
                let args = parse_call_args(cursor, expr_parser);
                return Some(TagCall {
                    name: variant_name,
                    qual_path: Some(path),
                    args,
                });
            }
            return Some(TagCall {
                name: variant_name,
                qual_path: Some(path),
                args: Vec::new(),
            });
        }
        cursor.rewind(qual_checkpoint);
    }

    let (name, _name_span) = match cursor.peek()? {
        &Token::Tag(n) => {
            let name = cursor.intern(n);
            let _span = cursor.peek_span()?;
            cursor.advance();
            (name, _span)
        }
        _ => return None,
    };

    if cursor.is_at(&Token::ParenOpen) {
        let args = parse_call_args(cursor, expr_parser);
        return Some(TagCall {
            name,
            qual_path: None,
            args,
        });
    }

    Some(TagCall {
        name,
        qual_path: None,
        args: Vec::new(),
    })
}

#[allow(dead_code)]
fn parse_call_args(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Vec<Typed<Expr>> {
    if cursor.expect(&Token::ParenOpen).is_none() {
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
