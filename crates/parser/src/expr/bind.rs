use indexmap::IndexMap;
use internment::Intern;
use lexer::Token;

use ast::{
    AttributeItem, Bind, BindAttributes, BindValue, DocComment, Expr, GroupParam, ModPath,
    ParamConvention, ParameterKind, Parameters, Return, Spanned, TraitBound, TypeExpr, Typed,
    type_surface_mangle_name,
};

use super::ExprFn;
use super::body::parse_body_exprs;
use super::control::parse_return;
use crate::cursor::TokenCursor;
use crate::path::{parse_id, parse_tag_variant_path};
use crate::tag::parse_type_expr;

type ReturnTypePart = (
    Option<Intern<String>>,
    Option<Box<Spanned<TypeExpr>>>,
    Option<(Intern<String>, Vec<Typed<Expr>>)>,
    Option<Spanned<ModPath>>,
);

type ParsedParams = (
    Option<Parameters>,
    IndexMap<Intern<String>, ParamConvention>,
    IndexMap<Intern<String>, Intern<String>>,
);

type ParsedOneParam = (
    Intern<String>,
    ParameterKind,
    ParamConvention,
    Option<Intern<String>>,
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

    let group_params = parse_group_params(cursor);
    let (params, conventions, param_groups) = parse_params(cursor, expr_parser);

    let (return_type_name, return_tag, type_annotation, type_annotation_qual) =
        parse_return_type_part(cursor, expr_parser);

    let trait_bounds = parse_trait_bounds(cursor, expr_parser);

    // Handle `extern` binds: `name(params) extern` without `:` or `:=`
    let (value, postfix_doc) = if cursor.eat(&Token::Extern) {
        (BindValue::Extern, None)
    } else if return_tag.is_some() && !cursor.is_at(&Token::Colon) && !cursor.is_at(&Token::ColonEq)
    {
        // `name Type` on its own line — declared, assigned later via `name: value`.
        (BindValue::Unassigned, None)
    } else {
        let is_constant = if cursor.eat(&Token::ColonEq) {
            true
        } else if cursor.eat(&Token::Colon) {
            false
        } else {
            cursor.error("expected ':', ':=' or 'extern'", cursor.current_span());
            return None;
        };
        let (value, postfix_doc) = parse_bind_value(cursor, expr_parser);
        let doc = postfix_doc.or(doc_before);
        let mut bind = Bind::new(name, name_span, value)
            .with_params(params)
            .with_return_type_name(return_type_name)
            .with_doc(doc);
        bind.is_constant = is_constant;
        if let Some(attrs) = attrs {
            bind = bind.with_attributes(attrs);
        }
        bind.param_conventions = conventions;
        bind.group_params = group_params;
        bind.param_groups = param_groups;
        bind.return_tag = return_tag;
        bind.type_annotation = type_annotation;
        bind.type_annotation_qual = type_annotation_qual;
        bind.trait_bounds = trait_bounds;
        return Some(bind);
    };

    let doc = postfix_doc.or(doc_before);

    let mut bind = Bind::new(name, name_span, value)
        .with_params(params)
        .with_return_type_name(return_type_name)
        .with_doc(doc);

    if let Some(attrs) = attrs {
        bind = bind.with_attributes(attrs);
    }
    bind.param_conventions = conventions;
    bind.group_params = group_params;
    bind.param_groups = param_groups;
    bind.return_tag = return_tag;
    bind.type_annotation = type_annotation;
    bind.type_annotation_qual = type_annotation_qual;
    bind.trait_bounds = trait_bounds;

    Some(bind)
}

