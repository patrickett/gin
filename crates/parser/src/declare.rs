use indexmap::IndexMap;

use crate::cursor::TokenCursor;
use crate::expr::ExprFn;
use ast::declare::HasMemberQualifier;
use ast::span::SpanId;
use ast::{
    Declare, DeclareValue, DocComment, Expr, HasFunction, HasFunctionKind, HasMember,
    HasMemberBody, HasProperty, NormalExpr, Parameter, ParameterKind, Parameters, Pattern,
    PredicateExpr, ProvidedTrait, Spanned, Typed, Variant,
};
use i256::I256;
use internment::Intern;
use lexer::Token;

type ParseResultType = (Option<Box<Spanned<Expr>>>, Option<Box<Spanned<Expr>>>);
type ParsedVariantParts = (Spanned<Pattern>, Option<Box<Spanned<Expr>>>);

impl<'src, 't> TokenCursor<'src, 't> {
    pub fn parse_declare(&mut self, expr_parser: ExprFn) -> Option<Declare> {
        let checkpoint = self.checkpoint();
        let doc_before = self.parse_doc_comment();
        if doc_before.is_some() {
            self.skip_newlines();
        }
        let attrs = self.parse_declare_attributes();
        self.eat(&Token::Indent);

        let (name, name_span) = match self.peek()? {
            &Token::Tag(n) => {
                let name = self.intern(n);
                let span = self.peek_span()?;
                self.advance();
                (name, span)
            }
            _ => {
                self.rewind(checkpoint);
                return None;
            }
        };

        let params = self.parse_params_for_declare(expr_parser);

        if self.eat(&Token::Is) {
            let doc_after_is = self.parse_doc_comment();
            self.eat(&Token::Indent);

            let (value, doc_after_value, mut provided_traits) = self.parse_is_rhs(expr_parser);
            provided_traits.extend(self.parse_provided_trait_clauses(expr_parser));
            self.eat(&Token::Dedent);

            let doc = DocComment::combine(
                DocComment::combine(doc_before, doc_after_is),
                doc_after_value,
            );
            let mut decl = Declare::new(name, name_span, value)
                .with_params(params)
                .with_doc(doc)
                .with_provided_traits(provided_traits);
            decl.span = self.merge_span(name_span, self.last_consumed_span());
            if let Some(a) = attrs {
                decl = decl.with_attributes(a);
            }
            Some(decl)
        } else if self.eat(&Token::Has) {
            self.parse_doc_comment();
            let has_block_body = self.eat(&Token::Indent);

            let (value, provided_traits) = if self.is_at(&Token::ParenOpen) {
                self.error(
                    "parse-removed-has-parens",
                    "parenthesized `has` members are no longer valid; remove the parentheses",
                    self.current_span(),
                );
                self.recover_removed_parenthesized_has_rhs();
                (DeclareValue::Has(Vec::new()), Vec::new())
            } else if matches!(self.peek(), Some(Token::Tag(_))) {
                let provided_traits = self.parse_trait_chain_after_has(expr_parser);
                let checkpoint = self.checkpoint();
                self.skip_newlines();
                let value = if self.eat(&Token::Indent) {
                    self.parse_has_rhs(expr_parser, true)
                } else {
                    self.rewind(checkpoint);
                    DeclareValue::Has(Vec::new())
                };
                (value, provided_traits)
            } else {
                (self.parse_has_rhs(expr_parser, has_block_body), Vec::new())
            };
            self.eat(&Token::Dedent);

            let mut provided_traits = provided_traits;
            Self::collect_qualified_property_provisions(name, &value, &mut provided_traits);
            let mut decl = Declare::new(name, name_span, value)
                .with_params(params)
                .with_doc(doc_before)
                .with_provided_traits(provided_traits);
            decl.span = self.merge_span(name_span, self.last_consumed_span());
            if let Some(a) = attrs {
                decl = decl.with_attributes(a);
            }
            Some(decl)
        } else {
            self.error(
                "parse-expected-declare-keyword",
                "expected 'is' or 'has'",
                self.current_span(),
            );
            None
        }
    }

    fn parse_declare_attributes(&mut self) -> Option<ast::DeclareAttributes> {
        let items = self.parse_attribute_items()?;
        let mut attrs = ast::DeclareAttributes {
            raw_attributes: Some(items),
            ..Default::default()
        };
        attrs.extract_intrinsic_attributes();
        Some(attrs)
    }

    fn recover_removed_parenthesized_has_rhs(&mut self) {
        self.advance();
        let mut depth = 1;
        while depth > 0 {
            match self.advance_raw() {
                Some((Token::ParenOpen, _)) => depth += 1,
                Some((Token::ParenClose, _)) => depth -= 1,
                Some(_) => {}
                None => break,
            }
        }
    }

