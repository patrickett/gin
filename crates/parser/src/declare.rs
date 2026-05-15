use crate::cursor::TokenCursor;
use crate::expr::ExprFn;
use crate::expr::literal::parse_literal;
use ast::span::SpanId;
use ast::{
    Declare, DeclareValue, DocComment, ParameterKind, Parameters, Spanned, TypeExpr, Variant,
    type_surface_mangle_name,
};

use crate::tag::{parse_pattern_type_expr, parse_type_expr};
use i256::I256;
use internment::Intern;
use lexer::Token;

/// asd
pub fn parse_declare(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<Declare> {
    let doc_before = parse_doc_comment(cursor);
    let attrs = parse_declare_attributes(cursor);
    cursor.eat(&Token::Indent);

    let (name, name_span) = match cursor.peek()? {
        &Token::Tag(n) => {
            let name = cursor.intern(n);
            let span = cursor.peek_span()?;
            cursor.advance();
            (name, span)
        }
        _ => return None,
    };

    let params = parse_params_for_declare(cursor, expr_parser);

    if cursor.eat(&Token::Is) {
        let doc_after_is = parse_doc_comment(cursor);
        cursor.eat(&Token::Indent);

        let (value, doc_after_value) = parse_is_rhs(cursor, expr_parser);
        cursor.eat(&Token::Dedent);

        let doc = doc_after_value.or(doc_after_is).or(doc_before);
        let mut decl = Declare::new(name, name_span, value)
            .with_params(params)
            .with_doc(doc);
        if let Some(a) = attrs {
            decl = decl.with_attributes(a);
        }
        Some(decl)
    } else if cursor.eat(&Token::Has) {
        parse_doc_comment(cursor);
        cursor.eat(&Token::Indent);

        let value = parse_has_rhs(cursor, expr_parser);
        cursor.eat(&Token::Dedent);

        let mut decl = Declare::new(name, name_span, value)
            .with_params(params)
            .with_doc(doc_before);
        if let Some(a) = attrs {
            decl = decl.with_attributes(a);
        }
        Some(decl)
    } else {
        cursor.error("expected 'is' or 'has'", cursor.current_span());
        None
    }
}

fn parse_declare_attributes(cursor: &mut TokenCursor) -> Option<ast::DeclareAttributes> {
    if !cursor.is_at(&Token::Pound) {
        return None;
    }
    cursor.advance();

    cursor.expect(&Token::BracketOpen)?;

    let mut items: Vec<ast::AttributeItem> = Vec::new();

    if !cursor.is_at(&Token::BracketClose) {
        loop {
            if let Some(item) = crate::expr::bind::parse_one_attribute_item(cursor) {
                items.push(item);
            }
            if cursor.eat(&Token::Comma) {
                continue;
            }
            break;
        }
    }

    cursor.expect(&Token::BracketClose);

    let mut attrs = ast::DeclareAttributes {
        raw_attributes: Some(items),
        ..Default::default()
    };
    attrs.extract_intrinsic_attributes();
    Some(attrs)
}

fn parse_doc_comment(cursor: &mut TokenCursor) -> Option<DocComment> {
    let text = match cursor.peek()? {
        &Token::DocComment(t) => t,
        _ => return None,
    };
    cursor.advance();

    let first = text
        .strip_prefix("---")
        .map(|s| s.trim_start())
        .unwrap_or(text)
        .to_owned();

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

fn parse_has_rhs(cursor: &mut TokenCursor, expr_parser: ExprFn) -> DeclareValue {
    if cursor.is_at(&Token::ParenOpen)
        && let Some(params) = parse_params_for_declare(cursor, expr_parser)
    {
        return DeclareValue::Record(params);
    }

    if let Some(sp) = parse_type_expr(cursor, expr_parser) {
        return DeclareValue::Alias(Box::new(sp));
    }

    cursor.error(
        "expected record parameters or tag after 'has'",
        cursor.current_span(),
    );
    DeclareValue::Record(Parameters::new())
}

fn parse_is_rhs(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
) -> (DeclareValue, Option<DocComment>) {
    // InRange: `in` N...M
    if cursor.eat(&Token::In) {
        if let Some((a, b)) = parse_int_range(cursor) {
            return (DeclareValue::InRange(a, b), None);
        }
        cursor.error("expected integer range after 'in'", cursor.current_span());
        return (
            DeclareValue::InRange(I256::from_u128(0), I256::from_u128(0)),
            None,
        );
    }

    // Range: [-]Int...[-]Int
    if matches!(cursor.peek(), Some(Token::Int(_)) | Some(Token::Minus)) {
        let checkpoint = cursor.checkpoint();
        if let Some((a, b)) = parse_int_range(cursor) {
            return (DeclareValue::Range(a, b), None);
        }
        cursor.rewind(checkpoint);
    }

    // Union or Alias: starts with optional doc then a type pattern or literal
    let checkpoint = cursor.checkpoint();
    let first_doc = parse_doc_comment(cursor);

    // Try literal variant first (e.g. 'debug'), then tag pattern (e.g. Some(x))
    let first_shape: Option<Spanned<TypeExpr>> = 'union_check: {
        // Check if we're at a literal that could be a union variant
        if matches!(
            cursor.peek(),
            Some(Token::String(_)) | Some(Token::Int(_)) | Some(Token::Float(_))
        ) {
            let cp = cursor.checkpoint();
            if let Some(Spanned {
                value: lit,
                span_id: span,
            }) = parse_literal(cursor)
            {
                // Skip Indent/Dedent tokens that may precede a continuation `or`
                while cursor.eat(&Token::Indent) || cursor.eat(&Token::Dedent) {}
                if cursor.is_at(&Token::Or) {
                    break 'union_check Some(Spanned {
                        value: TypeExpr::Literal(lit, span),
                        span_id: span,
                    });
                }
            }
            // Not a union — rewind and try tag pattern
            cursor.rewind(cp);
        }
        // Standard checkpoint approach for tag patterns
        let cp = cursor.checkpoint();
        if let Some(shape) = parse_pattern_type_expr(cursor, expr_parser) {
            // Skip Indent/Dedent tokens that may precede a continuation `or`
            // on an indented line (e.g. `Type is Variant(x)
            //                            or OtherVariant(y)`)
            while cursor.eat(&Token::Indent) || cursor.eat(&Token::Dedent) {}
            if cursor.is_at(&Token::Or) {
                break 'union_check Some(shape);
            }
        }
        cursor.rewind(cp);
        None
    };

    if let Some(first_shape) = first_shape {
        // Union: Tag (or Tag)+
        let first_post_doc = parse_doc_comment(cursor);

        let first_variant = make_variant(first_doc, first_shape, first_post_doc);
        let mut variants = vec![first_variant];

        while cursor.eat(&Token::Or) {
            let doc_on_or_line = parse_doc_comment(cursor);
            cursor.eat(&Token::Indent);

            match parse_variant(cursor, expr_parser) {
                Some(next_variant) => {
                    attach_doc_to_previous(&mut variants, doc_on_or_line);
                    variants.push(next_variant);
                }
                None => {
                    cursor.error("expected variant after 'or'", cursor.current_span());
                    break;
                }
            }
        }

        // Only consume doc comment if immediately after last variant (same line)
        let post_doc = if matches!(cursor.peek_at(0), Some(Token::DocComment(_))) {
            parse_doc_comment(cursor)
        } else {
            None
        };
        return (DeclareValue::Union { variants }, post_doc);
    }

    // Not a union — rewind to before any doc comment and try as alias
    cursor.rewind(checkpoint);

    // Single literal variant (e.g., `X0 is 'x0'`, `Status is 'ready'`)
    if matches!(
        cursor.peek(),
        Some(Token::String(_)) | Some(Token::Int(_)) | Some(Token::Float(_))
    ) && let Some(variant) = parse_variant(cursor, expr_parser)
    {
        // Only consume doc comment if immediately after the value (same line)
        let doc = if matches!(cursor.peek_at(0), Some(Token::DocComment(_))) {
            parse_doc_comment(cursor)
        } else {
            None
        };
        return (
            DeclareValue::Union {
                variants: vec![variant],
            },
            doc,
        );
    }

    // Try pattern type expression (allows parens for params): `Register(value: 'rax')`
    if let Some(sp) = parse_pattern_type_expr(cursor, expr_parser) {
        let doc = if matches!(cursor.peek_at(0), Some(Token::DocComment(_))) {
            parse_doc_comment(cursor)
        } else {
            None
        };
        return (DeclareValue::Alias(Box::new(sp)), doc);
    }

    if let Some(sp) = parse_type_expr(cursor, expr_parser) {
        let doc = if matches!(cursor.peek_at(0), Some(Token::DocComment(_))) {
            parse_doc_comment(cursor)
        } else {
            None
        };
        return (DeclareValue::Alias(Box::new(sp)), doc);
    }

    cursor.error(
        "expected type declaration body after 'is'",
        cursor.current_span(),
    );
    (
        DeclareValue::Alias(Box::new(Spanned {
            value: TypeExpr::Nominal(Intern::new(String::new()), cursor.current_span()),
            span_id: cursor.current_span(),
        })),
        None,
    )
}

