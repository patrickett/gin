use internment::Intern;
use lexer::Token;

use ast::span::SpanId;
use ast::{ParameterKind, Parameters, Spanned, TypeExpr, type_surface_mangle_name};

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
    if cursor.is_at(&Token::BracketOpen) {
        return parse_list_pattern(cursor, expr_parser);
    }
    if cursor.is_at(&Token::ParenOpen) {
        return parse_paren_tuple_pattern(cursor, expr_parser);
    }
    parse_type_expr_with(cursor, expr_parser)
}

/// `(A, B, …)` or `()` in `when … is` pattern position.
fn parse_paren_tuple_pattern(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
) -> Option<Spanned<TypeExpr>> {
    let start = cursor.peek_span()?;
    cursor.expect(&Token::ParenOpen)?;
    if cursor.eat(&Token::ParenClose) {
        let end = cursor.last_consumed_span();
        return Some(Spanned {
            value: TypeExpr::Unit,
            span_id: cursor.merge_span(start, end),
        });
    }
    let first = parse_pattern_type_expr(cursor, expr_parser)?;
    let mut elems = vec![first];
    while cursor.eat(&Token::Comma) {
        elems.push(parse_pattern_type_expr(cursor, expr_parser)?);
    }
    cursor.expect(&Token::ParenClose)?;
    let end = cursor.last_consumed_span();
    Some(Spanned {
        value: TypeExpr::Tuple(elems),
        span_id: cursor.merge_span(start, end),
    })
}

