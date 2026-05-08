use internment::Intern;
use lexer::Token;

use ast::{
    ArchTarget, Bind, BindAttributes, BindValue, Complexity, ComplexityExpr, DocComment, Expr,
    ModPath, OsTarget, ParameterKind, Parameters, Return, Spanned, type_surface_mangle_name,
};

use super::ExprFn;
use super::control::parse_return;
use crate::cursor::TokenCursor;
use crate::path::{parse_id, parse_tag_variant_path};
use crate::tag::{parse_tag_type_params, parse_type_expr};

type ReturnTypePart = (
    Option<Intern<String>>,
    Option<Box<Spanned<Expr>>>,
    Option<(Intern<String>, Vec<Spanned<Expr>>)>,
    Option<ModPath>,
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

    let params = parse_params(cursor, expr_parser);

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

        let (value, postfix_doc) = parse_bind_value(cursor, expr_parser);
        (value, postfix_doc, is_const)
    };

    let doc = postfix_doc.or(doc_before);

    let mut bind = Bind::new(name, name_span, value, is_const)
        .with_params(params)
        .with_return_type_name(return_type_name)
        .with_doc(doc);

    if let Some(attrs) = attrs {
        bind = bind.with_attributes(attrs);
    }
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

    let mut attrs = BindAttributes::default();

    if cursor.eat(&Token::BracketClose) {
        return Some(attrs);
    }

    loop {
        if let Some(attr) = parse_one_attribute(cursor) {
            if attr.os.is_some() {
                attrs.os = attr.os;
            }
            if attr.arch.is_some() {
                attrs.arch = attr.arch;
            }
            if attr.debug_only {
                attrs.debug_only = true;
            }
            if attr.test {
                attrs.test = true;
            }
            if attr.inline_always {
                attrs.inline_always = true;
            }
            if attr.complexity.is_some() {
                attrs.complexity = attr.complexity;
            }
        }

        if cursor.eat(&Token::Comma) {
            continue;
        }
        if cursor.eat(&Token::BracketClose) {
            break;
        }
        cursor.error(
            "expected ',' or ']' in attribute list",
            cursor.current_span(),
        );
        break;
    }

    Some(attrs)
}