    fn parse_has_rhs(&mut self, expr_parser: ExprFn, has_block_body: bool) -> DeclareValue {
        let mut members = Vec::new();

        loop {
            if self.is_at(&Token::ParenClose) {
                if !has_block_body {
                    self.advance();
                }
                break;
            }
            if matches!(self.raw_peek(), Some(Token::Dedent)) {
                break;
            }

            // Doc comment attached to this member (skip newlines first)
            self.skip_newlines();
            let member_doc = self.parse_doc_comment();

            let (qualifier, member_name) = match self.peek() {
                Some(Token::Id(n)) => {
                    let name = self.intern(n);
                    let name_span = self
                        .peek_span()
                        .expect("peek confirmed Id token, peek_span should succeed");
                    self.advance();
                    (None, (name, name_span))
                }
                Some(Token::Tag(n)) if self.peek_at(1) == Some(&Token::Dot) => {
                    let qualifier = HasMemberQualifier {
                        name: self.intern(n),
                        span: self.peek_span().unwrap_or(self.current_span()),
                    };
                    self.advance();
                    self.advance();
                    let Some(Token::Id(n)) = self.peek() else {
                        self.error(
                            "parse-expected-method-name",
                            "expected method name after trait qualifier",
                            self.current_span(),
                        );
                        break;
                    };
                    let name = self.intern(n);
                    let name_span = self.peek_span().unwrap_or(self.current_span());
                    self.advance();
                    (Some(qualifier), (name, name_span))
                }
                Some(_) => {
                    self.error(
                        "parse-expected-method-name",
                        "expected method name in interface body",
                        self.current_span(),
                    );
                    break;
                }
                None => break,
            };

            let is_function = self.is_at(&Token::ParenOpen);
            let (params, conventions, param_groups, param_refinements) = if is_function {
                self.parse_params(expr_parser)
            } else {
                (
                    Some(Parameters::new()),
                    IndexMap::new(),
                    IndexMap::new(),
                    IndexMap::new(),
                )
            };

            // Parse optional return type and error type
            let (return_ty, error_ty) = self.parse_method_return_type(expr_parser);

            // Parse optional refinement predicate: `and < n`, `and > 0 and < n`, etc.
            let refinement = if self.eat(&Token::And) {
                let mut preds = vec![self.parse_one_predicate()];
                while self.eat(&Token::And) {
                    preds.push(self.parse_one_predicate());
                }
                Some(if preds.len() == 1 {
                    preds.into_iter().next().unwrap()
                } else {
                    PredicateExpr::And(preds)
                })
            } else {
                None
            };

            let body = if self.eat(&Token::ColonEq) {
                let (value, _) = self.parse_bind_value(expr_parser);
                Some(HasMemberBody::Final(value))
            } else if self.eat(&Token::Colon) {
                let (value, _) = self.parse_bind_value(expr_parser);
                Some(HasMemberBody::Overrideable(value))
            } else {
                None
            };

            let params = params.unwrap_or_else(Parameters::new);
            let member = if is_function {
                let kind = if params.contains_key(&Intern::from_ref("self")) {
                    HasFunctionKind::Instance
                } else {
                    HasFunctionKind::Associated
                };
                HasMember::Function(Box::new(HasFunction {
                    qualifier,
                    name: member_name.0,
                    name_span: member_name.1,
                    params: Box::new(params),
                    conventions: Box::new(conventions),
                    param_groups: Box::new(param_groups),
                    param_refinements: Box::new(param_refinements),
                    return_ty,
                    error_ty,
                    body,
                    doc_comment: member_doc,
                    refinement,
                    kind,
                }))
            } else {
                HasMember::Property(HasProperty {
                    qualifier,
                    name: member_name.0,
                    name_span: member_name.1,
                    ty: return_ty,
                    body,
                    doc_comment: member_doc,
                    refinement,
                })
            };
            members.push(member);

            if self.eat(&Token::Comma) {
                if has_block_body {
                    // A trailing comma after the last member is allowed.
                    self.skip_newlines();
                    self.skip_indents();
                    if matches!(self.raw_peek(), Some(Token::Dedent | Token::ParenClose)) {
                        if self.eat(&Token::ParenClose) {
                            let _ = self.eat(&Token::Dedent);
                        }
                        break;
                    }
                } else {
                    self.skip_layout();
                    if matches!(self.raw_peek(), Some(Token::ParenClose | Token::Dedent)) {
                        if self.eat(&Token::ParenClose) {
                            let _ = self.eat(&Token::Dedent);
                        }
                        break;
                    }
                }
                continue;
            }

            if has_block_body {
                let checkpoint = self.checkpoint();
                self.skip_newlines();
                if matches!(self.raw_peek(), Some(Token::Dedent | Token::Comma)) {
                    self.rewind(checkpoint);
                    break;
                }
                if self.checkpoint() != checkpoint {
                    continue;
                }
            }

            break;
        }

        if members.is_empty() {
            // Empty interface: `Allocator has `
        }

        DeclareValue::Has(members)
    }

