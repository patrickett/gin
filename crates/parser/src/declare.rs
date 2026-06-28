use indexmap::IndexMap;

use crate::cursor::TokenCursor;
use crate::expr::ExprFn;
use ast::span::SpanId;
use ast::{
    ConstExpr, Declare, DeclareValue, DocComment, Expr, InterfaceMember, ParameterKind, Parameters,
    PredicateExpr, ProvidedTrait, Spanned, TypeExpr, Typed, Variant,
};
use i256::I256;
use internment::Intern;
use lexer::Token;

/// `(return_type, error_type)` after parsing a method return type.
type ParseResultType = (
    Option<Box<Spanned<TypeExpr>>>,
    Option<Box<Spanned<TypeExpr>>>,
);

/// asd
impl<'src, 't> TokenCursor<'src, 't> {
    pub fn parse_declare(&mut self, expr_parser: ExprFn) -> Option<Declare> {
        let doc_before = self.parse_doc_comment();
        let attrs = self.parse_declare_attributes();
        self.eat(&Token::Indent);

        let (name, name_span) = match self.peek()? {
            &Token::Tag(n) => {
                let name = self.intern(n);
                let span = self.peek_span()?;
                self.advance();
                (name, span)
            }
            _ => return None,
        };

        let params = self.parse_params_for_declare(expr_parser);

        if self.eat(&Token::Is) {
            let doc_after_is = self.parse_doc_comment();
            self.eat(&Token::Indent);

            let (value, doc_after_value, mut provided_traits) = self.parse_is_rhs(expr_parser);
            provided_traits.extend(self.parse_provided_trait_clauses(expr_parser));
            self.eat(&Token::Dedent);
            self.reject_removed_marker_syntax();

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
            self.eat(&Token::Indent);

            let (value, provided_traits) = if self.is_at(&Token::ParenOpen) {
                let value = self.parse_has_rhs(expr_parser);
                let provided_traits = self.parse_provided_trait_clauses(expr_parser);
                (value, provided_traits)
            } else if matches!(self.peek(), Some(Token::Tag(_))) {
                (
                    DeclareValue::Interface(Vec::new()),
                    self.parse_trait_chain_after_has(expr_parser),
                )
            } else {
                (self.parse_has_rhs(expr_parser), Vec::new())
            };
            self.eat(&Token::Dedent);
            self.reject_removed_marker_syntax();

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
            self.error("expected 'is' or 'has'", self.current_span());
            None
        }
    }

    fn parse_declare_attributes(&mut self) -> Option<ast::DeclareAttributes> {
        if !self.is_at(&Token::Pound) {
            return None;
        }
        self.advance();

        self.expect(&Token::BracketOpen)?;

        let mut items: Vec<ast::AttributeItem> = Vec::new();

        if !self.is_at(&Token::BracketClose) {
            loop {
                if let Some(item) = self.parse_one_attribute_item() {
                    items.push(item);
                }
                if self.eat(&Token::Comma) {
                    continue;
                }
                break;
            }
        }

        self.expect(&Token::BracketClose);

        let mut attrs = ast::DeclareAttributes {
            raw_attributes: Some(items),
        };
        attrs.extract_intrinsic_attributes();
        Some(attrs)
    }