fn parse_list_pattern(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<Spanned<TypeExpr>> {
    let start = cursor.peek_span()?;
    cursor.expect(&Token::BracketOpen)?;
    if cursor.eat(&Token::BracketClose) {
        let end = cursor.last_consumed_span();
        return Some(Spanned {
            value: TypeExpr::ListEmpty,
            span_id: cursor.merge_span(start, end),
        });
    }

    let mut heads = vec![parse_pattern_type_expr(cursor, expr_parser)?];
    while cursor.eat(&Token::Comma) {
        if cursor.is_at(&Token::Infer) {
            cursor.advance();
            let tail = parse_pattern_type_expr(cursor, expr_parser)?;
            cursor.expect(&Token::BracketClose)?;
            let end = cursor.last_consumed_span();
            return Some(fold_list_cons_pattern(
                heads,
                tail,
                cursor.merge_span(start, end),
            ));
        }
        if cursor.is_at(&Token::BracketClose) {
            break;
        }
        heads.push(parse_pattern_type_expr(cursor, expr_parser)?);
    }
    cursor.expect(&Token::BracketClose)?;
    let end = cursor.last_consumed_span();
    let empty_tail = Spanned {
        value: TypeExpr::ListEmpty,
        span_id: end,
    };
    Some(fold_list_cons_pattern(
        heads,
        empty_tail,
        cursor.merge_span(start, end),
    ))
}

fn fold_list_cons_pattern(
    heads: Vec<Spanned<TypeExpr>>,
    tail: Spanned<TypeExpr>,
    span: SpanId,
) -> Spanned<TypeExpr> {
    let mut result = tail;
    for head in heads.into_iter().rev() {
        result = Spanned {
            value: TypeExpr::ListCons {
                head: Box::new(head),
                tail: Box::new(result),
            },
            span_id: span,
        };
    }
    result
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

    // Reference type: `ref T` or `mut T`
    if cursor.eat(&Token::Ref) {
        let inner = parse_type_expr_with(cursor, expr_parser)?;
        let end_span = inner.span_id;
        let span = cursor.merge_span(start_span, end_span);
        return Some(Spanned {
            value: TypeExpr::Ref {
                inner: Box::new(inner),
                mutable: false,
            },
            span_id: span,
        });
    }
    if cursor.eat(&Token::Mut) {
        let inner = parse_type_expr_with(cursor, expr_parser)?;
        let end_span = inner.span_id;
        let span = cursor.merge_span(start_span, end_span);
        return Some(Spanned {
            value: TypeExpr::Ref {
                inner: Box::new(inner),
                mutable: true,
            },
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
                if !params.params.is_empty() {
                    let name = *path.segments.last().unwrap_or(&path.root);
                    // `span` on the type is the head name only (diagnostics); full site is `span_id`.
                    let name_span = path.span_id;
                    let (params, param_spans) = params.into_ordered_param_spans();
                    return Some(Spanned {
                        value: TypeExpr::Generic {
                            name,
                            params,
                            param_spans,
                            span: name_span,
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

    let (name, name_span) = match *cursor.peek()? {
        Token::Tag(n) => {
            let name = cursor.intern(n);
            let span = cursor.peek_span()?;
            cursor.advance();
            (name, span)
        }
        Token::Id(n) => {
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
        if !params.params.is_empty() {
            let (params, param_spans) = params.into_ordered_param_spans();
            return Some(Spanned {
                value: TypeExpr::Generic {
                    name,
                    params,
                    param_spans,
                    span: name_span,
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

type OrderedTagParams = (
    Vec<(Intern<String>, ParameterKind)>,
    Vec<(Intern<String>, ast::SpanId)>,
);

/// Ordered type parameters plus per-name spans for IDE hover.
struct ParsedTagParams {
    params: Parameters,
    param_spans: Vec<(Intern<String>, ast::SpanId)>,
}

impl ParsedTagParams {
    fn into_ordered_param_spans(self) -> OrderedTagParams {
        let mut params = Vec::with_capacity(self.params.len());
        let param_spans = self.param_spans;
        for (name, kind) in self.params {
            params.push((name, kind));
        }
        (params, param_spans)
    }
}

fn parse_tag_type_params_delimited(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
    open_token: Token<'static>,
    close_token: Token<'static>,
) -> ParsedTagParams {
    if cursor.expect(&open_token).is_none() {
        return ParsedTagParams {
            params: Parameters::new(),
            param_spans: Vec::new(),
        };
    }

    let mut params = Parameters::new();
    let mut param_spans = Vec::new();
    let mut seen_default = false;

    if !cursor.is_at(&close_token) {
        if let Some((name, kind, span)) = parse_one_tag_param(cursor, expr_parser) {
            seen_default = matches!(kind, ParameterKind::Default(_));
            param_spans.push((name, span));
            params.insert(name, kind);
        }
        while cursor.eat(&Token::Comma) {
            if cursor.is_at(&close_token) {
                break;
            }
            if let Some((name, kind, span)) = parse_one_tag_param(cursor, expr_parser) {
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
                param_spans.push((name, span));
                params.insert(name, kind);
            }
        }
    }

    cursor.expect(&close_token);
    ParsedTagParams {
        params,
        param_spans,
    }
}

fn parse_one_tag_param(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
) -> Option<(Intern<String>, ParameterKind, ast::SpanId)> {
    if matches!(cursor.peek(), Some(&Token::Tag(_))) {
        let sp = parse_type_expr(cursor, expr_parser)?;
        let key = Intern::<String>::from_ref(type_surface_mangle_name(&sp.value));
        return Some((
            key,
            ParameterKind::Tagged(Box::new(Spanned {
                value: sp.value,
                span_id: sp.span_id,
            })),
            sp.span_id,
        ));
    }

    let (name, name_span) = match cursor.peek()? {
        &Token::Id(n) => {
            let span = cursor.peek_span()?;
            let id = cursor.intern(n);
            cursor.advance();
            (id, span)
        }
        _ => return None,
    };

    if matches!(cursor.peek(), Some(&Token::Tag(_))) {
        let sp = parse_type_expr(cursor, expr_parser)?;
        return Some((
            name,
            ParameterKind::Tagged(Box::new(Spanned {
                value: sp.value,
                span_id: sp.span_id,
            })),
            name_span,
        ));
    }

    if cursor.eat(&Token::Colon) {
        let expr = expr_parser(cursor);
        return Some((name, ParameterKind::Default(Box::new(expr)), name_span));
    }

    Some((name, ParameterKind::Generic, name_span))
}