    /// Parse the return type (and optional error type) after a method's parameter list.
    ///
    /// Handles:
    /// - `) Slice(Byte)` → return_ty = Some(Slice(Byte)), error_ty = None
    /// - `) Slice(Byte) or AllocError` → return_ty = Some(Slice(Byte)), error_ty = Some(AllocError)
    /// - `)` → return_ty = None, error_ty = None (no return type)
    /// - `) ref T` / `) mut T` → return_ty = Some(ref/mut T)
    /// - `) ()` → return_ty = Some(unit)
    ///
    /// IMPORTANT: if the next token is a raw newline (return type on a different
    /// line from params), we skip return type parsing entirely. This prevents
    /// consuming the next member's name as a bare-Id return type when there's
    /// no explicit separator, which enables newline-separated interface members.
    fn parse_method_return_type(&mut self, expr_parser: ExprFn) -> ParseResultType {
        // If the return type would be on a different line from the params,
        // there's no return type — the real return type must be on the same
        // line. This prevents consuming the next member name as a bare-Id
        // return type when using newline-separated members.
        if matches!(self.raw_peek(), Some(Token::Newline)) {
            return (None, None);
        }

        // Check if the next token could start a return type.
        // `parse_type_expr` handles: `@T`, `ref T`, `mut T`, `()`, `Tag`, `Tag(...)`, `Qual.Path`.
        // But in interface method position, `(` followed by something other than `)` could be
        // a tuple type — skip unit check since parse_type_expr handles it.
        let can_start = matches!(
            self.peek(),
            Some(Token::Tag(_) | Token::SelfTag)
                | Some(Token::Ref)
                | Some(Token::Mut)
                | Some(Token::At)
        ) || matches!(self.peek(), Some(Token::ParenOpen) if self.peek_at(1) == Some(&Token::ParenClose));

        if !can_start {
            // Bare lowercase id as return type (e.g. `start x` where x is a type variable)
            if let Some(Token::Id(t)) = self.peek() {
                let name = self.intern(t);
                let span = self.peek_span().unwrap_or(SpanId::INVALID);
                self.advance();
                let sp = Spanned {
                    value: Expr::AnonymousTag(name),
                    span_id: span,
                };
                return self.check_or_error(expr_parser, Some(Box::new(sp)));
            }
            return (None, None);
        }

        if let Some(sp) = self.parse_type_expr(expr_parser) {
            return self.check_or_error(expr_parser, Some(Box::new(sp)));
        }

        (None, None)
    }

    /// After parsing a return type, check for `or ErrorType`.
    ///
    /// Uses `peek_past_indent` (non-consuming) to determine if `or` follows
    /// before consuming layout, so it won't steal a newline that should be
    /// consumed as a list-item separator by a caller
    /// (e.g. `parse_has_rhs` or a top-level sequence parser).
    fn check_or_error(
        &mut self,
        expr_parser: ExprFn,
        return_ty: Option<Box<Spanned<Expr>>>,
    ) -> ParseResultType {
        // Non-consuming peek: is `or` the next significant token?
        if self.peek_past_indent() == Some(&Token::Or) {
            // `or` is present — consume layout then `or`.
            self.skip_indents();
            self.advance(); // consume `or`
            self.skip_indents();
            if let Some(sp) = self.parse_type_expr(expr_parser) {
                return (return_ty, Some(Box::new(sp)));
            }
            self.error(
                "parse-expected-error-type",
                "expected error type after 'or' in interface method return type",
                self.current_span(),
            );
        }
        // No `or` — leave layout unconsumed so it can serve as a list separator.
        (return_ty, None)
    }

