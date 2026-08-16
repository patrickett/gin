use internment::Intern;
use lexer::Token;

use ast::span::SpanId;
use ast::{
    BindValue, Expr, GroupPath, Parameter, ParameterKind, Parameters, Pattern, Spanned, TypeExpr,
    Typed,
};

use crate::cursor::TokenCursor;
use crate::expr::ExprFn;

impl<'src, 't> TokenCursor<'src, 't> {
    /// Type surface after `is` in `if … is …` / `when … is …` — structural [`TypeExpr`].
    #[inline]
    pub fn parse_is_pattern_tag(&mut self, expr_parser: ExprFn) -> Option<Spanned<Pattern>> {
        self.parse_pattern_node(expr_parser)
    }

    fn parse_pattern_node(&mut self, expr_parser: ExprFn) -> Option<Spanned<Pattern>> {
        if self.is_at(&Token::BracketOpen) {
            return self.parse_pattern_list_node(expr_parser);
        }
        if self.is_at(&Token::ParenOpen) {
            return self.parse_pattern_tuple_node(expr_parser);
        }
        self.parse_type_expr_structural(expr_parser)
            .map(|pattern| Spanned {
                value: Pattern::from(pattern.value),
                span_id: pattern.span_id,
            })
    }

    fn parse_pattern_tuple_node(&mut self, expr_parser: ExprFn) -> Option<Spanned<Pattern>> {
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
        let first = self.parse_pattern_node(expr_parser)?;
        let mut elems = vec![first];
        loop {
            if self.is_at(&Token::ParenClose) {
                break;
            }
            if self.eat_list_separator() {
                elems.push(self.parse_pattern_node(expr_parser)?);
                continue;
            }
            break;
        }
        self.skip_layout();
        self.expect(&Token::ParenClose);
        let end = self.last_consumed_span();
        Some(Spanned {
            value: Pattern::Tuple(elems),
            span_id: self.merge_span(start, end),
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

        let mut heads = vec![self.parse_pattern_node(expr_parser)?];
        loop {
            if self.is_at(&Token::BracketClose) {
                break;
            }
            if !self.eat_list_separator() {
                break;
            }
            if self.is_at(&Token::Infer) {
                self.advance();
                let tail = self.parse_pattern_node(expr_parser)?;
                self.skip_layout();
                self.expect(&Token::BracketClose);
                let end = self.last_consumed_span();
                let span = self.merge_span(start, end);
                return Some(self.fold_pattern_list_node(heads, tail, span));
            }
            if self.is_at(&Token::BracketClose) {
                break;
            }
            heads.push(self.parse_pattern_node(expr_parser)?);
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
        self.parse_type_expr_expr(expr_parser)
    }

    fn parse_type_expr_expr(&mut self, expr_parser: ExprFn) -> Option<Spanned<Expr>> {
        let start_span = self.current_span();

        if self.eat(&Token::In) {
            return self.parse_in_range_expr();
        }

        if self.eat(&Token::At) {
            let inner = self.parse_type_expr_expr(expr_parser)?;
            let span_id = self.merge_span(start_span, inner.span_id);
            return Some(Spanned {
                value: Expr::TakePtr(Box::new(Typed::infer(inner.value, inner.span_id))),
                span_id,
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
            let inner = self.parse_type_expr_expr(expr_parser)?;
            let span_id = self.merge_span(start_span, inner.span_id);
            return Some(Spanned {
                value: Expr::Ref {
                    inner: Box::new(Typed::infer(inner.value, inner.span_id)),
                    mutable,
                    group,
                },
                span_id,
            });
        }

        if self.is_at(&Token::ParenOpen) && self.peek_at(1) == Some(&Token::ParenClose) {
            let open_span = self.peek_span()?;
            self.advance();
            self.advance();
            let span_id = self.merge_span(open_span, self.last_consumed_span());
            return Some(Spanned {
                value: Expr::Lit(ast::Literal::Number(0)),
                span_id,
            });
        }

        if self.peek_at(1) == Some(&Token::Dot) {
            let checkpoint = self.checkpoint();
            if let Some(path) = self.parse_tag_variant_path() {
                if self.is_at(&Token::ParenOpen) {
                    let params = self.parse_tag_type_params_delimited(
                        expr_parser,
                        Token::ParenOpen,
                        Token::ParenClose,
                    );
                    if !params.params.is_empty() {
                        let name = *path.segments.last().unwrap_or(&path.root);
                        return Some(Spanned {
                            value: Expr::TagCall(ast::TagCall {
                                name,
                                qual_path: None,
                                args: type_params_to_expr_args(params),
                            }),
                            span_id: self.merge_span(start_span, self.last_consumed_span()),
                        });
                    }
                }
                return Some(Spanned {
                    value: Expr::FnCall(ast::FnCall { path, args: None }),
                    span_id: self.merge_span(start_span, self.last_consumed_span()),
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

        if self.is_at(&Token::ParenOpen) {
            let params = self.parse_tag_type_params_delimited(
                expr_parser,
                Token::ParenOpen,
                Token::ParenClose,
            );
            if !params.params.is_empty() {
                return Some(Spanned {
                    value: Expr::TagCall(ast::TagCall {
                        name,
                        qual_path: None,
                        args: type_params_to_expr_args(params),
                    }),
                    span_id: self.merge_span(name_span, self.last_consumed_span()),
                });
            }
        }

        let value = if is_lowercase {
            Expr::FnCall(ast::FnCall {
                path: Spanned::new(ast::ModPath::new(name, Vec::new()), name_span),
                args: None,
            })
        } else {
            Expr::AnonymousTag(name)
        };
        Some(Spanned {
            value,
            span_id: name_span,
        })
    }

    pub(crate) fn parse_type_expr_structural(
        &mut self,
        expr_parser: ExprFn,
    ) -> Option<Spanned<TypeExpr>> {
        let start_span = self.current_span();

        // Range type: `in N...M` or `in Tag`
        if self.eat(&Token::In) {
            if let Some(range) = self.parse_in_range_type() {
                let span = self.current_span();
                return Some(Spanned {
                    value: range,
                    span_id: span,
                });
            }
            return None;
        }

        // Pointer type: `@TypeExpr`
        if self.eat(&Token::At) {
            let inner = self.parse_type_expr_structural(expr_parser)?;
            let end_span = inner.span_id;
            let span = self.merge_span(start_span, end_span);

            return Some(Spanned {
                value: TypeExpr::Pointer(Box::new(inner)),
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
            let inner = self.parse_type_expr_structural(expr_parser)?;
            let end_span = inner.span_id;
            let span = self.merge_span(start_span, end_span);
            return Some(Spanned {
                value: TypeExpr::Ref {
                    inner: Box::new(inner),
                    mutable,
                    group,
                },
                span_id: span,
            });
        }

        // Unit type: `()`
        if self.is_at(&Token::ParenOpen) && self.peek_at(1) == Some(&Token::ParenClose) {
            let open_span = self.peek_span()?;
            self.advance(); // eat (
            self.advance(); // eat )
            let close_span = self.last_consumed_span();
            let span = self.merge_span(open_span, close_span);
            return Some(Spanned {
                value: TypeExpr::Unit,
                span_id: span,
            });
        }

        if self.peek_at(1) == Some(&Token::Dot) {
            let qual_checkpoint = self.checkpoint();
            if let Some(path) = self.parse_tag_variant_path() {
                if self.is_at(&Token::ParenOpen) {
                    let params = self.parse_tag_type_params_delimited(
                        expr_parser,
                        Token::ParenOpen,
                        Token::ParenClose,
                    );
                    let end_span = self.last_consumed_span();
                    let span = self.merge_span(start_span, end_span);
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
            self.rewind(qual_checkpoint);
        }

        let (name, name_span) = match *self.peek()? {
            Token::Tag(n) => {
                let name = self.intern(n);
                let span = self.peek_span()?;
                self.advance();
                (name, span)
            }
            Token::SelfTag => {
                let name = Intern::from_ref("Self");
                let span = self.peek_span()?;
                self.advance();
                (name, span)
            }
            Token::Id(n) => {
                // Lowercase identifiers in type position are type variables
                // (e.g., `x` in `@x` or `x` in `Range[x]`).
                let name = self.intern(n);
                let span = self.peek_span()?;
                self.advance();
                (name, span)
            }
            _ => return None,
        };

        if self.is_at(&Token::ParenOpen) {
            let params = self.parse_tag_type_params_delimited(
                expr_parser,
                Token::ParenOpen,
                Token::ParenClose,
            );
            let end_span = self.last_consumed_span();
            let span = self.merge_span(name_span, end_span);
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

    fn parse_tag_type_params_delimited(
        &mut self,
        expr_parser: ExprFn,
        open_token: Token<'static>,
        close_token: Token<'static>,
    ) -> ParsedTagParams {
        if self.expect(&open_token).is_none() {
            return ParsedTagParams {
                params: Parameters::new(),
                param_spans: Vec::new(),
            };
        }

        self.skip_layout();

        let mut params = Parameters::new();
        let mut param_spans = Vec::new();
        let mut seen_default = false;

        if !self.is_at(&close_token) {
            loop {
                if let Some((name, mut parameter, span)) =
                    self.parse_one_tag_param(expr_parser, params.len())
                {
                    // NOTE: Named type arguments (e.g. `Box(a: BumpAllocator)` to skip
                    // a defaulted positional param) would allow omitting defaults without
                    // triggering this flaw. Currently only positional type args are supported.
                    parameter.name_span = span;
                    if seen_default && !matches!(parameter.kind, ParameterKind::Default(_)) {
                        self.error(
                            "parse-parameter-after-default",
                            format!(
                                "positional type parameter `{}` appears after a default parameter",
                                name.as_str()
                            ),
                            self.current_span(),
                        );
                    }
                    if matches!(parameter.kind, ParameterKind::Default(_)) {
                        seen_default = true;
                    }
                    param_spans.push((name, span));
                    params.insert(name, parameter);
                }

                if self.is_at(&close_token) {
                    break;
                }

                if self.eat_list_separator() {
                    continue;
                }

                self.error(
                    "parse-expected-param-separator",
                    "expected ',' or newline between type parameters",
                    self.current_span(),
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
            let expression_ends_parameter = self.is_at(&Token::ParenClose)
                || self.is_at(&Token::Comma)
                || self.previous_token_was_newline_separator();
            if expression_ends_parameter {
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
            let expression_ends_parameter = self.is_at(&Token::ParenClose)
                || self.is_at(&Token::Comma)
                || self.previous_token_was_newline_separator();
            if expression_ends_parameter
                && !is_parse_error_expr(&expr.value)
                && !matches!(&expr.value, ast::Expr::Bind(bind) if bind.params.is_none())
                && !matches!(
                    &expr.value,
                    ast::Expr::FnCall(call) if call.args.is_none() && call.path.value.segments.is_empty()
                )
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
        let expression_ends_parameter = self.is_at(&Token::ParenClose)
            || self.is_at(&Token::Comma)
            || self.previous_token_was_newline_separator();
        if expression_ends_parameter
            && !is_parse_error_expr(&expr.value)
            && !matches!(
                &expr.value,
                ast::Expr::Bind(bind) if bind.params.is_none()
            )
            && !matches!(
                &expr.value,
                ast::Expr::FnCall(call) if call.args.is_none() && call.path.value.segments.is_empty()
            )
        {
            let span = expr.span_id;
            return Some((
                Intern::new(format!("__expr_{param_index}")),
                Parameter::new(span, ParameterKind::Default(Box::new(expr))),
                span,
            ));
        }
        self.rewind(expr_checkpoint);
        let expr = self.parse_expression();
        let expression_ends_parameter = self.is_at(&Token::ParenClose)
            || self.is_at(&Token::Comma)
            || self.previous_token_was_newline_separator();
        if expression_ends_parameter
            && !is_parse_error_expr(&expr.value)
            && !matches!(
                &expr.value,
                ast::Expr::Bind(bind) if bind.params.is_none()
            )
            && !matches!(
                &expr.value,
                ast::Expr::FnCall(call) if call.args.is_none() && call.path.value.segments.is_empty()
            )
        {
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
        ast::Expr::TupleAlloc { init, size } => {
            is_parse_error_expr(&init.value) || is_parse_error_expr(&size.value)
        }
        ast::Expr::TupleGet { base, .. } => is_parse_error_expr(&base.value),
        ast::Expr::TupleSet { base, value, .. } => {
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
        ast::Expr::TupleLit(fields) | ast::Expr::List(fields) => {
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
        ast::Condition::Is { subject, .. } => is_parse_error_expr(&subject.value),
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
            ParameterKind::Generic => Typed::infer(
                Expr::FnCall(ast::FnCall {
                    path: Spanned::new(ast::ModPath::new(name, Vec::new()), ast::SpanId::INVALID),
                    args: None,
                }),
                parameter.name_span,
            ),
            ParameterKind::Tagged(sp)
            | ParameterKind::ValueParam { ty: sp }
            | ParameterKind::Inferred { ty: sp } => Typed::infer(sp.value, sp.span_id),
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
    params: Parameters,
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
