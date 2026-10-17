use crate::cursor::TokenCursor;
use crate::expr::ExprFn;
use ast::declare::HasMemberQualifier;
use ast::span::SpanId;
use ast::{
    Declare, DeclareValue, DocComment, Expr, HasFunction, HasFunctionKind, HasMember,
    HasMemberBody, HasProperty, NormalExpr, Parameter, ParameterKind, Parameters, Pattern,
    PredicateExpr, ProvidedTrait, Spanned, Typed, Variant,
};
use diagnostic::DiagnosticCode;
use diagnostic::code::{ExpectedSyntaxCode, ParseCode, RemovedSyntaxCode, SyntaxCode};
use i256::I256;
use internment::Intern;
use lexer::Token;

type ParsedVariantParts = (
    Spanned<Pattern>,
    Option<Box<Spanned<Expr>>>,
    Option<Spanned<I256>>,
);

impl<'src, 't> TokenCursor<'src, 't> {
    pub fn parse_declare(&mut self, expr_parser: ExprFn) -> Option<Declare> {
        let checkpoint = self.checkpoint();
        let doc_before = self.parse_doc_comment();
        if doc_before.is_some() {
            self.skip_newlines();
        }
        let attrs = self.parse_declare_attributes();
        if attrs.is_some() && self.attribute_is_detached_from_next_token() {
            // parse-detached-attribute
            self.error_here(
                DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::DetachedAttribute)),
                "attribute must be on the line directly above the item it attaches to",
            );
        }
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
            let keyword_span = self.last_consumed_span();
            let doc_after_is = self.parse_doc_comment();
            self.eat(&Token::Indent);

            let (value, doc_after_value, mut provided_traits) = self.parse_is_rhs(expr_parser);
            if params.as_ref().is_some_and(|params| !params.is_empty())
                && let DeclareValue::Alias(alias) = &value
                && matches!(alias.value, Expr::TakePtr(_))
            {
                // parse-removed-pointer-former
                self.error(
                    DiagnosticCode::Parse(ParseCode::Removed(RemovedSyntaxCode::PointerFormer)),
                    "generic raw-pointer formers have been removed; use @x directly",
                    alias.span_id,
                );
            }
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
            decl.keyword_span = keyword_span;
            if let Some(a) = attrs {
                decl = decl.with_attributes(a);
            }
            Some(decl)
        } else if self.eat(&Token::Has) {
            let keyword_span = self.last_consumed_span();
            self.parse_doc_comment();
            let has_block_body = self.eat(&Token::Indent);

            let mut provided_traits_span = None;
            let (value, provided_traits) = if self.is_at(&Token::ParenOpen) {
                // parse-removed-has-parens
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Removed(RemovedSyntaxCode::HasParens)),
                    "parenthesized `has` members are no longer valid; remove the parentheses",
                );
                self.recover_removed_parenthesized_has_rhs();
                (DeclareValue::Has(Vec::new()), Vec::new())
            } else if !matches!(self.raw_peek(), Some(Token::Newline))
                && matches!(self.peek(), Some(Token::Tag(_)))
            {
                let start = self.current_span();
                let provided_traits = self.parse_trait_chain_after_has(expr_parser);
                provided_traits_span = Some(self.merge_span(start, self.last_consumed_span()));
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
            decl.provided_traits_span = provided_traits_span;
            decl.keyword_span = keyword_span;
            if let Some(a) = attrs {
                decl = decl.with_attributes(a);
            }
            Some(decl)
        } else {
            // parse-expected-declare-keyword
            self.error_here(
                DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::DeclarationKeyword)),
                "expected 'is' or 'has'",
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
        let mut private_block: Option<ast::BindAttributes> = None;
        let mut private_block_doc = None;
        loop {
            self.skip_newlines();
            if !has_block_body
                && matches!(self.peek(), Some(Token::Tag(_)))
                && self.peek_at(1) != Some(&Token::Dot)
            {
                break;
            }
            if self.is_at(&Token::ParenClose) {
                if !has_block_body {
                    self.advance();
                }
                break;
            }
            if self.is_at(&Token::Dedent) {
                if private_block.take().is_some() {
                    self.advance();
                    continue;
                }
                break;
            }
            let member_doc =
                DocComment::combine(private_block_doc.take(), self.parse_doc_comment());
            self.skip_newlines();
            let mut attributes = self.parse_bind_attributes().unwrap_or_default();
            if self.is_at(&Token::Indent) {
                if !self.validate_private_block(
                    &attributes,
                    has_block_body,
                    private_block.is_some(),
                ) {
                    break;
                }
                self.advance();
                private_block = Some(attributes);
                private_block_doc = member_doc;
                continue;
            }
            if let Some(inherited) = &private_block {
                if attributes.is_private() {
                    // parse-private-block-override
                    self.error(
                        DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::PrivateBlockOverride)),
                        concat!(
                            "members inherit the private block's module grants; ",
                            "use a separate block to change them",
                        ),
                        self.current_span(),
                    );
                    break;
                }
                attributes
                    .raw_attributes
                    .get_or_insert_default()
                    .extend(inherited.raw_attributes.iter().flatten().cloned());
            }
            let Some(member) = self.parse_has_member(expr_parser, member_doc, attributes) else {
                break;
            };
            members.push(member);
            if !self.parse_has_member_separator(has_block_body, private_block.is_some()) {
                break;
            }
        }
        DeclareValue::Has(members)
    }

    fn validate_private_block(
        &mut self,
        attributes: &ast::BindAttributes,
        has_block_body: bool,
        nested: bool,
    ) -> bool {
        if !has_block_body
            || !attributes.is_private()
            || attributes.raw_attributes.as_ref().map(Vec::len) != Some(1)
        {
            // parse-invalid-private-block
            self.error_here(
                DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::InvalidPrivateBlock)),
                "only #private or #private(modules) may introduce a block inside a has body",
            );
            return false;
        }
        if nested {
            // parse-nested-private-block
            self.error_here(
                DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::NestedPrivateBlock)),
                "private blocks cannot be nested",
            );
            return false;
        }
        true
    }

    fn parse_has_member(
        &mut self,
        expr_parser: ExprFn,
        doc_comment: Option<DocComment>,
        attributes: ast::BindAttributes,
    ) -> Option<HasMember> {
        let (qualifier, name, name_span) = self.parse_has_member_name()?;
        if self.is_at(&Token::ParenOpen) {
            let (params, conventions, param_groups, param_refinements) =
                self.parse_params(expr_parser);
            let mut return_ty = self.parse_method_return_type(expr_parser, true);
            let mut payload_postconditions = Vec::new();
            let first_name = match return_ty.as_deref_mut().map(|ty| &mut ty.value) {
                Some(Expr::SequenceLiteral(ast::SequenceLiteral::Elements(sequence)))
                    if sequence.len() == 1
                        && matches!(self.peek(), Some(Token::Thus | Token::Or)) =>
                {
                    let result = sequence.pop().unwrap();
                    let name = result.name;
                    return_ty = Some(Box::new(Spanned::new(
                        result.expr.value,
                        result.expr.span_id,
                    )));
                    name
                }
                _ => None,
            };
            let anonymous_result_alternatives = self.parse_result_alternatives(
                &mut return_ty,
                expr_parser,
                first_name,
                &mut payload_postconditions,
            );
            let refinement = self.parse_has_member_refinement();
            let body = self.parse_has_member_body(expr_parser);
            let params = params.unwrap_or_else(Parameters::new);
            let kind = if params.contains_key(&Intern::from_ref("self")) {
                HasFunctionKind::Instance
            } else {
                HasFunctionKind::Associated
            };
            return Some(HasMember::Function(Box::new(HasFunction {
                qualifier,
                name,
                name_span,
                params: Box::new(params),
                conventions: Box::new(conventions),
                param_groups: Box::new(param_groups),
                param_refinements: Box::new(param_refinements),
                return_ty,
                anonymous_result_alternatives,
                payload_postconditions,
                body,
                doc_comment,
                refinement,
                kind,
                attributes,
            })));
        }
        Some(HasMember::Property(Box::new(self.parse_has_property(
            expr_parser,
            qualifier,
            (name, name_span),
            doc_comment,
            attributes,
        ))))
    }

    fn parse_has_property(
        &mut self,
        expr_parser: ExprFn,
        qualifier: Option<HasMemberQualifier>,
        (name, name_span): (Intern<String>, SpanId),
        doc_comment: Option<DocComment>,
        attributes: ast::BindAttributes,
    ) -> HasProperty {
        let owned_storage = self.eat(&Token::Own);
        let storage_span = self.last_consumed_span();
        let mut ty = self.parse_method_return_type(expr_parser, false);
        if owned_storage {
            if let Some(ty) = ty.as_mut() {
                ty.span_id = self.merge_span(storage_span, ty.span_id);
            } else {
                // parse-expected-owned-storage-type
                self.error(
                    DiagnosticCode::Parse(ParseCode::Expected(
                        ExpectedSyntaxCode::OwnedStorageType,
                    )),
                    "expected an element type after 'own' on a field",
                    name_span,
                );
            }
        }
        let refinement = self.parse_has_member_refinement();
        let body = self.parse_has_member_body(expr_parser);
        HasProperty {
            qualifier,
            name,
            name_span,
            ty,
            owned_storage,
            attributes,
            body,
            doc_comment,
            refinement,
        }
    }

    fn parse_has_member_name(
        &mut self,
    ) -> Option<(Option<HasMemberQualifier>, Intern<String>, SpanId)> {
        let qualifier =
            if matches!(self.peek(), Some(Token::Tag(_))) && self.peek_at(1) == Some(&Token::Dot) {
                let Some((Token::Tag(name), span)) = self.advance() else {
                    unreachable!()
                };
                let qualifier = HasMemberQualifier {
                    name: self.intern(name),
                    span,
                };
                self.advance();
                Some(qualifier)
            } else {
                None
            };
        match self.peek() {
            Some(Token::Id(name)) => {
                let name = self.intern(name);
                let name_span = self.peek_span().expect("member name has a span");
                self.advance();
                Some((qualifier, name, name_span))
            }
            None if qualifier.is_none() => None,
            _ => {
                let message = if qualifier.is_some() {
                    "expected method name after trait qualifier"
                } else {
                    "expected method name in interface body"
                };
                // parse-expected-method-name
                self.error(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::MethodName)),
                    message,
                    self.current_span(),
                );
                None
            }
        }
    }

    fn parse_has_member_refinement(&mut self) -> Option<PredicateExpr> {
        if !self.eat(&Token::And) {
            return None;
        }
        let mut predicates = vec![self.parse_one_predicate()];
        while self.eat(&Token::And) {
            predicates.push(self.parse_one_predicate());
        }
        Some(if predicates.len() == 1 {
            predicates.pop().unwrap()
        } else {
            PredicateExpr::And(predicates)
        })
    }

    fn parse_has_member_body(&mut self, expr_parser: ExprFn) -> Option<HasMemberBody> {
        if self.eat(&Token::ColonEq) {
            let (value, _) = self.parse_bind_value(expr_parser);
            Some(HasMemberBody::Final(value))
        } else if self.eat(&Token::Colon) {
            let (value, _) = self.parse_bind_value(expr_parser);
            Some(HasMemberBody::Overrideable(value))
        } else {
            None
        }
    }

    fn parse_has_member_separator(&mut self, has_block_body: bool, private_block: bool) -> bool {
        if self.eat(&Token::Comma) {
            if has_block_body {
                self.skip_newlines();
                self.skip_indents();
            } else {
                self.skip_layout();
            }
            if matches!(self.raw_peek(), Some(Token::Dedent | Token::ParenClose)) {
                if private_block && self.is_at(&Token::Dedent) {
                    return true;
                }
                if self.eat(&Token::ParenClose) {
                    self.eat(&Token::Dedent);
                }
                return false;
            }
            return true;
        }
        if has_block_body {
            let checkpoint = self.checkpoint();
            self.skip_newlines();
            if matches!(self.raw_peek(), Some(Token::Dedent | Token::Comma)) {
                if private_block && self.is_at(&Token::Dedent) {
                    return true;
                }
                self.rewind(checkpoint);
                return false;
            }
            return self.checkpoint() != checkpoint || self.previous_token_was_newline_separator();
        }
        false
    }

    fn parse_method_return_type(
        &mut self,
        expr_parser: ExprFn,
        is_function: bool,
    ) -> Option<Box<Spanned<Expr>>> {
        if !is_function || matches!(self.raw_peek(), Some(Token::Newline)) {
            return self.parse_method_result_type(expr_parser);
        }
        let start = self.current_span();
        let name = self.parse_result_element_name();
        let ty = self.parse_method_result_type(expr_parser)?;
        if name.is_none() && !self.has_more_method_results() {
            return Some(ty);
        }
        let mut elems = vec![ast::SequenceElement {
            name,
            expr: ast::Typed::infer(ty.value, ty.span_id),
        }];
        while self.has_more_method_results() {
            self.advance();
            let name = self.parse_result_element_name();
            let Some(ty) = self.parse_type_expr(expr_parser) else {
                // parse-expected-result-type
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::ResultType)),
                    "expected a result type after ','",
                );
                break;
            };
            elems.push(ast::SequenceElement {
                name,
                expr: ast::Typed::infer(ty.value, ty.span_id),
            });
        }
        let sequence = self.sequence_value(elems, start);
        Some(Box::new(Spanned::new(sequence.value, sequence.span_id)))
    }

    fn has_more_method_results(&self) -> bool {
        self.is_at(&Token::Comma)
            && matches!(
                self.peek_at(1),
                Some(
                    Token::Tag(_)
                        | Token::Id(_)
                        | Token::Ref
                        | Token::Mut
                        | Token::At
                        | Token::BracketOpen
                        | Token::SelfTag
                )
            )
            && !matches!(
                (self.peek_at(1), self.peek_at(2)),
                (Some(Token::Id(_)), Some(Token::ParenOpen))
            )
    }

    fn parse_result_element_name(&mut self) -> Option<Intern<String>> {
        if matches!(self.peek(), Some(Token::Id(_)))
            && matches!(
                self.peek_at(1),
                Some(
                    Token::Tag(_)
                        | Token::SelfTag
                        | Token::Id(_)
                        | Token::Ref
                        | Token::Mut
                        | Token::At
                        | Token::BracketOpen
                )
            )
        {
            self.parse_name()
        } else {
            None
        }
    }

    fn parse_method_result_type(&mut self, expr_parser: ExprFn) -> Option<Box<Spanned<Expr>>> {
        if matches!(self.raw_peek(), Some(Token::Newline)) {
            return None;
        }
        if self.eat(&Token::Is) {
            let start = self.last_consumed_span();
            self.expect(&Token::In)?;
            let ty = self.parse_type_expr(expr_parser)?;
            return Some(Box::new(Spanned::new(
                Expr::IntegerDomain(Box::new(Typed::infer(ty.value, ty.span_id))),
                self.merge_span(start, self.last_consumed_span()),
            )));
        }
        self.parse_type_expr(expr_parser).map(Box::new)
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

        // InRange: `in` N to M
        if self.eat(&Token::In) {
            let checkpoint = self.checkpoint();
            if let Some((a, b)) = self.parse_int_range()
                && !self.is_at(&Token::And)
            {
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
            self.rewind(checkpoint);
            if let Some(domain) = self.parse_type_expr(expr_parser) {
                let span = domain.span_id;
                return (
                    DeclareValue::Alias(Box::new(Spanned::new(
                        Expr::IntegerDomain(Box::new(Typed::infer(domain.value, span))),
                        span,
                    ))),
                    None,
                    Vec::new(),
                );
            }
            // parse-expected-integer-range
            self.error_here(
                DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::IntegerRange)),
                "expected integer range after 'in'",
            );
            return (
                DeclareValue::InRange(I256::from_u128(0), I256::from_u128(0)),
                None,
                Vec::new(),
            );
        }

        // Range: [-]Int to [-]Int
        if matches!(self.peek(), Some(Token::Int(_)) | Some(Token::Minus)) {
            let checkpoint = self.checkpoint();
            if let Some((a, b)) = self.parse_int_range() {
                self.reject_removed_marker_clause();
                return (DeclareValue::Range(a, b), None, Vec::new());
            }
            self.rewind(checkpoint);
        }

        let range_checkpoint = self.checkpoint();
        if self.parse_symbolic_range().is_some() {
            self.rewind(range_checkpoint);
            let range = expr_parser(self);
            return (
                DeclareValue::Alias(Box::new(Spanned::new(range.value, range.span_id))),
                None,
                Vec::new(),
            );
        }
        self.rewind(range_checkpoint);

        // Unit type: `()`
        if self.is_at(&Token::ParenOpen) && self.peek_at(1) == Some(&Token::ParenClose) {
            self.advance(); // eat (
            self.advance(); // eat )
            let span = self.last_consumed_span();
            return (
                DeclareValue::Alias(Box::new(Spanned {
                    value: Expr::Lit(ast::Literal::Int(0.into())),
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
                            None,
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
                let numbered = self.is_at(&Token::ColonEq);
                let discriminant = self.parse_variant_discriminant();
                let cp_after_shape = self.checkpoint();
                self.parse_doc_comment();
                // Skip Indent/Dedent tokens that may precede a continuation `or`
                // on an indented line (e.g. `Type is Variant(x)
                //                            or OtherVariant(y)`)
                while self.eat(&Token::Indent) || self.eat(&Token::Dedent) {}
                if numbered || self.is_at(&Token::Or) {
                    self.rewind(cp_after_shape);
                    break 'union_check Some((shape, result_ty, discriminant));
                }
                self.rewind(cp_after_shape);
            }
            self.rewind(cp);
            None
        };

        if let Some((first_shape, first_result_ty, first_discriminant)) = first_variant_parts {
            // Union: Tag (or Tag)+
            let first_post_doc = self.parse_doc_comment();

            let first_variant = Self::make_variant(
                first_doc,
                first_shape,
                first_post_doc,
                first_result_ty,
                first_discriminant,
            );
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
                        // parse-expected-variant
                        self.error_here(
                            DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::Variant)),
                            "expected variant after 'or'",
                        );
                        break;
                    }
                }
            }

            // Only consume doc comment if immediately after last variant (same line).
            // If a Dedent was consumed when exiting the loop, the doc is on a NEW line
            // and belongs to the next top-level element, not this union.
            let post_doc = if !had_dedent_in_exit {
                self.parse_same_line_doc()
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
            let doc = self.parse_same_line_doc();
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
            let doc = self.parse_same_line_doc();
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
                        // parse-expected-interface-name
                        self.error_here(
                            DiagnosticCode::Parse(ParseCode::Expected(
                                ExpectedSyntaxCode::InterfaceName,
                            )),
                            "expected interface name after 'and'",
                        );
                        break;
                    };
                    provided_traits.push(Self::provided_trait_from_expr(next));
                    self.skip_indents();
                    if !self.eat(&Token::And) {
                        break;
                    }
                }
                let doc = self.parse_same_line_doc();
                return (DeclareValue::Has(Vec::new()), doc, provided_traits);
            }
        }
        self.rewind(composition_checkpoint);

        // Try pattern type expression (allows parens for params): `Register(value: 'rax')`
        if let Some(sp) = self.parse_type_expr(expr_parser) {
            let doc = self.parse_same_line_doc();
            return (DeclareValue::Alias(Box::new(sp)), doc, Vec::new());
        }

        // parse-expected-declare-body
        self.error_here(
            DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::DeclarationBody)),
            "expected type declaration body after 'is'",
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
        // parse-removed-marker-syntax
        self.error(
            DiagnosticCode::Parse(ParseCode::Removed(RemovedSyntaxCode::MarkerSyntax)),
            "`and is Copy` and `and is Sized` have been removed",
            span,
        );
    }

    fn provided_trait_from_expr(sp: Spanned<Expr>) -> ProvidedTrait {
        let (trait_name, trait_name_span, args) = match sp.value {
            Expr::AnonymousTag(name) => (name, sp.span_id, Vec::new()),
            Expr::TagCall(call) => (call.name, sp.span_id, call.args),
            Expr::FnCall(call) => (
                *call
                    .path
                    .value
                    .segments
                    .last()
                    .unwrap_or(&call.path.value.root),
                call.path.span_id,
                call.args.unwrap_or_default(),
            ),
            _ => (Intern::<String>::from_ref(""), sp.span_id, Vec::new()),
        };
        ProvidedTrait {
            trait_name,
            trait_name_span,
            args,
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
        let (shape, result_ty, discriminant) = if matches!(
            self.peek(),
            Some(Token::String(_)) | Some(Token::Int(_)) | Some(Token::Float(_))
        ) && let Some(Spanned {
            value: lit,
            span_id: span,
        }) = self.parse_literal()
        {
            (
                Box::new(Spanned {
                    value: Pattern::Literal(lit, span),
                    span_id: span,
                }),
                self.parse_variant_result_ty(expr_parser),
                None,
            )
        } else {
            // Fall through to tag-based pattern
            let shape = Box::new(self.parse_is_pattern_tag(expr_parser)?);
            let result_ty = self.parse_variant_result_ty(expr_parser);
            let discriminant = self.parse_variant_discriminant();
            (shape, result_ty, discriminant)
        };

        let doc_after = self.parse_same_line_doc();
        let doc = DocComment::combine(doc_before, doc_after);
        match doc.filter(|d| !d.is_empty()) {
            Some(d) => Some(Variant::Local {
                doc_comment: Some(d),
                shape,
                result_ty,
                discriminant,
            }),
            None => Some(Variant::External {
                shape,
                result_ty,
                discriminant,
            }),
        }
    }

    fn parse_variant_discriminant(&mut self) -> Option<Spanned<I256>> {
        if !self.eat(&Token::ColonEq) {
            return None;
        }
        let start = self.current_span();
        let value = self.parse_signed_int();
        if let Some(value) = value {
            Some(Spanned::new(
                value,
                self.merge_span(start, self.last_consumed_span()),
            ))
        } else {
            // parse-expected-discriminant-integer
            self.error(
                DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::DiscriminantInteger)),
                "expected an integer literal after ':=' in a union variant",
                start,
            );
            None
        }
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
                // parse-expected-trait-name
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::TraitName)),
                    "expected trait name after `has`",
                );
                return None;
            }
        };

        let args = self.parse_paren_args().unwrap_or_default();
        let fields = if self.eat(&Token::Has) {
            let has_block_body = self.eat(&Token::Indent);
            self.parse_provided_trait_field_list(expr_parser, has_block_body)?
        } else {
            Vec::new()
        };
        Some(ParsedTraitFields {
            trait_name,
            trait_name_span,
            args,
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
                    // parse-expected-field-name
                    self.error_here(
                        DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::FieldName)),
                        "expected field name in trait parameters",
                    );
                    return None;
                }
            };
            self.expect(&Token::Colon)?;
            fields.push((field_name, expr_parser(self)));

            let has_separator = if has_block_body {
                self.eat_list_separator()
            } else {
                self.eat(&Token::Comma)
            };

            if has_separator {
                self.skip_layout();
                continue;
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

    pub(crate) fn parse_trait_chain_after_has(
        &mut self,
        expr_parser: ExprFn,
    ) -> Vec<ProvidedTrait> {
        let mut traits = Vec::new();

        loop {
            let Some(parsed) = self.parse_trait_fields(expr_parser) else {
                break;
            };
            traits.push(ProvidedTrait {
                trait_name: parsed.trait_name,
                trait_name_span: parsed.trait_name_span,
                args: parsed.args,
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
                    // parse-parameter-after-default
                    self.error(
                        DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::ParameterAfterDefault)),
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

            // parse-expected-param-separator
            self.error_here(
                DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::ParameterSeparator)),
                "expected ',' or newline between parameters",
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
        if let Some(Token::Id(start)) = self.peek().copied()
            && self.peek_at(1) == Some(&Token::To)
        {
            let span = self.current_span();
            self.advance();
            self.advance();
            let end_span = self.current_span();
            let Some(Token::Id(end)) = self.peek().copied() else {
                // parse-range-parameter
                self.error(
                    DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::RangeParameter)),
                    "expected an endpoint binding after `to`",
                    end_span,
                );
                return None;
            };
            self.advance();
            let start = self.intern(start);
            let end = self.intern(end);
            if start == end {
                // parse-range-parameter
                self.error(
                    DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::RangeParameter)),
                    "range endpoints must have distinct binding names",
                    end_span,
                );
            }
            let mut parameter = Parameter::new(span, ParameterKind::Generic);
            parameter.range_pattern = Some(ast::parameter::RangeParameterPattern {
                start,
                end,
                end_span,
            });
            return Some((start, parameter));
        }
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

        let name_span = self.peek_span()?;
        let name = self.parse_name()?;
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
        discriminant: Option<Spanned<I256>>,
    ) -> Variant {
        let doc = DocComment::combine(doc_before, doc_after);
        let sp = Box::new(shape);
        match doc.filter(|d| !d.is_empty()) {
            Some(d) => Variant::Local {
                doc_comment: Some(d),
                shape: sp,
                result_ty,
                discriminant,
            },
            None => Variant::External {
                shape: sp,
                result_ty,
                discriminant,
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
                discriminant: None,
            };
            let prev_owned = std::mem::replace(prev, placeholder);
            *prev = match prev_owned {
                Variant::External {
                    shape,
                    result_ty,
                    discriminant,
                } => Variant::Local {
                    doc_comment: Some(doc),
                    shape,
                    result_ty,
                    discriminant,
                },
                Variant::Local {
                    mut doc_comment,
                    shape,
                    result_ty,
                    discriminant,
                } => {
                    doc_comment = DocComment::combine(doc_comment, Some(doc));
                    Variant::Local {
                        doc_comment,
                        shape,
                        result_ty,
                        discriminant,
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
            Some(
                Token::Id(_)
                | Token::Tag(_)
                | Token::SelfInstance
                | Token::ParenOpen
                | Token::Int(_)
                | Token::Minus,
            ) => PredicateExpr::Proposition(Box::new(self.parse_proof_proposition())),
            _ => {
                // parse-expected-predicate
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::Predicate)),
                    "expected predicate after 'and' (e.g. `< n`, `> 0`)",
                );
                PredicateExpr::Lt(NormalExpr::from(0))
            }
        }
    }

    /// Parse the right-hand side of a predicate: a name or literal.
    pub(crate) fn parse_predicate_rhs(&mut self) -> NormalExpr {
        self.skip_layout();
        let Some((token, _span)) = self.advance() else {
            // parse-expected-predicate-value
            self.error_here(
                DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::PredicateValue)),
                "expected value after predicate operator",
            );
            return NormalExpr::from(0);
        };
        match token {
            Token::Id(name) | Token::Tag(name) => {
                NormalExpr::Var(self.parse_proof_member_path(Intern::from_ref(name)))
            }
            Token::SelfInstance => {
                NormalExpr::Var(self.parse_proof_member_path(Intern::from_ref("self")))
            }
            Token::Int(n) => NormalExpr::from(n),
            Token::Pound => {
                let kind = self.parse_target_query_kind();
                self.expect(&Token::ParenOpen);
                let operand = self.parse_type_reference().unwrap_or_else(|| {
                    ast::TypeReference::Nominal(Intern::from_ref("__parse_error"))
                });
                self.expect(&Token::ParenClose);
                NormalExpr::TargetQuery { kind, operand }
            }
            _ => {
                // parse-expected-predicate-value
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::PredicateValue)),
                    "expected value after predicate operator",
                );
                NormalExpr::from(0)
            }
        }
    }

    pub(crate) fn parse_symbolic_range(&mut self) -> Option<(NormalExpr, NormalExpr)> {
        let start = self.parse_symbolic_integer_atom()?;
        if !self.eat(&Token::To) {
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
    args: Vec<Typed<Expr>>,
    trait_name: Intern<String>,
    trait_name_span: SpanId,
    fields: Vec<(Intern<String>, Typed<Expr>)>,
}
#[cfg(test)]
#[path = "../tests/declare_tests.rs"]
mod tests;