    fn parse_is_rhs(
        &mut self,
        expr_parser: ExprFn,
    ) -> (DeclareValue, Option<DocComment>, Vec<ProvidedTrait>) {
        if matches!(self.peek(), Some(Token::When))
            && let Some(when) = self.parse_when_expr(expr_parser)
        {
            return (DeclareValue::When(Box::new(when)), None, Vec::new());
        }

        // InRange: `in` N...M
        if self.eat(&Token::In) {
            let checkpoint = self.checkpoint();
            if let Some((a, b)) = self.parse_int_range() {
                self.reject_removed_marker_clause();
                return (DeclareValue::InRange(a, b), None, Vec::new());
            }
            self.rewind(checkpoint);
            if let Some((min, max)) = self.parse_symbolic_range() {
                let mut predicates = vec![PredicateExpr::Ge(min), PredicateExpr::Le(max)];
                self.reject_removed_marker_clause();
                while self.eat(&Token::And) {
                    predicates.push(self.parse_one_predicate());
                }
                return (
                    DeclareValue::Refinement(Box::new(PredicateExpr::And(predicates))),
                    None,
                    Vec::new(),
                );
            }
            self.error(
                "parse-expected-integer-range",
                "expected integer range after 'in'",
                self.current_span(),
            );
            return (
                DeclareValue::InRange(I256::from_u128(0), I256::from_u128(0)),
                None,
                Vec::new(),
            );
        }

        // Range: [-]Int...[-]Int
        if matches!(self.peek(), Some(Token::Int(_)) | Some(Token::Minus)) {
            let checkpoint = self.checkpoint();
            if let Some((a, b)) = self.parse_int_range() {
                self.reject_removed_marker_clause();
                return (DeclareValue::Range(a, b), None, Vec::new());
            }
            self.rewind(checkpoint);
        }

        // Unit type: `()`
        if self.is_at(&Token::ParenOpen) && self.peek_at(1) == Some(&Token::ParenClose) {
            self.advance(); // eat (
            self.advance(); // eat )
            let span = self.last_consumed_span();
            return (
                DeclareValue::Alias(Box::new(Spanned {
                    value: Expr::Lit(ast::Literal::Number(0)),
                    span_id: span,
                })),
                None,
                Vec::new(),
            );
        }

        // Union or Alias: starts with optional doc then a type pattern or literal
        let checkpoint = self.checkpoint();
        let first_doc = self.parse_doc_comment();

        // Try literal variant first (e.g. 'debug'), then tag pattern (e.g. Some(x))
        let first_variant_parts: Option<ParsedVariantParts> = 'union_check: {
            // Check if we're at a literal that could be a union variant
            if matches!(
                self.peek(),
                Some(Token::String(_)) | Some(Token::Int(_)) | Some(Token::Float(_))
            ) {
                let cp = self.checkpoint();
                if let Some(Spanned {
                    value: lit,
                    span_id: span,
                }) = self.parse_literal()
                {
                    let result_ty = self.parse_variant_result_ty(expr_parser);
                    let cp_after_lit = self.checkpoint();
                    self.parse_doc_comment();
                    // Skip Indent/Dedent tokens that may precede a continuation `or`
                    while self.eat(&Token::Indent) || self.eat(&Token::Dedent) {}
                    if self.is_at(&Token::Or) {
                        self.rewind(cp_after_lit);
                        break 'union_check Some((
                            Spanned {
                                value: Pattern::Literal(lit, span),
                                span_id: span,
                            },
                            result_ty,
                        ));
                    }
                    self.rewind(cp_after_lit);
                }
                // Not a union — rewind and try tag pattern
                self.rewind(cp);
            }
            // Standard checkpoint approach for tag patterns
            let cp = self.checkpoint();
            if let Some(shape) = self.parse_is_pattern_tag(expr_parser) {
                let result_ty = self.parse_variant_result_ty(expr_parser);
                let cp_after_shape = self.checkpoint();
                self.parse_doc_comment();
                // Skip Indent/Dedent tokens that may precede a continuation `or`
                // on an indented line (e.g. `Type is Variant(x)
                //                            or OtherVariant(y)`)
                while self.eat(&Token::Indent) || self.eat(&Token::Dedent) {}
                if self.is_at(&Token::Or) {
                    self.rewind(cp_after_shape);
                    break 'union_check Some((shape, result_ty));
                }
                self.rewind(cp_after_shape);
            }
            self.rewind(cp);
            None
        };

        if let Some((first_shape, first_result_ty)) = first_variant_parts {
            // Union: Tag (or Tag)+
            let first_post_doc = self.parse_doc_comment();

            let first_variant =
                Self::make_variant(first_doc, first_shape, first_post_doc, first_result_ty);
            let mut variants = vec![first_variant];

            let mut had_dedent_in_exit = false;
            loop {
                let mut saw_dedent = false;
                while self.eat(&Token::Indent) || {
                    let d = self.eat(&Token::Dedent);
                    if d {
                        saw_dedent = true;
                    }
                    d
                } {}
                if !self.eat(&Token::Or) {
                    had_dedent_in_exit = saw_dedent;
                    break;
                }
                let doc_on_or_line = self.parse_doc_comment();
                self.eat(&Token::Indent);

                match self.parse_variant(expr_parser) {
                    Some(next_variant) => {
                        Self::attach_doc_to_previous(&mut variants, doc_on_or_line);
                        variants.push(next_variant);
                    }
                    None => {
                        self.error(
                            "parse-expected-variant",
                            "expected variant after 'or'",
                            self.current_span(),
                        );
                        break;
                    }
                }
            }

            // Only consume doc comment if immediately after last variant (same line).
            // If a Dedent was consumed when exiting the loop, the doc is on a NEW line
            // and belongs to the next top-level element, not this union.
            let post_doc =
                if !had_dedent_in_exit && matches!(self.peek_at(0), Some(Token::DocComment(_))) {
                    self.parse_doc_comment()
                } else {
                    None
                };
            return (DeclareValue::Union { variants }, post_doc, Vec::new());
        }

        // Not a union — rewind to before any doc comment and try as alias
        self.rewind(checkpoint);

        // Single literal variant (e.g., `X0 is 'x0'`, `Status is 'ready'`)
        if matches!(
            self.peek(),
            Some(Token::String(_)) | Some(Token::Int(_)) | Some(Token::Float(_))
        ) && let Some(variant) = self.parse_variant(expr_parser)
        {
            // Only consume doc comment if immediately after the value (same line)
            let doc = if matches!(self.raw_peek(), Some(Token::DocComment(_))) {
                self.parse_doc_comment()
            } else {
                None
            };
            return (
                DeclareValue::Union {
                    variants: vec![variant],
                },
                doc,
                Vec::new(),
            );
        }

        if self.is_at(&Token::SelfInstance) {
            let span_id = self.current_span();
            self.advance();
            let doc = if matches!(self.raw_peek(), Some(Token::DocComment(_))) {
                self.parse_doc_comment()
            } else {
                None
            };
            return (
                DeclareValue::Alias(Box::new(Spanned {
                    value: Expr::SelfRef,
                    span_id,
                })),
                doc,
                Vec::new(),
            );
        }

        // Interface composition: `InOut is Input and Output`.
        let composition_checkpoint = self.checkpoint();
        if let Some(sp) = self.parse_type_expr(expr_parser) {
            self.skip_indents();
            if self.eat(&Token::And) {
                let mut provided_traits = vec![Self::provided_trait_from_expr(sp)];
                loop {
                    self.skip_indents();
                    let Some(next) = self.parse_type_expr(expr_parser) else {
                        self.error(
                            "parse-expected-interface-name",
                            "expected interface name after 'and'",
                            self.current_span(),
                        );
                        break;
                    };
                    provided_traits.push(Self::provided_trait_from_expr(next));
                    self.skip_indents();
                    if !self.eat(&Token::And) {
                        break;
                    }
                }
                let doc = if matches!(self.raw_peek(), Some(Token::DocComment(_))) {
                    self.parse_doc_comment()
                } else {
                    None
                };
                return (DeclareValue::Has(Vec::new()), doc, provided_traits);
            }
        }
        self.rewind(composition_checkpoint);

        // Try pattern type expression (allows parens for params): `Register(value: 'rax')`
        if let Some(sp) = self.parse_type_expr(expr_parser) {
            let doc = if matches!(self.raw_peek(), Some(Token::DocComment(_))) {
                self.parse_doc_comment()
            } else {
                None
            };
            return (DeclareValue::Alias(Box::new(sp)), doc, Vec::new());
        }

        self.error(
            "parse-expected-declare-body",
            "expected type declaration body after 'is'",
            self.current_span(),
        );
        (
            DeclareValue::Alias(Box::new(Spanned {
                value: Expr::AnonymousTag(Intern::new(String::new())),
                span_id: self.current_span(),
            })),
            None,
            Vec::new(),
        )
    }

    fn reject_removed_marker_clause(&mut self) {
        if self.peek() != Some(&Token::And)
            || self.peek_at(1) != Some(&Token::Is)
            || !matches!(self.peek_at(2), Some(Token::Tag("Copy" | "Sized")))
        {
            return;
        }
        let span = self.peek_span().unwrap_or_else(|| self.current_span());
        self.advance();
        self.advance();
        self.advance();
        self.error(
            "parse-removed-marker-syntax",
            "`and is Copy` and `and is Sized` have been removed",
            span,
        );
    }

    fn provided_trait_from_expr(sp: Spanned<Expr>) -> ProvidedTrait {
        let (trait_name, trait_name_span) = match sp.value {
            Expr::AnonymousTag(name) => (name, sp.span_id),
            Expr::TagCall(call) => (call.name, sp.span_id),
            Expr::FnCall(call) => (
                *call
                    .path
                    .value
                    .segments
                    .last()
                    .unwrap_or(&call.path.value.root),
                call.path.span_id,
            ),
            _ => (Intern::<String>::from_ref(""), sp.span_id),
        };
        ProvidedTrait {
            trait_name,
            trait_name_span,
            fields: Vec::new(),
        }
    }

    fn parse_variant_result_ty(&mut self, expr_parser: ExprFn) -> Option<Box<Spanned<Expr>>> {
        if self.is_at(&Token::Minus) && self.peek_at(1) == Some(&Token::Greater) {
            self.advance();
            self.advance();
            self.parse_type_expr(expr_parser).map(Box::new)
        } else {
            None
        }
    }

    fn parse_variant(&mut self, expr_parser: ExprFn) -> Option<Variant> {
        let doc_before = self.parse_doc_comment();

        // Try literal variant first (string, int, or float)
        if matches!(
            self.peek(),
            Some(Token::String(_)) | Some(Token::Int(_)) | Some(Token::Float(_))
        ) && let Some(Spanned {
            value: lit,
            span_id: span,
        }) = self.parse_literal()
        {
            let shape = Box::new(Spanned {
                value: Pattern::Literal(lit, span),
                span_id: span,
            });
            let result_ty = self.parse_variant_result_ty(expr_parser);
            let doc_after = if matches!(self.raw_peek(), Some(Token::DocComment(_))) {
                self.parse_doc_comment()
            } else {
                None
            };
            let doc = DocComment::combine(doc_before, doc_after);
            return Some(match doc.filter(|d| !d.is_empty()) {
                Some(d) => Variant::Local {
                    doc_comment: Some(d),
                    shape,
                    result_ty,
                },
                None => Variant::External { shape, result_ty },
            });
        }

        // Fall through to tag-based pattern
        let shape = self.parse_is_pattern_tag(expr_parser)?;
        let sp = Box::new(shape);
        let result_ty = self.parse_variant_result_ty(expr_parser);

        let doc_after = if matches!(self.raw_peek(), Some(Token::DocComment(_))) {
            self.parse_doc_comment()
        } else {
            None
        };

        let doc = DocComment::combine(doc_before, doc_after);
        Some(match doc.filter(|d| !d.is_empty()) {
            Some(d) => Variant::Local {
                doc_comment: Some(d),
                shape: sp,
                result_ty,
            },
            None => Variant::External {
                shape: sp,
                result_ty,
            },
        })
    }

    fn collect_qualified_property_provisions(
        type_name: Intern<String>,
        value: &DeclareValue,
        provided_traits: &mut [ProvidedTrait],
    ) {
        let DeclareValue::Has(members) = value else {
            return;
        };
        for member in members {
            let HasMember::Property(property) = member else {
                continue;
            };
            let Some(qualifier) = &property.qualifier else {
                continue;
            };
            if qualifier.name == type_name {
                continue;
            }
            let Some(body) = &property.body else {
                continue;
            };
            let Some(provided) = provided_traits
                .iter_mut()
                .find(|provided| provided.trait_name == qualifier.name)
            else {
                continue;
            };
            let Some(value) = body.as_expr().cloned() else {
                continue;
            };
            provided.fields.push((property.name, value));
        }
    }

    fn parse_trait_fields(&mut self, expr_parser: ExprFn) -> Option<ParsedTraitFields> {
        let (trait_name, trait_name_span) = match self.peek() {
            Some(Token::Tag(n)) => {
                let name = self.intern(n);
                let span = self.peek_span().unwrap_or(self.current_span());
                self.advance();
                (name, span)
            }
            _ => {
                self.error(
                    "parse-expected-trait-name",
                    "expected trait name after `has`",
                    self.current_span(),
                );
                return None;
            }
        };

        let fields = if self.eat(&Token::Has) {
            let has_block_body = self.eat(&Token::Indent);
            self.parse_provided_trait_field_list(expr_parser, has_block_body)?
        } else if self.is_at(&Token::ParenOpen) {
            self.error(
                "parse-removed-provision-parens",
                format!(
                    "`Trait(...)` provision syntax is no longer valid; use `Type.{} has ...` instead",
                    trait_name.as_str(),
                ),
                self.current_span(),
            );
            return None;
        } else {
            Vec::new()
        };
        Some(ParsedTraitFields {
            trait_name,
            trait_name_span,
            fields,
        })
    }

    fn parse_provided_trait_field_list(
        &mut self,
        expr_parser: ExprFn,
        has_block_body: bool,
    ) -> Option<Vec<(Intern<String>, Typed<Expr>)>> {
        let mut fields = Vec::new();

        loop {
            if matches!(self.raw_peek(), Some(Token::Dedent)) {
                break;
            }
            self.skip_newlines();

            if has_block_body && !matches!(self.peek(), Some(Token::Id(_))) {
                break;
            }

            let field_name = match self.peek() {
                Some(Token::Id(n)) => {
                    let id = self.intern(n);
                    self.advance();
                    id
                }
                _ => {
                    self.error(
                        "parse-expected-field-name",
                        "expected field name in trait parameters",
                        self.current_span(),
                    );
                    return None;
                }
            };
            self.expect(&Token::Colon)?;
            fields.push((field_name, expr_parser(self)));

            if self.eat(&Token::Comma) {
                self.skip_layout();
            } else if !has_block_body {
                break;
            }
        }

        Some(fields)
    }

    /// Parse a provided trait chain after `is`: `and has Trait(...) and Other(...)`.
    /// Parse provided-trait clauses after a declaration.
    fn parse_provided_trait_clauses(&mut self, expr_parser: ExprFn) -> Vec<ProvidedTrait> {
        self.skip_indents();
        let checkpoint = self.checkpoint();
        if !self.eat(&Token::And) {
            return Vec::new();
        }
        self.skip_indents();
        if !self.eat(&Token::Has) {
            self.rewind(checkpoint);
            return Vec::new();
        }

        self.parse_trait_chain_after_has(expr_parser)
    }

    fn parse_trait_chain_after_has(&mut self, expr_parser: ExprFn) -> Vec<ProvidedTrait> {
        let mut traits = Vec::new();

        loop {
            let Some(parsed) = self.parse_trait_fields(expr_parser) else {
                break;
            };
            traits.push(ProvidedTrait {
                trait_name: parsed.trait_name,
                trait_name_span: parsed.trait_name_span,
                fields: parsed.fields,
            });

            let checkpoint = self.checkpoint();
            self.skip_indents();
            if !self.eat(&Token::And) {
                self.rewind(checkpoint);
                break;
            }
            self.skip_indents();
            self.eat(&Token::Has);
            if !matches!(self.peek(), Some(Token::Tag(_))) {
                self.rewind(checkpoint);
                break;
            }
        }

        traits
    }

    fn parse_params_for_declare(&mut self, expr_parser: ExprFn) -> Option<Parameters> {
        if !self.is_at(&Token::ParenOpen) {
            return None;
        }
        let close_token = Token::ParenClose;
        self.advance();

        self.skip_layout();

        let mut params = Parameters::new();
        let mut seen_default = false;

        if self.is_at(&close_token) {
            self.advance();
            return Some(params);
        }

        loop {
            if let Some((key, kind)) = self.parse_one_declare_param(expr_parser) {
                if seen_default
                    && !matches!(
                        kind.kind,
                        ParameterKind::Default(_) | ParameterKind::Inferred { .. }
                    )
                {
                    self.error(
                        "parse-parameter-after-default",
                        format!(
                            "positional parameter `{}` appears after a default parameter",
                            key.as_str()
                        ),
                        self.current_span(),
                    );
                }
                if matches!(
                    kind.kind,
                    ParameterKind::Default(_) | ParameterKind::Inferred { .. }
                ) {
                    seen_default = true;
                }
                params.insert(key, kind);
            }

            if self.is_at(&close_token) {
                break;
            }

            if self.eat_list_separator() {
                continue;
            }

            self.error(
                "parse-expected-param-separator",
                "expected ',' or newline between parameters",
                self.current_span(),
            );
            break;
        }

        self.skip_layout();
        self.expect(&close_token);
        Some(params)
    }

    fn parse_one_declare_param(
        &mut self,
        expr_parser: ExprFn,
    ) -> Option<(Intern<String>, Parameter)> {
        // Positional: bare Tag
        if matches!(self.peek(), Some(Token::Tag(_))) {
            let sp = self.parse_type_expr(expr_parser)?;
            let key = Intern::<String>::from_ref(
                Pattern::from_expr(sp.value.clone()).surface_mangle_name(),
            );
            return Some((
                key,
                Parameter::new(
                    sp.span_id,
                    ParameterKind::Tagged(Box::new(Spanned {
                        value: sp.value,
                        span_id: sp.span_id,
                    })),
                ),
            ));
        }

        // Named: id [Tag | id | : TypeExpr]
        let name = match self.peek() {
            Some(Token::Id(n)) => {
                let name_span = self.peek_span()?;
                let id = self.intern(n);
                // Consume the parameter identifier token after capturing its span.
                self.advance();
                (id, name_span)
            }
            _ => return None,
        };

        let (name, name_span) = name;
        // In declaration (type-level) position, `:` introduces a type default,
        // not a value default. This is how `Box(x, a: LibcAllocator)` works.
        if self.eat(&Token::Colon) {
            if matches!(self.peek(), Some(Token::Tag(_))) {
                let sp = self.parse_type_expr(expr_parser)?;
                return Some((
                    name,
                    Parameter::new(
                        name_span,
                        ParameterKind::Default(Box::new(Typed::infer(sp.value, sp.span_id))),
                    ),
                ));
            }
            let expr_checkpoint = self.checkpoint();
            let expr = expr_parser(self);
            let expression_ends_parameter = self.is_at(&Token::ParenClose)
                || self.is_at(&Token::Comma)
                || self.previous_token_was_newline_separator();
            if expression_ends_parameter {
                return Some((
                    name,
                    Parameter::new(name_span, ParameterKind::Default(Box::new(expr))),
                ));
            }
            self.rewind(expr_checkpoint);
            let sp = self.parse_type_expr(expr_parser)?;
            return Some((
                name,
                Parameter::new(
                    name_span,
                    ParameterKind::Default(Box::new(Typed::infer(sp.value, sp.span_id))),
                ),
            ));
        }

        self.parse_param_after_name(expr_parser, name, name_span, true)
    }

    fn make_variant(
        doc_before: Option<DocComment>,
        shape: Spanned<Pattern>,
        doc_after: Option<DocComment>,
        result_ty: Option<Box<Spanned<Expr>>>,
    ) -> Variant {
        let doc = DocComment::combine(doc_before, doc_after);
        let sp = Box::new(shape);
        match doc.filter(|d| !d.is_empty()) {
            Some(d) => Variant::Local {
                doc_comment: Some(d),
                shape: sp,
                result_ty,
            },
            None => Variant::External {
                shape: sp,
                result_ty,
            },
        }
    }

    fn attach_doc_to_previous(variants: &mut [Variant], doc: Option<DocComment>) {
        if let Some(doc) = doc.filter(|d| !d.is_empty())
            && let Some(prev) = variants.last_mut()
        {
            let placeholder = Variant::External {
                shape: Box::new(Spanned {
                    value: Pattern::Nominal(Intern::new(String::new()), SpanId::INVALID),
                    span_id: SpanId::INVALID,
                }),
                result_ty: None,
            };
            let prev_owned = std::mem::replace(prev, placeholder);
            *prev = match prev_owned {
                Variant::External { shape, result_ty } => Variant::Local {
                    doc_comment: Some(doc),
                    shape,
                    result_ty,
                },
                Variant::Local {
                    mut doc_comment,
                    shape,
                    result_ty,
                } => {
                    doc_comment = DocComment::combine(doc_comment, Some(doc));
                    Variant::Local {
                        doc_comment,
                        shape,
                        result_ty,
                    }
                }
            };
        }
    }

    /// Parse a single predicate after `and`, e.g. `< n`, `> 0`, `<= n + 1`.
    pub(crate) fn parse_one_predicate(&mut self) -> PredicateExpr {
        match self.peek() {
            Some(Token::Less) => {
                self.advance();
                PredicateExpr::Lt(self.parse_predicate_rhs())
            }
            Some(Token::Greater) => {
                self.advance();
                PredicateExpr::Gt(self.parse_predicate_rhs())
            }
            Some(Token::LessEq) => {
                self.advance();
                PredicateExpr::Le(self.parse_predicate_rhs())
            }
            Some(Token::GreaterEq) => {
                self.advance();
                PredicateExpr::Ge(self.parse_predicate_rhs())
            }
            Some(Token::Eq) => {
                self.advance();
                PredicateExpr::Eq(self.parse_predicate_rhs())
            }
            Some(Token::NotEq) => {
                self.advance();
                PredicateExpr::Ne(self.parse_predicate_rhs())
            }
            Some(Token::Not) if self.peek_at(1) == Some(&Token::Eq) => {
                self.advance();
                self.advance();
                PredicateExpr::Ne(self.parse_predicate_rhs())
            }
            Some(Token::Star) => {
                self.advance();
                let right = self.parse_proof_proposition();
                let proposition = match right {
                    ast::ProofProposition::Compare {
                        left,
                        relation,
                        right,
                    } => ast::ProofProposition::Compare {
                        left: ast::ProofTerm::Mul(
                            Box::new(ast::ProofTerm::Name(Intern::from_ref("self"))),
                            Box::new(left),
                        ),
                        relation,
                        right,
                    },
                    proposition => proposition,
                };
                PredicateExpr::Proposition(Box::new(proposition))
            }
            Some(Token::Id(_) | Token::Tag(_) | Token::SelfInstance | Token::ParenOpen) => {
                PredicateExpr::Proposition(Box::new(self.parse_proof_proposition()))
            }
            _ => {
                self.error(
                    "parse-expected-predicate",
                    "expected predicate after 'and' (e.g. `< n`, `> 0`)",
                    self.current_span(),
                );
                PredicateExpr::Lt(NormalExpr::from(0))
            }
        }
    }

    /// Parse the right-hand side of a predicate: a name or literal.
    pub(crate) fn parse_predicate_rhs(&mut self) -> NormalExpr {
        self.skip_layout();
        let Some((token, _span)) = self.advance() else {
            self.error(
                "parse-expected-predicate-value",
                "expected value after predicate operator",
                self.current_span(),
            );
            return NormalExpr::from(0);
        };
        match token {
            Token::Id(name) => NormalExpr::Var(Intern::from_ref(name)),
            Token::Tag(name) => NormalExpr::Var(Intern::from_ref(name)),
            Token::Int(n) => NormalExpr::from(n),
            Token::Pound => {
                let kind = match self.advance() {
                    Some((Token::Id("size"), _)) => ast::TargetQueryKind::Size,
                    Some((Token::Id("alignment"), _)) => ast::TargetQueryKind::Alignment,
                    Some((Token::Id("index_bits"), _)) => ast::TargetQueryKind::IndexBits,
                    _ => ast::TargetQueryKind::Size,
                };
                self.expect(&Token::ParenOpen);
                let operand = self.parse_type_reference().unwrap_or_else(|| {
                    ast::TypeReference::Nominal(Intern::from_ref("__parse_error"))
                });
                self.expect(&Token::ParenClose);
                NormalExpr::TargetQuery { kind, operand }
            }
            _ => {
                self.error(
                    "parse-expected-predicate-value",
                    "expected value after predicate operator",
                    self.current_span(),
                );
                NormalExpr::from(0)
            }
        }
    }

    pub(crate) fn parse_symbolic_range(&mut self) -> Option<(NormalExpr, NormalExpr)> {
        let start = self.parse_symbolic_integer_atom()?;
        if !self.eat(&Token::Infer) {
            return None;
        }
        let end = self.parse_symbolic_integer_atom()?;
        Some((start, end))
    }

    fn parse_symbolic_integer_atom(&mut self) -> Option<NormalExpr> {
        let negative = self.eat(&Token::Minus);
        let value = match self.peek()? {
            Token::Int(value) => {
                let value = *value;
                self.advance();
                NormalExpr::from(value)
            }
            Token::Id(name) | Token::Tag(name) => {
                let name = Intern::from_ref(*name);
                self.advance();
                NormalExpr::Var(name)
            }
            _ => return None,
        };
        if negative {
            Some(NormalExpr::Sub(
                Box::new(NormalExpr::from(0)),
                Box::new(value),
            ))
        } else {
            Some(value)
        }
    }
}

struct ParsedTraitFields {
    trait_name: Intern<String>,
    trait_name_span: SpanId,
    fields: Vec<(Intern<String>, Typed<Expr>)>,
}
#[cfg(test)]
#[path = "../tests/declare_tests.rs"]
mod tests;