pub(crate) fn parse_one_attribute(cursor: &mut TokenCursor) -> Option<BindAttributes> {
    match cursor.peek()? {
        Token::Id("os") => {
            cursor.advance();
            cursor.expect(&Token::ParenOpen)?;
            cursor.expect(&Token::CurlyOpen)?;

            let mut targets = Vec::new();
            loop {
                if cursor.is_at(&Token::CurlyClose) {
                    break;
                }
                if let Some(target) = parse_os_target(cursor) {
                    targets.push(target);
                }
                if !cursor.eat(&Token::Comma) {
                    break;
                }
            }

            cursor.expect(&Token::CurlyClose)?;
            cursor.expect(&Token::ParenClose)?;

            Some(BindAttributes {
                os: Some(targets),
                ..Default::default()
            })
        }
        Token::Id("arch") => {
            cursor.advance();
            cursor.expect(&Token::ParenOpen)?;
            cursor.expect(&Token::CurlyOpen)?;

            let mut targets = Vec::new();
            loop {
                if cursor.is_at(&Token::CurlyClose) {
                    break;
                }
                if let Some(target) = parse_arch_target(cursor) {
                    targets.push(target);
                }
                if !cursor.eat(&Token::Comma) {
                    break;
                }
            }

            cursor.expect(&Token::CurlyClose)?;
            cursor.expect(&Token::ParenClose)?;

            Some(BindAttributes {
                arch: Some(targets),
                ..Default::default()
            })
        }
        Token::Id("debug") => {
            cursor.advance();
            Some(BindAttributes {
                debug_only: true,
                ..Default::default()
            })
        }
        Token::Id("test") => {
            cursor.advance();
            Some(BindAttributes {
                test: true,
                ..Default::default()
            })
        }
        Token::Id("inline") => {
            cursor.advance();
            Some(BindAttributes {
                inline_always: true,
                ..Default::default()
            })
        }
        Token::Id("complexity") => {
            cursor.advance();
            cursor.expect(&Token::ParenOpen)?;

            let variant_ident = match cursor.peek() {
                Some(Token::Id(n)) => {
                    let id = cursor.intern(n);
                    cursor.advance();
                    id
                }
                Some(Token::Tag(n)) => {
                    let id = cursor.intern(n);
                    cursor.advance();
                    id
                }
                _ => {
                    cursor.error("expected complexity variant", cursor.current_span());
                    return None;
                }
            };

            let complexity = match variant_ident.as_str() {
                "Constant" => Complexity::Constant,
                "Logarithmic" | "Linear" | "LogLinear" | "Quadratic" | "Cubic" | "Exponential"
                | "Factorial" => {
                    cursor.expect(&Token::ParenOpen)?;
                    let expr = parse_complexity_expr(cursor)?;
                    cursor.expect(&Token::ParenClose)?;
                    match variant_ident.as_str() {
                        "Logarithmic" => Complexity::Logarithmic(expr),
                        "Linear" => Complexity::Linear(expr),
                        "LogLinear" => Complexity::LogLinear(expr),
                        "Quadratic" => Complexity::Quadratic(expr),
                        "Cubic" => Complexity::Cubic(expr),
                        "Exponential" => Complexity::Exponential(expr),
                        "Factorial" => Complexity::Factorial(expr),
                        _ => unreachable!(),
                    }
                }
                other => {
                    cursor.error(
                        format!("unknown complexity variant: {}", other),
                        cursor.current_span(),
                    );
                    return None;
                }
            };

            cursor.expect(&Token::ParenClose)?;

            Some(BindAttributes {
                complexity: Some(complexity),
                ..Default::default()
            })
        }
        _ => None,
    }
}

pub(crate) fn parse_os_target(cursor: &mut TokenCursor) -> Option<OsTarget> {
    match cursor.peek()? {
        Token::Id("linux") => {
            cursor.advance();
            Some(OsTarget::Linux)
        }
        Token::Id("macos") => {
            cursor.advance();
            Some(OsTarget::MacOS)
        }
        Token::Id("windows") => {
            cursor.advance();
            Some(OsTarget::Windows)
        }
        Token::Id("unknown") => {
            cursor.advance();
            Some(OsTarget::Unknown)
        }
        _ => None,
    }
}

pub(crate) fn parse_arch_target(cursor: &mut TokenCursor) -> Option<ArchTarget> {
    match cursor.peek()? {
        Token::Id("x86_64") => {
            cursor.advance();
            Some(ArchTarget::X86_64)
        }
        Token::Id("arm64") => {
            cursor.advance();
            Some(ArchTarget::Arm64)
        }
        Token::Id("wasm32") => {
            cursor.advance();
            Some(ArchTarget::Wasm32)
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
            let span = path.span;
            return (
                None,
                Some(Box::new(Spanned(Expr::TypeQualified(path), span))),
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
                Some(Box::new(Spanned(
                    Expr::TypeGeneric {
                        name,
                        params: params.into_iter().collect(),
                        span,
                    },
                    span,
                ))),
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
                    Some(Box::new(Spanned(
                        Expr::TypeGeneric {
                            name,
                            params: type_params,
                            span: name_span,
                        },
                        name_span,
                    ))),
                    None,
                    None,
                );
            }
            return (None, None, Some((name, args)), None);
        }
    }

    (
        None,
        Some(Box::new(Spanned(
            Expr::TypeNominal(name, name_span),
            name_span,
        ))),
        None,
        None,
    )
}

