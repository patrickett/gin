use diagnostic::DiagnosticCode;
use diagnostic::code::{ExpectedSyntaxCode, ParseCode, RemovedSyntaxCode, SyntaxCode};
use internment::Intern;
use lexer::Token;

use ast::span::SpanId;
use ast::{
    Bind, BindValue, Expr, GroupPath, Parameter, ParameterKind, Pattern, Spanned, TypeExpr, Typed,
};

use crate::cursor::TokenCursor;
use crate::expr::ExprFn;
use crate::expr::path_call;
use crate::int_range::in_range_expr;

/// Parsed type surface shared by expression-position and pattern-position parses.
///
/// Variants carry the spans each consumer recorded before the two parsers were
/// unified, so both conversions keep their original diagnostics geometry.
enum TypeSurface {
    Sequence(Typed<Expr>),
    InRange {
        bounds: ast::InRangeBounds,
        span: SpanId,
        outer_span: SpanId,
    },
    Pointer {
        inner: Box<Spanned<TypeSurface>>,
    },
    Ref {
        inner: Box<Spanned<TypeSurface>>,
        mutable: bool,
        group: Option<GroupPath>,
    },
    Unit,
    Qualified {
        path: Spanned<ast::ModPath>,
        params: Option<ParsedTagParams>,
        span: SpanId,
    },
    Name {
        name: Intern<String>,
        name_span: SpanId,
        is_lowercase: bool,
        params: Option<ParsedTagParams>,
        span: SpanId,
    },
}

fn surface_to_expr(surface: Spanned<TypeSurface>) -> Spanned<Expr> {
    let Spanned { value, span_id } = surface;
    let expr = match value {
        TypeSurface::Sequence(typed) => {
            return Spanned {
                value: typed.value,
                span_id: typed.span_id,
            };
        }
        TypeSurface::InRange { bounds, span, .. } => in_range_expr(bounds, span),
        TypeSurface::Pointer { inner, .. } => {
            let inner = surface_to_expr(*inner);
            Expr::TakePtr(Box::new(Typed::infer(inner.value, inner.span_id)))
        }
        TypeSurface::Ref {
            inner,
            mutable,
            group,
            ..
        } => {
            let inner = surface_to_expr(*inner);
            Expr::Ref {
                inner: Box::new(Typed::infer(inner.value, inner.span_id)),
                mutable,
                group,
            }
        }
        TypeSurface::Unit => Expr::ResultValues(Vec::new()),
        TypeSurface::Qualified { path, params, span } => {
            if let Some(params) = params
                && !params.params.is_empty()
            {
                let name = *path.value.segments.last().unwrap_or(&path.value.root);
                return Spanned {
                    value: Expr::TagCall(ast::TagCall {
                        name,
                        qual_path: None,
                        args: type_params_to_expr_args(params),
                    }),
                    span_id: span,
                };
            }
            path_call(path)
        }
        TypeSurface::Name {
            name,
            name_span,
            is_lowercase,
            params,
            span,
        } => {
            if let Some(params) = params
                && !params.params.is_empty()
            {
                return Spanned {
                    value: Expr::TagCall(ast::TagCall {
                        name,
                        qual_path: None,
                        args: type_params_to_expr_args(params),
                    }),
                    span_id: span,
                };
            }
            if is_lowercase {
                return Spanned {
                    value: path_call(Spanned::new(ast::ModPath::new(name, Vec::new()), name_span)),
                    span_id: name_span,
                };
            }
            return Spanned {
                value: Expr::AnonymousTag(name),
                span_id: name_span,
            };
        }
    };
    Spanned {
        value: expr,
        span_id,
    }
}