/// `where T has Trait(field Pattern)` clauses after the signature, before `:`.
pub(crate) fn parse_trait_bounds(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Vec<TraitBound> {
    let mut bounds = Vec::new();
    while matches!(cursor.peek(), Some(Token::Id(n)) if *n == "where") {
        let span = cursor.peek_span().unwrap_or(cursor.current_span());
        cursor.advance();
        let type_var = match cursor.peek() {
            Some(Token::Id(n)) | Some(Token::Tag(n)) => {
                let v = cursor.intern(n);
                cursor.advance();
                v
            }
            _ => break,
        };
        if !cursor.eat(&Token::Has) {
            break;
        }
        let (trait_name, trait_name_span) = match cursor.peek() {
            Some(Token::Tag(n)) => {
                let name = cursor.intern(n);
                let s = cursor.peek_span().unwrap_or(cursor.current_span());
                cursor.advance();
                (name, s)
            }
            _ => break,
        };
        if cursor.expect(&Token::ParenOpen).is_none() {
            break;
        }
        let (field_name, pattern) = match cursor.peek() {
            Some(Token::Id(n)) => {
                let field = cursor.intern(n);
                cursor.advance();
                let Some(pat) = crate::tag::parse_pattern_type_expr(cursor, expr_parser) else {
                    break;
                };
                (field, pat)
            }
            _ => break,
        };
        cursor.expect(&Token::ParenClose);
        bounds.push(TraitBound {
            type_var,
            trait_name,
            trait_name_span,
            field_name,
            pattern: Box::new(pattern),
            span,
        });
    }
    bounds
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

    // `ref T` / `mut T` return type (e.g. `r ref Entity:` in a body bind)
    if matches!(cursor.peek(), Some(Token::Ref) | Some(Token::Mut)) {
        let mutable = cursor.eat(&Token::Mut);
        if !mutable {
            cursor.eat(&Token::Ref);
        }
        if let Some(sp) = parse_type_expr(cursor, expr_parser) {
            return (
                None,
                Some(Box::new(Spanned {
                    value: TypeExpr::Ref {
                        inner: Box::new(Spanned {
                            value: sp.value,
                            span_id: sp.span_id,
                        }),
                        mutable,
                    },
                    span_id: sp.span_id,
                })),
                None,
                None,
            );
        }
        return (None, None, None, None);
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

    // `Tag(args)` in return position — try type params first, fall through
    // to value annotation if args are value expressions.
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
                            param_spans: Vec::new(),
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
fn try_args_as_type_params(args: &[Typed<Expr>]) -> Option<Vec<(Intern<String>, ParameterKind)>> {
    let mut out = Vec::with_capacity(args.len());
    for Typed {
        value: arg,
        span_id: span,
        ..
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
            Expr::AnonymousTag(n) => {
                let sp = Spanned {
                    value: TypeExpr::Nominal(*n, *span),
                    span_id: *span,
                };
                out.push((*n, ParameterKind::Tagged(Box::new(sp))));
            }
            _ => return None,
        }
    }
    Some(out)
}

fn parse_type_annotation_args(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Vec<Typed<Expr>> {
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

fn parse_group_params(cursor: &mut TokenCursor) -> Vec<GroupParam> {
    if !cursor.is_at(&Token::BracketOpen) {
        return Vec::new();
    }
    cursor.advance();

    let mut groups = Vec::new();
    if !cursor.is_at(&Token::BracketClose) {
        loop {
            let mutable = cursor.eat(&Token::Mut);
            let Some(name) = parse_id(cursor) else {
                break;
            };
            let ty_name = match cursor.peek() {
                Some(Token::Tag(t)) => {
                    let ty = cursor.intern(t);
                    cursor.advance();
                    ty
                }
                Some(Token::Id(t)) => {
                    let ty = cursor.intern(t);
                    cursor.advance();
                    ty
                }
                _ => break,
            };
            groups.push(GroupParam {
                name,
                ty_name,
                mutable,
            });
            if !cursor.eat(&Token::Comma) {
                break;
            }
        }
    }

    cursor.expect(&Token::BracketClose);
    groups
}

fn parse_params(cursor: &mut TokenCursor, expr_parser: ExprFn) -> ParsedParams {
    if !cursor.is_at(&Token::ParenOpen) {
        return (None, IndexMap::new(), IndexMap::new());
    }

    let mut params = Parameters::new();
    let mut conventions = IndexMap::new();
    let mut param_groups = IndexMap::new();
    let mut seen_default = false;
    cursor.advance();

    if cursor.is_at(&Token::ParenClose) {
        cursor.advance();
        return (Some(params), conventions, param_groups);
    }

    let mut ingest = |key: Intern<String>,
                      kind: ParameterKind,
                      conv: ParamConvention,
                      group: Option<Intern<String>>| {
        params.insert(key, kind);
        if conv != ParamConvention::Inferred {
            conventions.insert(key, conv);
        }
        if let Some(gn) = group {
            param_groups.insert(key, gn);
        }
    };

    if let Some((key, kind, conv, group)) = parse_one_param(cursor, expr_parser) {
        seen_default = matches!(kind, ParameterKind::Default(_));
        ingest(key, kind, conv, group);
    }

    while cursor.eat(&Token::Comma) {
        if cursor.is_at(&Token::ParenClose) {
            break;
        }
        if let Some((key, kind, conv, group)) = parse_one_param(cursor, expr_parser) {
            if seen_default && !matches!(kind, ParameterKind::Default(_)) {
                cursor.error(
                    format!(
                        "positional parameter `{}` appears after a default parameter",
                        key.as_str()
                    ),
                    cursor.current_span(),
                );
            }
            if matches!(kind, ParameterKind::Default(_)) {
                seen_default = true;
            }
            ingest(key, kind, conv, group);
        }
    }

    cursor.expect(&Token::ParenClose);
    (Some(params), conventions, param_groups)
}

fn parse_param_convention_prefix(cursor: &mut TokenCursor) -> ParamConvention {
    if cursor.eat(&Token::Tilde) || cursor.eat(&Token::Eat) {
        ParamConvention::Eat
    } else if cursor.eat(&Token::Ref) {
        ParamConvention::Ref(false)
    } else if cursor.eat(&Token::Mut) {
        ParamConvention::Ref(true)
    } else {
        ParamConvention::Inferred
    }
}

fn parse_one_param(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<ParsedOneParam> {
    // Positional: bare Tag → (tag_name, ParameterKind::Tagged(tag))
    if matches!(cursor.peek(), Some(Token::Tag(_))) {
        let sp = parse_type_expr(cursor, expr_parser)?;
        let key = Intern::<String>::from_ref(type_surface_mangle_name(&sp.value));
        return Some((
            key,
            ParameterKind::Tagged(Box::new(Spanned {
                value: sp.value,
                span_id: sp.span_id,
            })),
            ParamConvention::Inferred,
            None,
        ));
    }

    let convention = parse_param_convention_prefix(cursor);

    let group_name = if cursor.is_at(&Token::BracketOpen) {
        cursor.advance();
        let gn = parse_id(cursor)?;
        cursor.expect(&Token::BracketClose)?;
        Some(gn)
    } else {
        None
    };

    // Named: id [Tag | id | : expr]
    let name = match cursor.peek() {
        Some(Token::Id(n)) => {
            let id = cursor.intern(n);
            cursor.advance();
            id
        }
        Some(Token::SelfInstance) => {
            cursor.advance();
            Intern::from_ref("self")
        }
        _ => return None,
    };

    let (name, kind) = crate::params::parse_param_after_name(cursor, expr_parser, name)?;
    Some((name, kind, convention, group_name))
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