    fn parse_has_rhs(&mut self, expr_parser: ExprFn) -> DeclareValue {
        if !self.is_at(&Token::ParenOpen) {
            self.error(
                "expected interface body after 'has' (e.g. `has (method1, method2, ...)`)",
                self.current_span(),
            );
            return DeclareValue::Interface(Vec::new());
        }

        self.expect(&Token::ParenOpen);
        self.skip_indents();

        let mut members = Vec::new();

        loop {
            // Allow trailing comma before closing paren
            if self.is_at(&Token::ParenClose) {
                break;
            }

            // Doc comment attached to this member (skip newlines first)
            self.skip_newlines();
            let member_doc = self.parse_doc_comment();

            // Member name (must be a lowercase id)
            let member_name = match self.peek() {
                Some(Token::Id(n)) => {
                    let name = self.intern(n);
                    let name_span = self
                        .peek_span()
                        .expect("peek confirmed Id token, peek_span should succeed");
                    self.advance();
                    (name, name_span)
                }
                Some(_) => {
                    self.error(
                        "expected method name in interface body",
                        self.current_span(),
                    );
                    break;
                }
                None => break,
            };

            let (params, conventions, _param_groups) = if self.is_at(&Token::ParenOpen) {
                // Method with parameters: `name(params...)`
                self.parse_params(expr_parser)
            } else {
                // No params — method with empty parameter list
                (Some(Parameters::new()), IndexMap::new(), IndexMap::new())
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

            members.push(InterfaceMember {
                name: member_name.0,
                name_span: member_name.1,
                params: params.unwrap_or_else(Parameters::new),
                conventions,
                return_ty,
                error_ty,
                doc_comment: member_doc,
                refinement,
            });

            // Separator: comma or newline
            if self.is_at(&Token::ParenClose) {
                break;
            }
            if !self.eat_list_separator() {
                self.error(
                    "expected ',' or newline between interface members",
                    self.current_span(),
                );
                break;
            }
        }

        self.expect(&Token::ParenClose);

        if members.is_empty() {
            // Empty interface: `Allocator has ()`
        }

        DeclareValue::Interface(members)
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
            Some(Token::Tag(_)) | Some(Token::Ref) | Some(Token::Mut) | Some(Token::At)
        ) || matches!(self.peek(), Some(Token::ParenOpen) if self.peek_at(1) == Some(&Token::ParenClose));

        if !can_start {
            // Bare lowercase id as return type (e.g. `start x` where x is a type variable)
            if let Some(Token::Id(t)) = self.peek() {
                let name = self.intern(t);
                let span = self.peek_span().unwrap_or(SpanId::INVALID);
                self.advance();
                let sp = Spanned {
                    value: TypeExpr::Nominal(name, span),
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
        return_ty: Option<Box<Spanned<TypeExpr>>>,
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
            if let Some((a, b)) = self.parse_int_range() {
                return (DeclareValue::InRange(a, b), None, Vec::new());
            }
            self.error("expected integer range after 'in'", self.current_span());
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
                return (DeclareValue::Range(a, b), None, Vec::new());
            }
            self.rewind(checkpoint);
        }

        // Unit type: `()`
        if self.is_at(&Token::ParenOpen) && self.peek_at(1) == Some(&Token::ParenClose) {
            self.advance(); // eat (
            self.advance(); // eat )
            let span = self.last_consumed_span();
            let unit_expr = Spanned {
                value: TypeExpr::Unit,
                span_id: span,
            };
            return (DeclareValue::Alias(Box::new(unit_expr)), None, Vec::new());
        }

        // Union or Alias: starts with optional doc then a type pattern or literal
        let checkpoint = self.checkpoint();
        let first_doc = self.parse_doc_comment();

        // Try literal variant first (e.g. 'debug'), then tag pattern (e.g. Some(x))
        let first_shape: Option<Spanned<TypeExpr>> = 'union_check: {
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
                    let cp_after_lit = self.checkpoint();
                    self.parse_doc_comment();
                    // Skip Indent/Dedent tokens that may precede a continuation `or`
                    while self.eat(&Token::Indent) || self.eat(&Token::Dedent) {}
                    if self.is_at(&Token::Or) {
                        self.rewind(cp_after_lit);
                        break 'union_check Some(Spanned {
                            value: TypeExpr::Literal(lit, span),
                            span_id: span,
                        });
                    }
                    self.rewind(cp_after_lit);
                }
                // Not a union — rewind and try tag pattern
                self.rewind(cp);
            }
            // Standard checkpoint approach for tag patterns
            let cp = self.checkpoint();
            if let Some(shape) = self.parse_pattern_type_expr(expr_parser) {
                let cp_after_shape = self.checkpoint();
                self.parse_doc_comment();
                // Skip Indent/Dedent tokens that may precede a continuation `or`
                // on an indented line (e.g. `Type is Variant(x)
                //                            or OtherVariant(y)`)
                while self.eat(&Token::Indent) || self.eat(&Token::Dedent) {}
                if self.is_at(&Token::Or) {
                    self.rewind(cp_after_shape);
                    break 'union_check Some(shape);
                }
                self.rewind(cp_after_shape);
            }
            self.rewind(cp);
            None
        };

        if let Some(first_shape) = first_shape {
            // Union: Tag (or Tag)+
            let first_post_doc = self.parse_doc_comment();

            let first_variant = Self::make_variant(first_doc, first_shape, first_post_doc);
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
                        self.error("expected variant after 'or'", self.current_span());
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
            let doc = if matches!(self.peek_at(0), Some(Token::DocComment(_))) {
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

        // Interface composition: `InOut is Input and Output`.
        let composition_checkpoint = self.checkpoint();
        if let Some(sp) = self.parse_pattern_type_expr(expr_parser) {
            self.skip_indents();
            if self.eat(&Token::And) {
                let mut provided_traits = vec![Self::provided_trait_from_type_expr(sp)];
                loop {
                    self.skip_indents();
                    let Some(next) = self.parse_pattern_type_expr(expr_parser) else {
                        self.error("expected interface name after 'and'", self.current_span());
                        break;
                    };
                    provided_traits.push(Self::provided_trait_from_type_expr(next));
                    self.skip_indents();
                    if !self.eat(&Token::And) {
                        break;
                    }
                }
                let doc = if matches!(self.peek_at(0), Some(Token::DocComment(_))) {
                    self.parse_doc_comment()
                } else {
                    None
                };
                return (DeclareValue::Interface(Vec::new()), doc, provided_traits);
            }
        }
        self.rewind(composition_checkpoint);

        // Try pattern type expression (allows parens for params): `Register(value: 'rax')`
        if let Some(sp) = self.parse_pattern_type_expr(expr_parser) {
            let doc = if matches!(self.peek_at(0), Some(Token::DocComment(_))) {
                self.parse_doc_comment()
            } else {
                None
            };
            return (DeclareValue::Alias(Box::new(sp)), doc, Vec::new());
        }

        if let Some(sp) = self.parse_type_expr(expr_parser) {
            let doc = if matches!(self.peek_at(0), Some(Token::DocComment(_))) {
                self.parse_doc_comment()
            } else {
                None
            };
            return (DeclareValue::Alias(Box::new(sp)), doc, Vec::new());
        }

        self.error(
            "expected type declaration body after 'is'",
            self.current_span(),
        );
        (
            DeclareValue::Alias(Box::new(Spanned {
                value: TypeExpr::Nominal(Intern::new(String::new()), self.current_span()),
                span_id: self.current_span(),
            })),
            None,
            Vec::new(),
        )
    }

    fn provided_trait_from_type_expr(sp: Spanned<TypeExpr>) -> ProvidedTrait {
        let (trait_name, trait_name_span) = match sp.value {
            TypeExpr::Nominal(name, span) => (name, span),
            TypeExpr::Generic { name, span, .. } => (name, span),
            _ => (
                Intern::new(sp.value.surface_mangle_name().to_string()),
                sp.span_id,
            ),
        };
        ProvidedTrait {
            trait_name,
            trait_name_span,
            fields: Vec::new(),
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
                value: TypeExpr::Literal(lit, span),
                span_id: span,
            });
            // Only consume doc comment if immediately after the value (same line)
            let doc_after = if matches!(self.peek_at(0), Some(Token::DocComment(_))) {
                self.parse_doc_comment()
            } else {
                None
            };
            let doc = DocComment::combine(doc_before, doc_after);
            return Some(match doc.filter(|d| !d.is_empty()) {
                Some(d) => Variant::Local {
                    doc_comment: Some(d),
                    shape,
                },
                None => Variant::External(shape),
            });
        }

        // Fall through to tag-based pattern
        let shape = self.parse_pattern_type_expr(expr_parser)?;
        let sp = Box::new(shape);

        // Only consume doc comment if immediately after the variant (same line)
        let doc_after = if matches!(self.peek_at(0), Some(Token::DocComment(_))) {
            self.parse_doc_comment()
        } else {
            None
        };

        let doc = DocComment::combine(doc_before, doc_after);
        Some(match doc.filter(|d| !d.is_empty()) {
            Some(d) => Variant::Local {
                doc_comment: Some(d),
                shape: sp,
            },
            None => Variant::External(sp),
        })
    }

    /// True when `cursor` points at a lowercase `Id` followed by `has` (blanket impl start).
    pub fn is_blanket_impl_start(&self) -> bool {
        let Some(Token::Id(name)) = self.peek() else {
            return false;
        };
        let Some(c) = name.chars().next() else {
            return false;
        };
        if !c.is_ascii_lowercase() {
            return false;
        }
        matches!(self.peek_at(1), Some(Token::Has))
    }

    pub fn is_dot_blanket_impl_start(&self) -> bool {
        let Some(Token::Id(name)) = self.peek() else {
            return false;
        };
        let Some(c) = name.chars().next() else {
            return false;
        };
        c.is_ascii_lowercase()
            && matches!(self.peek_at(1), Some(Token::Dot))
            && matches!(self.peek_at(2), Some(Token::Tag(_)))
    }

    pub fn parse_dot_blanket_impl(&mut self, expr_parser: ExprFn) -> Option<ast::BlanketImpl> {
        let type_var = match self.peek() {
            Some(Token::Id(n)) => {
                let name = Intern::new(n.to_string());
                self.advance();
                name
            }
            _ => return None,
        };
        if !self.eat(&Token::Dot) {
            return None;
        }
        let parsed = self.parse_trait_fields(expr_parser)?;
        Some(ast::BlanketImpl {
            trait_name: parsed.trait_name,
            type_var,
            fields: parsed.fields,
        })
    }

    pub fn parse_dot_provided_impl(
        &mut self,
        expr_parser: ExprFn,
    ) -> Option<(Intern<String>, ProvidedTrait)> {
        let type_name = match self.peek()? {
            Token::Tag(name) => {
                let name = self.intern(name);
                self.advance();
                name
            }
            _ => return None,
        };
        if !self.eat(&Token::Dot) {
            return None;
        }
        let parsed = self.parse_trait_fields(expr_parser)?;
        Some((
            type_name,
            ProvidedTrait {
                trait_name: parsed.trait_name,
                trait_name_span: parsed.trait_name_span,
                fields: parsed.fields,
            },
        ))
    }

    /// Parse legacy `x has TraitName(field: expr, ...)` where `x` is a lowercase type variable.
    pub fn parse_blanket_impl(&mut self, expr_parser: ExprFn) -> Option<ast::BlanketImpl> {
        let type_var = match self.peek() {
            Some(Token::Id(n)) => {
                let id = n.to_string();
                let lowercase = id.chars().next().is_some_and(|c| c.is_ascii_lowercase());
                self.advance();
                let name = Intern::new(id);
                if !lowercase {
                    self.error(
                        "blanket impl requires a lowercase type variable; write `x.Trait(...)` instead",
                        self.current_span(),
                    );
                    return None;
                }
                name
            }
            Some(Token::Tag(_)) => {
                self.error(
                    "blanket impl requires a lowercase type variable; write `x.Trait(...)` instead",
                    self.current_span(),
                );
                return None;
            }
            _ => {
                self.error(
                    "expected type variable before `has` in blanket impl",
                    self.current_span(),
                );
                return None;
            }
        };
        if !self.eat(&Token::Has) {
            self.error(
                "expected `has` after type variable in legacy blanket impl",
                self.current_span(),
            );
            return None;
        }
        let parsed = self.parse_trait_fields(expr_parser)?;
        Some(ast::BlanketImpl {
            trait_name: parsed.trait_name,
            type_var,
            fields: parsed.fields,
        })
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
                self.error("expected trait name after `has`", self.current_span());
                return None;
            }
        };

        let mut fields = Vec::new();
        if self.eat(&Token::ParenOpen) {
            if !self.is_at(&Token::ParenClose) {
                loop {
                    let field_name = match self.peek() {
                        Some(Token::Id(n)) => {
                            let id = self.intern(n);
                            self.advance();
                            id
                        }
                        _ => {
                            self.error(
                                "expected field name in trait parameters",
                                self.current_span(),
                            );
                            break;
                        }
                    };
                    if self.expect(&Token::Colon).is_none() {
                        break;
                    }
                    fields.push((field_name, expr_parser(self)));
                    if !self.eat(&Token::Comma) {
                        break;
                    }
                }
            }
            self.expect(&Token::ParenClose)?;
        }
        Some(ParsedTraitFields {
            trait_name,
            trait_name_span,
            fields,
        })
    }

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

            self.skip_indents();
            let checkpoint = self.checkpoint();
            if !self.eat(&Token::And) {
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

    /// Reject legacy `and is [not] Copy` marker syntax after a declaration.
    fn reject_removed_marker_syntax(&mut self) {
        self.skip_indents();
        let checkpoint = self.checkpoint();
        if self.eat(&Token::And) && self.eat(&Token::Is) {
            self.error(
                "marker syntax removed; use `and has Copy(can_copy: False)` to opt out of copyability",
                self.current_span(),
            );
        } else {
            self.rewind(checkpoint);
        }
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
                if seen_default && !matches!(kind, ParameterKind::Default(_)) {
                    self.error(
                        format!(
                            "positional parameter `{}` appears after a default parameter",
                            key.as_str()
                        ),
                        self.current_span(),
                    );
                }
                if matches!(kind, ParameterKind::Default(_)) {
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
    ) -> Option<(Intern<String>, ParameterKind)> {
        // Positional: bare Tag
        if matches!(self.peek(), Some(Token::Tag(_))) {
            let sp = self.parse_type_expr(expr_parser)?;
            let key = Intern::<String>::from_ref(sp.value.surface_mangle_name());
            return Some((
                key,
                ParameterKind::Tagged(Box::new(Spanned {
                    value: sp.value,
                    span_id: sp.span_id,
                })),
            ));
        }

        // Named: id [Tag | id | : TypeExpr]
        let name = match self.peek() {
            Some(Token::Id(n)) => {
                let id = self.intern(n);
                self.advance();
                id
            }
            _ => return None,
        };

        // In declaration (type-level) position, `:` introduces a type default,
        // not a value default. This is how `Box(x, a: LibcAllocator)` works.
        if self.eat(&Token::Colon) {
            let sp = self.parse_type_expr(expr_parser)?;
            return Some((
                name,
                ParameterKind::Default(Box::new(Typed::infer(sp.value.into(), sp.span_id))),
            ));
        }

        // Shared helper supports `id Tag`, `id id` (type variable, e.g.
        // `Range[x] has (start x, end x)`), and bare `id` (generic).
        self.parse_param_after_name(expr_parser, name)
    }

    fn make_variant(
        doc_before: Option<DocComment>,
        shape: Spanned<TypeExpr>,
        doc_after: Option<DocComment>,
    ) -> Variant {
        let doc = DocComment::combine(doc_before, doc_after);
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
                    doc_comment = DocComment::combine(doc_comment, Some(doc));
                    Variant::Local { doc_comment, shape }
                }
            };
        }
    }

    /// Parse a single predicate after `and`, e.g. `< n`, `> 0`, `<= n + 1`.
    fn parse_one_predicate(&mut self) -> PredicateExpr {
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
            Some(Token::EqEq) => {
                self.advance();
                PredicateExpr::Eq(self.parse_predicate_rhs())
            }
            Some(Token::NotEq) => {
                self.advance();
                PredicateExpr::Ne(self.parse_predicate_rhs())
            }
            _ => {
                self.error(
                    "expected predicate after 'and' (e.g. `< n`, `> 0`)",
                    self.current_span(),
                );
                PredicateExpr::Lt(ConstExpr::from(0))
            }
        }
    }

    /// Parse the right-hand side of a predicate: a name or literal.
    fn parse_predicate_rhs(&mut self) -> ConstExpr {
        self.skip_layout();
        let Some((token, _span)) = self.advance() else {
            self.error(
                "expected value after predicate operator",
                self.current_span(),
            );
            return ConstExpr::from(0);
        };
        match token {
            Token::Id(name) => ConstExpr::Var(Intern::from_ref(name)),
            Token::Tag(name) => ConstExpr::Var(Intern::from_ref(name)),
            Token::Int(n) => ConstExpr::from(n as i128),
            _ => {
                self.error(
                    "expected value after predicate operator",
                    self.current_span(),
                );
                ConstExpr::from(0)
            }
        }
    }
}