fn surface_to_type_expr(surface: Spanned<TypeSurface>) -> Option<Spanned<TypeExpr>> {
    let Spanned { value, span_id } = surface;
    let type_expr = match value {
        TypeSurface::Sequence(_) => return None,
        TypeSurface::InRange {
            bounds,
            span,
            outer_span,
        } => {
            return Some(Spanned {
                value: TypeExpr::InRange { bounds, span },
                span_id: outer_span,
            });
        }
        TypeSurface::Pointer { inner, .. } => {
            TypeExpr::Pointer(Box::new(surface_to_type_expr(*inner)?))
        }
        TypeSurface::Ref {
            inner,
            mutable,
            group,
            ..
        } => TypeExpr::Ref {
            inner: Box::new(surface_to_type_expr(*inner)?),
            mutable,
            group,
        },
        TypeSurface::Unit => TypeExpr::Unit,
        TypeSurface::Qualified { path, params, span } => match params {
            Some(params) if !params.params.is_empty() => {
                let name = *path.value.segments.last().unwrap_or(&path.value.root);
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
            Some(_) => {
                return Some(Spanned {
                    value: TypeExpr::Qualified(path),
                    span_id: span,
                });
            }
            None => {
                let span_id = path.span_id;
                return Some(Spanned {
                    value: TypeExpr::Qualified(path),
                    span_id,
                });
            }
        },
        TypeSurface::Name {
            name,
            name_span,
            params,
            span,
            ..
        } => match params {
            Some(params) if !params.params.is_empty() => {
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
            Some(_) => {
                return Some(Spanned {
                    value: TypeExpr::Nominal(name, span),
                    span_id: span,
                });
            }
            None => {
                return Some(Spanned {
                    value: TypeExpr::Nominal(name, name_span),
                    span_id: name_span,
                });
            }
        },
    };
    Some(Spanned {
        value: type_expr,
        span_id,
    })
}

impl<'src, 't> TokenCursor<'src, 't> {
    /// Type surface after `is` in `if … is …` / `when … is …` — structural [`TypeExpr`].
    #[inline]
    pub fn parse_is_pattern_tag(&mut self, expr_parser: ExprFn) -> Option<Spanned<Pattern>> {
        if matches!(
            self.peek(),
            Some(Token::Int(_) | Token::Float(_) | Token::String(_))
        ) {
            let literal = self.parse_literal()?;
            return Some(Spanned::new(
                Pattern::Literal(literal.value, literal.span_id),
                literal.span_id,
            ));
        }
        if self.is_at(&Token::BracketOpen) {
            return self.parse_pattern_list_node(expr_parser);
        }
        if self.is_at(&Token::ParenOpen) {
            return self.parse_pattern_value_row_node(expr_parser);
        }
        let surface = self.parse_type_surface(expr_parser, false)?;
        surface_to_type_expr(surface).map(|pattern| Spanned {
            value: Pattern::from(pattern.value),
            span_id: pattern.span_id,
        })
    }

    fn parse_pattern_value_row_node(&mut self, expr_parser: ExprFn) -> Option<Spanned<Pattern>> {
        let start = self.peek_span()?;
        self.expect(&Token::ParenOpen);
        self.skip_layout();
        if self.eat(&Token::ParenClose) {
            let end = self.last_consumed_span();
            return Some(Spanned {
                value: Pattern::Unit,
                span_id: self.merge_span(start, end),
            });
        }
        self.parse_is_pattern_tag(expr_parser)?;
        loop {
            if self.is_at(&Token::ParenClose) {
                break;
            }
            if self.eat_list_separator() {
                self.parse_is_pattern_tag(expr_parser)?;
                continue;
            }
            break;
        }
        self.skip_layout();
        self.expect(&Token::ParenClose);
        let end = self.last_consumed_span();
        let pattern_span = self.merge_span(start, end);
        // parse-positional-destructure-removed
        self.error(
            DiagnosticCode::Parse(ParseCode::Removed(RemovedSyntaxCode::PositionalDestructure)),
            "positional destructuring patterns are not supported; use named result projection",
            pattern_span,
        );
        Some(Spanned {
            value: Pattern::Invalid,
            span_id: pattern_span,
        })
    }

    fn parse_pattern_list_node(&mut self, expr_parser: ExprFn) -> Option<Spanned<Pattern>> {
        let start = self.peek_span()?;
        self.expect(&Token::BracketOpen);
        self.skip_layout();
        if self.eat(&Token::BracketClose) {
            let end = self.last_consumed_span();
            return Some(Spanned {
                value: Pattern::ListEmpty,
                span_id: self.merge_span(start, end),
            });
        }

        let mut heads = vec![self.parse_is_pattern_tag(expr_parser)?];
        loop {
            if self.is_at(&Token::BracketClose) {
                break;
            }
            if !self.eat_list_separator() {
                break;
            }
            if self.is_at(&Token::Infer) {
                self.advance();
                let tail = self.parse_is_pattern_tag(expr_parser)?;
                self.skip_layout();
                self.expect(&Token::BracketClose);
                let end = self.last_consumed_span();
                let span = self.merge_span(start, end);
                return Some(self.fold_pattern_list_node(heads, tail, span));
            }
            if self.is_at(&Token::BracketClose) {
                break;
            }
            heads.push(self.parse_is_pattern_tag(expr_parser)?);
        }
        self.skip_layout();
        self.expect(&Token::BracketClose);
        let end = self.last_consumed_span();
        let span = self.merge_span(start, end);
        Some(self.fold_pattern_list_node(
            heads,
            Spanned {
                value: Pattern::ListEmpty,
                span_id: end,
            },
            span,
        ))
    }

    fn fold_pattern_list_node(
        &mut self,
        heads: Vec<Spanned<Pattern>>,
        tail: Spanned<Pattern>,
        span: SpanId,
    ) -> Spanned<Pattern> {
        let mut result = tail;
        for head in heads.into_iter().rev() {
            result = Spanned {
                value: Pattern::ListCons {
                    head: Box::new(head),
                    tail: Box::new(result),
                },
                span_id: span,
            };
        }
        result
    }

    pub(crate) fn parse_group_path(&mut self) -> Option<GroupPath> {
        let root = self.parse_id()?;
        let mut segments = Vec::new();
        while self.eat(&Token::Dot) {
            let segment = match self.peek() {
                Some(Token::Id(name) | Token::Tag(name)) => {
                    let segment = self.intern(name);
                    self.advance();
                    segment
                }
                _ => return None,
            };
            segments.push(segment);
        }
        Some(GroupPath::new(root, segments))
    }

    pub fn parse_type_expr(&mut self, expr_parser: ExprFn) -> Option<Spanned<Expr>> {
        let surface = self.parse_type_surface(expr_parser, true)?;
        Some(surface_to_expr(surface))
    }

    pub(crate) fn parse_type_argument(&mut self, expr_parser: ExprFn) -> Typed<Expr> {
        if matches!(self.peek(), Some(Token::Id(_))) && self.peek_at(1) == Some(&Token::Colon) {
            let name_span = self.current_span();
            let name = self.parse_name().expect("named type argument");
            self.advance();
            let value = self.parse_type_argument(expr_parser);
            let span = self.merge_span(name_span, value.span_id);
            return Typed::infer(
                Expr::Bind(Box::new(Bind::new(
                    name,
                    name_span,
                    BindValue::Expr(Box::new(value)),
                ))),
                span,
            );
        }
        let unit = self.is_at(&Token::ParenOpen) && self.peek_at(1) == Some(&Token::ParenClose);
        if unit || self.is_at(&Token::BracketOpen) {
            let checkpoint = self.checkpoint();
            let errors = self.errors.len();
            if let Some(expr) = self.parse_type_expr(expr_parser)
                && self.errors.len() == errors
                && matches!(
                    self.peek(),
                    Some(Token::Comma | Token::ParenClose | Token::Newline)
                )
            {
                return Typed::infer(expr.value, expr.span_id);
            }
            self.rewind(checkpoint);
            self.errors.truncate(errors);
        }
        expr_parser(self)
    }

    fn parse_sequence_type(&mut self, expr_parser: ExprFn) -> Typed<Expr> {
        let start = self.current_span();
        self.advance();
        self.skip_layout();
        let mut elems = Vec::new();
        while !self.is_at(&Token::BracketClose) && self.peek().is_some() {
            let name = if matches!(self.peek(), Some(Token::Id(_)))
                && matches!(
                    self.peek_at(1),
                    Some(
                        Token::Tag(_) | Token::Ref | Token::Mut | Token::BracketOpen | Token::Id(_)
                    )
                ) {
                self.parse_name()
            } else {
                None
            };
            let Some(ty) = self.parse_type_expr(expr_parser) else {
                break;
            };
            let expr = Typed::infer(ty.value, ty.span_id);
            if self.eat(&Token::ColonSemi) {
                if name.is_some() || !elems.is_empty() {
                    // parse-sequence-repeat-elements
                    self.error(
                        DiagnosticCode::Parse(ParseCode::Syntax(
                            SyntaxCode::SequenceRepeatElements,
                        )),
                        "a repeated type requires one unnamed element type",
                        start,
                    );
                }
                let count = self.parse_expression();
                self.skip_layout();
                self.expect(&Token::BracketClose);
                return Typed::infer(
                    Expr::SequenceLiteral(ast::SequenceLiteral::Repeat {
                        seed: Box::new(expr),
                        count: Box::new(count),
                    }),
                    self.merge_span(start, self.last_consumed_span()),
                );
            }
            elems.push(ast::SequenceElement { name, expr });
            if self.is_at(&Token::BracketClose) {
                break;
            }
            if !self.eat_list_separator() {
                break;
            }
            self.skip_layout();
        }
        self.skip_layout();
        self.expect(&Token::BracketClose);
        self.sequence_value(elems, start)
    }

    fn parse_type_surface(
        &mut self,
        expr_parser: ExprFn,
        allow_sequence: bool,
    ) -> Option<Spanned<TypeSurface>> {
        let start_span = self.current_span();

        // Fixed-array type syntax `[Element; length]` is expression-position only.
        if allow_sequence && self.is_at(&Token::BracketOpen) {
            let array = self.parse_sequence_type(expr_parser);
            let span_id = array.span_id;
            return Some(Spanned {
                value: TypeSurface::Sequence(array),
                span_id,
            });
        }

        if self.eat(&Token::In) {
            let (bounds, span) = self.parse_in_range_bounds()?;
            let outer_span = self.current_span();
            return Some(Spanned {
                value: TypeSurface::InRange {
                    bounds,
                    span,
                    outer_span,
                },
                span_id: span,
            });
        }

        if self.eat(&Token::At) {
            let inner = self.parse_type_surface(expr_parser, allow_sequence)?;
            let span = self.merge_span(start_span, inner.span_id);
            return Some(Spanned {
                value: TypeSurface::Pointer {
                    inner: Box::new(inner),
                },
                span_id: span,
            });
        }

        if matches!(self.peek(), Some(Token::Ref) | Some(Token::Mut)) {
            let mutable = self.eat(&Token::Mut);
            if !mutable {
                self.advance();
            }
            let group = if self.eat(&Token::CurlyOpen) {
                let group = self.parse_group_path()?;
                self.expect(&Token::CurlyClose)?;
                Some(group)
            } else {
                None
            };
            let inner = self.parse_type_surface(expr_parser, allow_sequence)?;
            let span = self.merge_span(start_span, inner.span_id);
            return Some(Spanned {
                value: TypeSurface::Ref {
                    inner: Box::new(inner),
                    mutable,
                    group,
                },
                span_id: span,
            });
        }

        if self.is_at(&Token::ParenOpen) && self.peek_at(1) == Some(&Token::ParenClose) {
            let open_span = self.peek_span()?;
            self.advance();
            self.advance();
            let span = self.merge_span(open_span, self.last_consumed_span());
            return Some(Spanned {
                value: TypeSurface::Unit,
                span_id: span,
            });
        }

        if self.peek_at(1) == Some(&Token::Dot) {
            let checkpoint = self.checkpoint();
            if let Some(path) = self.parse_tag_rooted_path(true) {
                let params = if self.is_at(&Token::ParenOpen) {
                    Some(self.parse_tag_type_params_delimited(
                        expr_parser,
                        Token::ParenOpen,
                        Token::ParenClose,
                    ))
                } else {
                    None
                };
                let span = self.merge_span(start_span, self.last_consumed_span());
                return Some(Spanned {
                    value: TypeSurface::Qualified { path, params, span },
                    span_id: span,
                });
            }
            self.rewind(checkpoint);
        }

        let (name, name_span, is_lowercase) = match *self.peek()? {
            Token::Tag(n) => {
                let name = self.intern(n);
                let span = self.peek_span()?;
                self.advance();
                (name, span, false)
            }
            Token::SelfTag => {
                let span = self.peek_span()?;
                self.advance();
                (Intern::from_ref("Self"), span, false)
            }
            Token::Id(n) => {
                let name = self.intern(n);
                let span = self.peek_span()?;
                self.advance();
                (name, span, true)
            }
            _ => return None,
        };

        let params = if self.is_at(&Token::ParenOpen) {
            Some(self.parse_tag_type_params_delimited(
                expr_parser,
                Token::ParenOpen,
                Token::ParenClose,
            ))
        } else {
            None
        };
        let span = if params.is_some() {
            self.merge_span(name_span, self.last_consumed_span())
        } else {
            name_span
        };
        Some(Spanned {
            value: TypeSurface::Name {
                name,
                name_span,
                is_lowercase,
                params,
                span,
            },
            span_id: span,
        })
    }

    fn parse_tag_type_params_delimited(
        &mut self,
        expr_parser: ExprFn,
        open_token: Token<'static>,
        close_token: Token<'static>,
    ) -> ParsedTagParams {
        if self.expect(&open_token).is_none() {
            return ParsedTagParams {
                params: Vec::new(),
                param_spans: Vec::new(),
            };
        }

        self.skip_layout();

        let mut params = Vec::new();
        let mut param_spans = Vec::new();

        if !self.is_at(&close_token) {
            loop {
                if let Some((name, mut parameter, span)) =
                    self.parse_one_tag_param(expr_parser, params.len())
                {
                    parameter.name_span = span;
                    param_spans.push((name, span));
                    params.push((name, parameter));
                }

                if self.is_at(&close_token) {
                    break;
                }

                if self.eat_list_separator() {
                    continue;
                }

                // parse-expected-param-separator
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(
                        ExpectedSyntaxCode::ParameterSeparator,
                    )),
                    "expected ',' or newline between type parameters",
                );
                break;
            }
        }

        self.skip_layout();
        self.expect(&close_token);
        ParsedTagParams {
            params,
            param_spans,
        }
    }

    fn ends_param_list(&mut self) -> bool {
        self.is_at(&Token::ParenClose)
            || self.is_at(&Token::Comma)
            || self.previous_token_was_newline_separator()
    }

    fn parse_one_tag_param(
        &mut self,
        expr_parser: ExprFn,
        param_index: usize,
    ) -> Option<(Intern<String>, Parameter, ast::SpanId)> {
        // Integer literal as const argument: `Vector(Int, 3)`
        if let Some(&Token::Int(n)) = self.peek() {
            let span = self.peek_span()?;
            let expr_checkpoint = self.checkpoint();
            self.advance();
            if self.ends_param_list() {
                let key = Intern::<String>::from_ref(&n.to_string());
                return Some((
                    key,
                    Parameter::new(
                        span,
                        ParameterKind::Tagged(Box::new(Spanned {
                            value: ast::Expr::Lit(ast::expr::Literal::Int(n)),
                            span_id: span,
                        })),
                    ),
                    span,
                ));
            }
            self.rewind(expr_checkpoint);
        }
        if matches!(self.peek(), Some(&Token::Tag(_))) {
            let expr_checkpoint = self.checkpoint();
            let expr = expr_parser(self);
            if self.ends_param_list()
                && usable_param_expr(&expr)
                && matches!(
                    &expr.value,
                    ast::Expr::AnonymousTag(_)
                        | ast::Expr::Ref { .. }
                        | ast::Expr::TakePtr(_)
                        | ast::Expr::FnCall(_)
                )
            {
                let key = Pattern::from_expr(expr.value.clone())
                    .surface_mangle_name()
                    .to_string();
                let span = expr.span_id;
                let key = Intern::new(key);
                return Some((
                    key,
                    Parameter::new(span, ParameterKind::Tagged(Box::new(expr.into()))),
                    span,
                ));
            }
            self.rewind(expr_checkpoint);
        }

        let expr_checkpoint = self.checkpoint();
        let expr = expr_parser(self);
        if self.ends_param_list() && usable_param_expr(&expr) {
            let span = expr.span_id;
            return Some((
                Intern::new(format!("__expr_{param_index}")),
                Parameter::new(span, ParameterKind::Default(Box::new(expr))),
                span,
            ));
        }
        self.rewind(expr_checkpoint);
        let expr = self.parse_expression();
        if self.ends_param_list() && usable_param_expr(&expr) {
            let span = expr.span_id;
            return Some((
                Intern::new(format!("__expr_{param_index}")),
                Parameter::new(span, ParameterKind::Default(Box::new(expr))),
                span,
            ));
        }
        self.rewind(expr_checkpoint);

        let (name, name_span) = match self.peek()? {
            &Token::Id(n) => {
                let span = self.peek_span()?;
                let id = self.intern(n);
                self.advance();
                (id, span)
            }
            _ => return None,
        };

        if matches!(self.peek(), Some(&Token::Tag(_))) {
            let sp = self.parse_type_expr(expr_parser)?;
            return Some((
                name,
                Parameter::new(
                    name_span,
                    ParameterKind::Tagged(Box::new(Spanned {
                        value: sp.value,
                        span_id: sp.span_id,
                    })),
                ),
                name_span,
            ));
        }

        if self.eat(&Token::Colon) {
            let expr = expr_parser(self);
            return Some((
                name,
                Parameter::new(name_span, ParameterKind::Default(Box::new(expr))),
                name_span,
            ));
        }

        Some((
            name,
            Parameter::new(name_span, ParameterKind::Generic),
            name_span,
        ))
    }
}

