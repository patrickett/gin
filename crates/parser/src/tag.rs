use internment::Intern;
use lexer::Token;

use ast::span::SpanId;
use ast::{ParameterKind, Parameters, Spanned, TypeExpr};

use crate::cursor::TokenCursor;
use crate::expr::ExprFn;

impl<'src, 't> TokenCursor<'src, 't> {
    /// Type surface after `is` in `if … is …` / `when … is …` — structural [`TypeExpr`].
    #[inline]
    pub fn parse_is_pattern_tag(&mut self, expr_parser: ExprFn) -> Option<Spanned<TypeExpr>> {
        self.parse_type_expr_with(expr_parser)
    }

    /// Parse a capitalized type path (`Str`, `Maybe(T)`, `Mod.Item`) into structural type [`TypeExpr`].
    pub fn parse_type_expr(&mut self, expr_parser: ExprFn) -> Option<Spanned<TypeExpr>> {
        self.parse_type_expr_with(expr_parser)
    }

    /// Parse a type-shaped variant/pattern surface.
    ///
    /// Type arguments use parentheses (`Maybe(x)`), as do variant and `is`
    /// pattern payloads (`Some(x)`).
    pub fn parse_pattern_type_expr(&mut self, expr_parser: ExprFn) -> Option<Spanned<TypeExpr>> {
        if self.is_at(&Token::BracketOpen) {
            return self.parse_list_pattern(expr_parser);
        }
        if self.is_at(&Token::ParenOpen) {
            return self.parse_paren_tuple_pattern(expr_parser);
        }
        self.parse_type_expr_with(expr_parser)
    }

    fn parse_paren_tuple_pattern(&mut self, expr_parser: ExprFn) -> Option<Spanned<TypeExpr>> {
        let start = self.peek_span()?;
        self.expect(&Token::ParenOpen)?;
        self.skip_layout();
        if self.eat(&Token::ParenClose) {
            let end = self.last_consumed_span();
            return Some(Spanned {
                value: TypeExpr::Unit,
                span_id: self.merge_span(start, end),
            });
        }
        let first = self.parse_pattern_type_expr(expr_parser)?;
        let mut elems = vec![first];
        loop {
            if self.is_at(&Token::ParenClose) {
                break;
            }
            if self.eat_list_separator() {
                elems.push(self.parse_pattern_type_expr(expr_parser)?);
                continue;
            }
            break;
        }
        self.skip_layout();
        self.expect(&Token::ParenClose)?;
        let end = self.last_consumed_span();
        Some(Spanned {
            value: TypeExpr::Tuple(elems),
            span_id: self.merge_span(start, end),
        })
    }

    fn parse_list_pattern(&mut self, expr_parser: ExprFn) -> Option<Spanned<TypeExpr>> {
        let start = self.peek_span()?;
        self.expect(&Token::BracketOpen)?;
        self.skip_layout();
        if self.eat(&Token::BracketClose) {
            let end = self.last_consumed_span();
            return Some(Spanned {
                value: TypeExpr::ListEmpty,
                span_id: self.merge_span(start, end),
            });
        }

        let mut heads = vec![self.parse_pattern_type_expr(expr_parser)?];
        loop {
            if self.is_at(&Token::BracketClose) {
                break;
            }
            if !self.eat_list_separator() {
                break;
            }
            if self.is_at(&Token::Infer) {
                self.advance();
                let tail = self.parse_pattern_type_expr(expr_parser)?;
                self.skip_layout();
                self.expect(&Token::BracketClose)?;
                let end = self.last_consumed_span();
                let merged_span = self.merge_span(start, end);
                return Some(self.fold_list_cons_pattern(heads, tail, merged_span));
            }
            if self.is_at(&Token::BracketClose) {
                break;
            }
            heads.push(self.parse_pattern_type_expr(expr_parser)?);
        }
        self.skip_layout();
        self.expect(&Token::BracketClose)?;
        let end = self.last_consumed_span();
        let empty_tail = Spanned {
            value: TypeExpr::ListEmpty,
            span_id: end,
        };
        let merged_span = self.merge_span(start, end);
        Some(self.fold_list_cons_pattern(heads, empty_tail, merged_span))
    }

    fn fold_list_cons_pattern(
        &mut self,
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

    fn parse_type_expr_with(&mut self, expr_parser: ExprFn) -> Option<Spanned<TypeExpr>> {
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
            let inner = self.parse_type_expr_with(expr_parser)?;
            let end_span = inner.span_id;
            let span = self.merge_span(start_span, end_span);
            return Some(Spanned {
                value: TypeExpr::Pointer(Box::new(inner)),
                span_id: span,
            });
        }

        // Reference type: `ref T` or `mut T`
        if self.eat(&Token::Ref) {
            let inner = self.parse_type_expr_with(expr_parser)?;
            let end_span = inner.span_id;
            let span = self.merge_span(start_span, end_span);
            return Some(Spanned {
                value: TypeExpr::Ref {
                    inner: Box::new(inner),
                    mutable: false,
                },
                span_id: span,
            });
        }
        if self.eat(&Token::Mut) {
            let inner = self.parse_type_expr_with(expr_parser)?;
            let end_span = inner.span_id;
            let span = self.merge_span(start_span, end_span);
            return Some(Spanned {
                value: TypeExpr::Ref {
                    inner: Box::new(inner),
                    mutable: true,
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
                if let Some((name, kind, span)) = self.parse_one_tag_param(expr_parser) {
                    // NOTE: Named type arguments (e.g. `Box(a: BumpAllocator)` to skip
                    // a defaulted positional param) would allow omitting defaults without
                    // triggering this flaw. Currently only positional type args are supported.
                    if seen_default && !matches!(kind, ParameterKind::Default(_)) {
                        self.error(
                            "parse-parameter-after-default",
                            format!(
                                "positional type parameter `{}` appears after a default parameter",
                                name.as_str()
                            ),
                            self.current_span(),
                        );
                    }
                    if matches!(kind, ParameterKind::Default(_)) {
                        seen_default = true;
                    }
                    param_spans.push((name, span));
                    params.insert(name, kind);
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
    ) -> Option<(Intern<String>, ParameterKind, ast::SpanId)> {
        // Integer literal as const argument: `Vector(Int, 3)`
        if let Some(&Token::Int(n)) = self.peek() {
            let span = self.peek_span()?;
            self.advance();
            let key = Intern::<String>::from_ref(&n.to_string());
            return Some((
                key,
                ParameterKind::Tagged(Box::new(Spanned {
                    value: ast::TypeExpr::Literal(ast::expr::Literal::Int(n), span),
                    span_id: span,
                })),
                span,
            ));
        }
        if matches!(self.peek(), Some(&Token::Tag(_))) {
            let sp = self.parse_type_expr(expr_parser)?;
            let key = Intern::<String>::from_ref(sp.value.surface_mangle_name());
            return Some((
                key,
                ParameterKind::Tagged(Box::new(Spanned {
                    value: sp.value,
                    span_id: sp.span_id,
                })),
                sp.span_id,
            ));
        }

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
                ParameterKind::Tagged(Box::new(Spanned {
                    value: sp.value,
                    span_id: sp.span_id,
                })),
                name_span,
            ));
        }

        if self.eat(&Token::Colon) {
            let expr = expr_parser(self);
            return Some((name, ParameterKind::Default(Box::new(expr)), name_span));
        }

        Some((name, ParameterKind::Generic, name_span))
    }
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