struct ParsedTraitFields {
    trait_name: Intern<String>,
    trait_name_span: SpanId,
    fields: Vec<(Intern<String>, Typed<Expr>)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::SourceParseExt;
    use ast::ParamConvention;

    #[test]
    fn parses_interface_method_signatures() {
        let source = r#"
Allocator has (
    --- Attempt to allocate.
    allocate(ref self, l Layout) Slice(Byte) or AllocError,
    --- Free a block.
    deallocate(ref self, p Pointer(Byte), l Layout),
)
"#;
        let ast = source.parse_source_full().ast;
        let allocator = ast
            .tags
            .get(&Intern::new("Allocator".to_string()))
            .expect("Allocator tag should exist");

        let DeclareValue::Interface(members) = &allocator.value else {
            panic!(
                "Allocator should be parsed as Interface, got {:?}",
                allocator.value
            );
        };

        assert_eq!(members.len(), 2, "expected 2 interface members");

        // First member: allocate(ref self, l Layout) Slice(Byte) or AllocError
        let allocate = &members[0];
        assert_eq!(allocate.name.as_str(), "allocate");
        assert!(
            allocate.doc_comment.is_some(),
            "allocate should have doc comment"
        );

        // Check params
        assert_eq!(allocate.params.len(), 2);
        assert!(
            allocate
                .conventions
                .contains_key(&Intern::new("self".to_string()))
        );
        assert_eq!(
            allocate.conventions.get(&Intern::new("self".to_string())),
            Some(&ParamConvention::Ref(false)),
        );

        // Check return type: Slice(Byte)
        assert!(
            allocate.return_ty.is_some(),
            "allocate should have return type"
        );
        let rt = &allocate.return_ty.as_ref().unwrap().value;
        match rt {
            TypeExpr::Generic { name, params, .. } => {
                assert_eq!(name.as_str(), "Slice");
                assert_eq!(params.len(), 1);
            }
            other => panic!("allocate return type should be Generic, got {:?}", other),
        }

        // Check error type: AllocError
        assert!(
            allocate.error_ty.is_some(),
            "allocate should have error type"
        );
        let et = &allocate.error_ty.as_ref().unwrap().value;
        match et {
            TypeExpr::Nominal(name, _) => {
                assert_eq!(name.as_str(), "AllocError");
            }
            other => panic!("allocate error type should be AllocError, got {:?}", other),
        }

        // Second member: deallocate(ref self, p Pointer(Byte), l Layout) (no return)
        let deallocate = &members[1];
        assert_eq!(deallocate.name.as_str(), "deallocate");
        assert!(
            deallocate.doc_comment.is_some(),
            "deallocate should have doc comment"
        );
        assert_eq!(deallocate.params.len(), 3);
        assert!(
            deallocate.return_ty.is_none(),
            "deallocate should have no return type"
        );
        assert!(
            deallocate.error_ty.is_none(),
            "deallocate should have no error type"
        );
    }
}
