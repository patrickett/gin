use indexmap::IndexMap;
use internment::Intern;
use lexer::Token;

use ast::{Expr, ParameterKind, Parameters, Spanned, Tag, TagCall};

use crate::cursor::TokenCursor;
use crate::expr::ExprFn;
use crate::path::parse_tag_variant_path;

pub fn parse_tag(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<Tag> {
    let start_span = cursor.current_span();

    // Try qualified path first: Tag.Tag[.Tag...]
    // 2-token lookahead guard: a qualified path requires at least `Tag .`
    if cursor.peek_at(1) == Some(&Token::Dot) {
        // checkpoint because parse_tag_variant_path may partially consume on failure
        let qual_checkpoint = cursor.checkpoint();
        if let Some(path) = parse_tag_variant_path(cursor) {
            if cursor.is_at(&Token::ParenOpen) {
                let params = parse_tag_params(cursor, expr_parser);
                if !params.is_empty() {
                    let name = *path.segments.last().unwrap_or(&path.root);
                    let end_span = cursor.last_consumed_span();
                    let span = cursor.merge_span(start_span, end_span);
                    return Some(Tag::Generic(name, params, span));
                }
            }
            return Some(Tag::Qualified(path));
        }
        cursor.rewind(qual_checkpoint);
    }

    // Simple tag: Tag or Tag(params)
    let (name, name_span) = match cursor.peek()? {
        &Token::Tag(n) => {
            let name = cursor.intern(n);
            let span = cursor.peek_span()?;
            cursor.advance();
            (name, span)
        }
        _ => return None,
    };

    if cursor.is_at(&Token::ParenOpen) {
        let params = parse_tag_params(cursor, expr_parser);
        let end_span = cursor.last_consumed_span();
        let span = cursor.merge_span(name_span, end_span);
        if !params.is_empty() {
            return Some(Tag::Generic(name, params, span));
        }
        return Some(Tag::Nominal(name, span));
    }

    Some(Tag::Nominal(name, name_span))
}

fn parse_tag_params(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Parameters {
    if cursor.expect(&Token::ParenOpen).is_none() {
        return Parameters::new();
    }

    let mut params = IndexMap::new();

    if !cursor.is_at(&Token::ParenClose) {
        if let Some(param) = parse_one_tag_param(cursor, expr_parser) {
            params.insert(param.0, param.1);
        }
        while cursor.eat(&Token::Comma) {
            if cursor.is_at(&Token::ParenClose) {
                break;
            }
            if let Some(param) = parse_one_tag_param(cursor, expr_parser) {
                params.insert(param.0, param.1);
            }
        }
    }

    cursor.expect(&Token::ParenClose);
    params
}

fn parse_one_tag_param(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
) -> Option<(Intern<String>, ParameterKind)> {
    // Positional: bare Tag → (tag_name, ParameterKind::Tagged(tag))
    if matches!(cursor.peek(), Some(&Token::Tag(_))) {
        let tag = parse_tag(cursor, expr_parser)?;
        let key = match &tag {
            Tag::Nominal(name, _) | Tag::Generic(name, _, _) => *name,
            Tag::Qualified(path) => *path.segments.last().unwrap_or(&path.root),
        };
        return Some((key, ParameterKind::Tagged(tag)));
    }

    // Named: id | id Tag | id: expr
    let name = match cursor.peek()? {
        &Token::Id(n) => {
            let id = cursor.intern(n);
            cursor.advance();
            id
        }
        _ => return None,
    };

    // id Tag → tagged
    if matches!(cursor.peek(), Some(&Token::Tag(_))) {
        let tag = parse_tag(cursor, expr_parser)?;
        return Some((name, ParameterKind::Tagged(tag)));
    }

    // id: expr → default
    if cursor.eat(&Token::Colon) {
        let expr = expr_parser(cursor);
        return Some((name, ParameterKind::Default(expr)));
    }

    // id → generic
    Some((name, ParameterKind::Generic))
}

#[allow(dead_code)]
pub fn parse_tag_call(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<TagCall> {
    let start_span = cursor.current_span();

    // Try qualified: Tag.Tag[(args)]
    // 2-token lookahead guard: a qualified path requires at least `Tag .`
    if cursor.peek_at(1) == Some(&Token::Dot) {
        let qual_checkpoint = cursor.checkpoint();
        if let Some(path) = parse_tag_variant_path(cursor) {
            let variant_name = *path.segments.last().unwrap_or(&path.root);
            if cursor.is_at(&Token::ParenOpen) {
                let args = parse_call_args(cursor, expr_parser);
                let end_span = cursor.last_consumed_span();
                let span = cursor.merge_span(start_span, end_span);
                return Some(TagCall {
                    name: variant_name,
                    qual_path: Some(path),
                    args,
                    span,
                });
            }
            let span = path.span;
            return Some(TagCall {
                name: variant_name,
                qual_path: Some(path),
                args: Vec::new(),
                span,
            });
        }
        cursor.rewind(qual_checkpoint);
    }

    // Simple: Tag[(args)]
    let (name, name_span) = match cursor.peek()? {
        &Token::Tag(n) => {
            let name = cursor.intern(n);
            let span = cursor.peek_span()?;
            cursor.advance();
            (name, span)
        }
        _ => return None,
    };

    if cursor.is_at(&Token::ParenOpen) {
        let args = parse_call_args(cursor, expr_parser);
        let end_span = cursor.last_consumed_span();
        let span = cursor.merge_span(name_span, end_span);
        return Some(TagCall {
            name,
            qual_path: None,
            args,
            span,
        });
    }

    Some(TagCall {
        name,
        qual_path: None,
        args: Vec::new(),
        span: name_span,
    })
}

#[allow(dead_code)]
fn parse_call_args(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Vec<Spanned<Expr>> {
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