/// If every arg is a "type-like" expression (bare lowercase identifier, bare
/// tag, qualified type, or already a type expression), convert to declaration
/// parameters suitable for `Expr::TypeGeneric`. Otherwise return `None` so the
/// caller can fall back to the value-annotation path (e.g., `Maybe(3)`).
fn try_args_as_type_params(args: &[Spanned<Expr>]) -> Option<Vec<(Intern<String>, ParameterKind)>> {
    let mut out = Vec::with_capacity(args.len());
    for Spanned(arg, span) in args {
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
            Expr::AnonymousTag(n, s) => {
                let sp = Spanned(Expr::TypeNominal(*n, *s), *s);
                out.push((*n, ParameterKind::Tagged(Box::new(sp))));
            }
            // Already a type-surface expression.
            Expr::TypeNominal(n, _) => {
                let sp = Spanned(arg.clone(), *span);
                out.push((*n, ParameterKind::Tagged(Box::new(sp))));
            }
            Expr::TypeGeneric { name, .. }
            | Expr::TypeQualified(ast::ModPath { root: name, .. }) => {
                let sp = Spanned(arg.clone(), *span);
                out.push((*name, ParameterKind::Tagged(Box::new(sp))));
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
            Return(None)
        });

        return (BindValue::Body { exprs, ret }, None);
    }

    // Otherwise → single expression with optional postfix doc comment
    let expr = expr_parser(cursor);
    let doc = parse_doc_comment(cursor);

    (BindValue::Expr(Box::new(expr)), doc)
}

fn parse_params(cursor: &mut TokenCursor, expr_parser: ExprFn) -> Option<Parameters> {
    if !cursor.is_at(&Token::ParenOpen) {
        return None;
    }

    let mut params = Parameters::new();
    cursor.advance();

    if cursor.is_at(&Token::ParenClose) {
        cursor.advance();
        return Some(params);
    }

    if let Some((key, kind)) = parse_one_param(cursor, expr_parser) {
        params.insert(key, kind);
    }

    while cursor.eat(&Token::Comma) {
        if cursor.is_at(&Token::ParenClose) {
            break;
        }
        if let Some((key, kind)) = parse_one_param(cursor, expr_parser) {
            params.insert(key, kind);
        }
    }

    cursor.expect(&Token::ParenClose);
    Some(params)
}

fn parse_one_param(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
) -> Option<(Intern<String>, ParameterKind)> {
    // Positional: bare Tag → (tag_name, ParameterKind::Tagged(tag))
    if matches!(cursor.peek(), Some(Token::Tag(_))) {
        let sp = parse_type_expr(cursor, expr_parser)?;
        let key = Intern::<String>::from_ref(type_surface_mangle_name(&sp.0));
        return Some((key, ParameterKind::Tagged(Box::new(sp))));
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

    crate::params::parse_param_after_name(cursor, expr_parser, name)
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
        let doc = DocComment(first);
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

    let doc = DocComment(lines.join("\n"));
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
                            exprs.push(Spanned(
                                Expr::Bind(Box::new(bind)),
                                cursor.merge_span(start_span, end_span),
                            ));
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

fn parse_complexity_expr(cursor: &mut TokenCursor) -> Option<ComplexityExpr> {
    let first = parse_complexity_var(cursor)?;

    match cursor.peek() {
        Some(Token::Star) => {
            let mut vars = vec![first];
            while cursor.eat(&Token::Star) {
                vars.push(parse_complexity_var(cursor)?);
            }
            Some(ComplexityExpr::Product(vars))
        }
        Some(Token::Plus) => {
            let mut vars = vec![first];
            while cursor.eat(&Token::Plus) {
                vars.push(parse_complexity_var(cursor)?);
            }
            Some(ComplexityExpr::Sum(vars))
        }
        _ => Some(ComplexityExpr::Var(first)),
    }
}

fn parse_complexity_var(cursor: &mut TokenCursor) -> Option<Intern<String>> {
    match cursor.peek() {
        Some(Token::Id(n)) => {
            let id = cursor.intern(n);
            cursor.advance();
            Some(id)
        }
        Some(Token::Tag(n)) => {
            let id = cursor.intern(n);
            cursor.advance();
            Some(id)
        }
        _ => {
            cursor.error("expected parameter name", cursor.current_span());
            None
        }
    }
}