fn usable_param_expr(expr: &Typed<Expr>) -> bool {
    !is_parse_error_expr(&expr.value)
        && !matches!(&expr.value, ast::Expr::Bind(bind) if bind.params.is_none())
        && !matches!(
            &expr.value,
            ast::Expr::FnCall(call) if call.args.is_none() && call.path.value.segments.is_empty()
        )
}

fn is_parse_error_expr(expr: &ast::Expr) -> bool {
    match expr {
        ast::Expr::AnonymousTag(name) if name.as_str() == "__parse_error" => true,
        ast::Expr::Binary(binary) => {
            is_parse_error_expr(&binary.lhs.value) || is_parse_error_expr(&binary.rhs.value)
        }
        ast::Expr::FnCall(call) => call.path.value.root.as_str() == "__parse_error",
        ast::Expr::Bind(bind) => match &bind.value {
            BindValue::Expr(expr) => is_parse_error_expr(&expr.value),
            _ => false,
        },
        ast::Expr::TakePtr(inner) => is_parse_error_expr(&inner.value),
        ast::Expr::Range(range) => {
            is_parse_error_expr(&range.start.value) || is_parse_error_expr(&range.end.value)
        }
        ast::Expr::Ref { inner, .. } => is_parse_error_expr(&inner.value),
        ast::Expr::Negate(inner) => is_parse_error_expr(&inner.value),
        ast::Expr::FixedArrayAlloc { init, size } => {
            is_parse_error_expr(&init.value) || is_parse_error_expr(&size.value)
        }
        ast::Expr::AggregateGet { base, .. } => is_parse_error_expr(&base.value),
        ast::Expr::AggregateSet { base, value, .. } => {
            is_parse_error_expr(&base.value) || is_parse_error_expr(&value.value)
        }
        ast::Expr::Cast { expr, .. } => is_parse_error_expr(&expr.value),
        ast::Expr::BufGet { buf, index } => {
            is_parse_error_expr(&buf.value) || is_parse_error_expr(&index.value)
        }
        ast::Expr::BufSet {
            buf, index, value, ..
        } => {
            is_parse_error_expr(&buf.value)
                || is_parse_error_expr(&index.value)
                || is_parse_error_expr(&value.value)
        }
        ast::Expr::RecordGet { base, .. } => is_parse_error_expr(&base.value),
        ast::Expr::RecordSet { base, value, .. } => {
            is_parse_error_expr(&base.value) || is_parse_error_expr(&value.value)
        }
        ast::Expr::RecordLit(fields) => fields
            .iter()
            .any(|(_, field)| is_parse_error_expr(&field.value)),
        ast::Expr::ArrayLit(fields) | ast::Expr::List(fields) => {
            fields.iter().any(|field| is_parse_error_expr(&field.value))
        }
        ast::Expr::Destructure { value, .. } => is_parse_error_expr(&value.value),
        ast::Expr::When(when_expr) => when_expr.arms.iter().any(|arm| match arm {
            ast::WhenArm::Cond {
                condition, body, ..
            } => is_parse_error_condition(condition) || is_parse_error_expr(&body.value),
            ast::WhenArm::Is { body, .. } => is_parse_error_expr(&body.value),
            ast::WhenArm::Else(body, _) => is_parse_error_expr(&body.value),
        }),
        ast::Expr::If(if_expr) => {
            is_parse_error_condition(&if_expr.condition)
                || if_expr
                    .body
                    .iter()
                    .any(|expr| is_parse_error_expr(&expr.value))
        }
        _ => false,
    }
}