fn parse_variant(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<Variant> {
    let doc_before = parse_doc_comment(cursor);

    // Try literal variant first (string, int, or float)
    if matches!(
        cursor.peek(),
        Some(Token::String(_)) | Some(Token::Int(_)) | Some(Token::Float(_))
    ) && let Some(Spanned {
        value: lit,
        span_id: span,
    }) = parse_literal(cursor)
    {
        let shape = Box::new(Spanned {
            value: TypeExpr::Literal(lit, span),
            span_id: span,
        });
        // Only consume doc comment if immediately after the value (same line)
        let doc_after = if matches!(cursor.peek_at(0), Some(Token::DocComment(_))) {
            parse_doc_comment(cursor)
        } else {
            None
        };
        let doc = doc_after.or(doc_before);
        return Some(match doc.filter(|d| !d.is_empty()) {
            Some(d) => Variant::Local {
                doc_comment: Some(d),
                shape,
            },
            None => Variant::External(shape),
        });
    }

    // Fall through to tag-based pattern
    let shape = parse_pattern_type_expr(cursor, expr_parser)?;
    let sp = Box::new(shape);

    // Only consume doc comment if immediately after the variant (same line)
    let doc_after = if matches!(cursor.peek_at(0), Some(Token::DocComment(_))) {
        parse_doc_comment(cursor)
    } else {
        None
    };

    let doc = doc_after.or(doc_before);
    Some(match doc.filter(|d| !d.is_empty()) {
        Some(d) => Variant::Local {
            doc_comment: Some(d),
            shape: sp,
        },
        None => Variant::External(sp),
    })
}

fn make_variant(
    doc_before: Option<DocComment>,
    shape: Spanned<TypeExpr>,
    doc_after: Option<DocComment>,
) -> Variant {
    let doc = doc_after.or(doc_before);
    let sp = Box::new(shape);
    match doc.filter(|d| !d.is_empty()) {
        Some(d) => Variant::Local {
            doc_comment: Some(d),
            shape: sp,
        },
        None => Variant::External(sp),
    }
}

fn attach_doc_to_previous(variants: &mut [Variant], doc: Option<DocComment>) {
    if let Some(doc) = doc.filter(|d| !d.is_empty())
        && let Some(prev) = variants.last_mut()
    {
        let placeholder = Variant::External(Box::new(Spanned {
            value: TypeExpr::Nominal(Intern::new(String::new()), SpanId::INVALID),
            span_id: SpanId::INVALID,
        }));
        let prev_owned = std::mem::replace(prev, placeholder);
        *prev = match prev_owned {
            Variant::External(shape) => Variant::Local {
                doc_comment: Some(doc),
                shape,
            },
            Variant::Local {
                mut doc_comment,
                shape,
            } => {
                if doc_comment.is_none() {
                    doc_comment = Some(doc);
                }
                Variant::Local { doc_comment, shape }
            }
        };
    }
}

fn parse_int_range(cursor: &mut TokenCursor) -> Option<(I256, I256)> {
    let start = parse_signed_int(cursor)?;
    if !cursor.eat(&Token::Infer) {
        return None;
    }
    let end = parse_signed_int(cursor)?;
    Some((start, end))
}

fn parse_signed_int(cursor: &mut TokenCursor) -> Option<I256> {
    let neg = cursor.eat(&Token::Minus);
    match cursor.peek()? {
        &Token::Int(n) => {
            cursor.advance();
            if neg {
                Some(-I256::from_u128(n))
            } else {
                Some(I256::from_u128(n))
            }
        }
        _ => None,
    }
}

fn parse_params_for_declare(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<Parameters> {
    let (_open_token, close_token) = match cursor.peek() {
        Some(Token::ParenOpen) => (Token::ParenOpen, Token::ParenClose),
        Some(Token::BracketOpen) => (Token::BracketOpen, Token::BracketClose),
        _ => return None,
    };
    cursor.advance();

    let mut params = Parameters::new();

    if cursor.is_at(&close_token) {
        cursor.advance();
        return Some(params);
    }

    if let Some((key, kind)) = parse_one_declare_param(cursor, expr_parser) {
        params.insert(key, kind);
    }

    while cursor.eat(&Token::Comma) {
        if cursor.is_at(&close_token) {
            break;
        }
        if let Some((key, kind)) = parse_one_declare_param(cursor, expr_parser) {
            params.insert(key, kind);
        }
    }

    cursor.expect(&close_token);
    Some(params)
}

fn parse_one_declare_param(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
) -> Option<(Intern<String>, ParameterKind)> {
    // Positional: bare Tag
    if matches!(cursor.peek(), Some(Token::Tag(_))) {
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

    // Named: id [Tag | id | : expr]
    let name = match cursor.peek() {
        Some(Token::Id(n)) => {
            let id = cursor.intern(n);
            cursor.advance();
            id
        }
        _ => return None,
    };

    // Shared helper supports `id Tag`, `id id` (type variable, e.g.
    // `Range[x] has (start x, end x)`), `id: expr`, and bare `id` (generic).
    crate::params::parse_param_after_name(cursor, expr_parser, name)
}