fn is_parse_error_condition(condition: &ast::Condition) -> bool {
    match condition {
        ast::Condition::Compare { left, right, .. } => {
            is_parse_error_expr(&left.value) || is_parse_error_expr(&right.value)
        }
        ast::Condition::Proposition { subject } | ast::Condition::Is { subject, .. } => {
            is_parse_error_expr(&subject.value)
        }
        ast::Condition::Not(inner) => is_parse_error_condition(inner),
        ast::Condition::And(left, right) | ast::Condition::Or(left, right) => {
            is_parse_error_condition(left) || is_parse_error_condition(right)
        }
    }
}

fn type_params_to_expr_args(params: ParsedTagParams) -> Vec<Typed<Expr>> {
    params
        .params
        .into_iter()
        .map(|(name, parameter)| match parameter.kind {
            ParameterKind::Proposition(_) => {
                unreachable!("type applications do not parse proposition parameters")
            }
            ParameterKind::Generic => Typed::infer(
                path_call(Spanned::new(
                    ast::ModPath::new(name, Vec::new()),
                    ast::SpanId::INVALID,
                )),
                parameter.name_span,
            ),
            ParameterKind::Tagged(sp)
            | ParameterKind::ValueParam { ty: sp }
            | ParameterKind::Inferred { ty: sp } => Typed::infer(sp.value, sp.span_id),
            ParameterKind::Default(expr) if !name.as_str().starts_with("__expr_") => {
                let span = parameter.name_span;
                Typed::infer(
                    Expr::Bind(Box::new(Bind::new(name, span, BindValue::Expr(expr)))),
                    span,
                )
            }
            ParameterKind::Default(expr) => *expr,
        })
        .collect()
}

type OrderedTagParams = (
    Vec<(Intern<String>, ParameterKind)>,
    Vec<(Intern<String>, ast::SpanId)>,
);

/// Ordered type parameters plus per-name spans for IDE hover.
struct ParsedTagParams {
    params: Vec<(Intern<String>, Parameter)>,
    param_spans: Vec<(Intern<String>, ast::SpanId)>,
}

impl ParsedTagParams {
    fn into_ordered_param_spans(self) -> OrderedTagParams {
        let mut params = Vec::with_capacity(self.params.len());
        let param_spans = self.param_spans;
        for (name, param) in self.params {
            params.push((name, param.kind));
        }
        (params, param_spans)
    }
}
