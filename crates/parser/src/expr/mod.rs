use diagnostic::DiagnosticCode;
use diagnostic::code::{ExpectedSyntaxCode, ParseCode, RemovedSyntaxCode, SyntaxCode};
pub(crate) mod bind;
pub(crate) mod body;
pub(crate) mod body_trivia;
pub(crate) mod control;
pub(crate) mod r#import;
pub(crate) mod literal;
mod when;
mod when_pattern;

// TODO: Split `control.rs` into per-construct files for better co-location of
// parsing logic with its corresponding AST type. Target structure:
//
//   body.rs        ← shared: parse_body_exprs, can_start_expr, parse_pattern, parse_simple_tag
//   for_in.rs      ← pub fn parse() -> Option<ForInLoop>
//   while_loop.rs  ← pub fn parse() -> Option<WhileLoop>
//   loop_expr.rs   ← pub fn parse() -> Option<Loop>  (dispatches to for_in / while_loop)
//   when_expr.rs   ← pub fn parse() -> Option<WhenExpr>
//   return_expr.rs ← pub fn parse() -> Option<Return>
//

use internment::Intern;
use lexer::{Lexer, Token};

use ast::span::{SpanId, SpanTable};
use ast::{Expr, FileAst, SequenceLiteral, Spanned, TypeReference, Typed};

use crate::cursor::{ParseError, TokenCursor};

use crate::unescape::UnescapeExt;
use ast::ModPath;
use ast::{BinOp, Binary, Bind, BindValue, FnCall, FormatPart, FormatString, Range, TagCall};

pub type ExprFn = fn(&mut TokenCursor) -> Typed<Expr>;

pub(crate) fn path_call(path: Spanned<ModPath>) -> Expr {
    Expr::FnCall(FnCall {
        syntax: ast::CallSyntax::Path,
        definition: None,
        path,
        args: None,
    })
}

fn comparison_token_desc(token: Token) -> &'static str {
    match token {
        Token::Eq => "`=`",
        Token::EqEq => "`==`",
        Token::NotEq => "`/=`",
        _ => unreachable!(),
    }
}

impl<'src, 't> TokenCursor<'src, 't> {
    pub fn parse_tokens_with_errors_cancellable(
        tokens: &[(Token<'src>, SpanId)],
        span_table: &mut SpanTable,
        cancel: &'t (dyn Fn() + 't),
    ) -> (FileAst, Vec<ParseError>) {
        let mut cursor = TokenCursor::new(tokens, span_table).with_cancel_check(cancel);
        let ast = cursor.parse_file(|c: &mut TokenCursor| c.parse_expression());
        let errors = std::mem::take(&mut cursor.errors);
        (ast, errors)
    }

    /// Convenience wrapper around [`Self::parse_tokens_with_errors_cancellable`]
    /// that uses a no-op cancellation hook.
    pub fn parse_tokens_with_errors(
        tokens: &[(Token<'src>, SpanId)],
        span_table: &mut SpanTable,
    ) -> (FileAst, Vec<ParseError>) {
        Self::parse_tokens_with_errors_cancellable(tokens, span_table, &|| {})
    }

    /// Parse source text directly, returning a fully-populated [`FileAst`].
    ///
    /// This is the primary entry point used by external consumers. It lexes the
    /// input, filters comments, runs the handwritten parser, and attaches lex
    /// errors as `parse_warnings` on the AST.
    pub fn parse_source(source: &'src str) -> FileAst {
        let mut lexer = Lexer::new(source);
        let mut tokens: Vec<_> = Vec::with_capacity(source.len() / 8);
        tokens.extend(
            std::iter::from_fn(|| lexer.next_raw())
                .filter(|(token, _)| !matches!(token, Token::Comment(_))),
        );
        let lex_errors = std::mem::take(&mut lexer.errors);
        let mut span_table = lexer.take_span_table();
        let (mut ast, hw_parse_errors) = Self::parse_tokens_with_errors(&tokens, &mut span_table);
        ast.blank_lines = lexer.take_blank_lines();
        ast.span_table = span_table;

        // Convert cursor/lex errors to diagnostics and store on the AST.
        ast.parse_warnings.extend(lex_errors);
        for err in &hw_parse_errors {
            ast.parse_warnings
                .push(err.to_warning_diagnostic(&ast.span_table));
        }

        ast
    }

    pub fn parse_paren_args(&mut self) -> Option<Vec<Typed<Expr>>> {
        // Swift guardrail: if there's a raw newline before `(`, don't treat as call args.
        // This prevents `foo\n(1)` from being parsed as `foo(1)` — the `(1)` on the next
        // line is a parenthesized group, not a function call.
        if !self.is_at(&Token::ParenOpen) {
            return None;
        }
        if matches!(self.peek_at(0), Some(Token::Newline)) {
            return None;
        }
        self.advance();
        self.skip_layout();

        let mut args = Vec::new();
        while !self.is_at(&Token::ParenClose) && !self.is_eof() {
            let before = self.pos();
            args.push(self.parse_arg_expr());

            if self.is_at(&Token::ParenClose) {
                break;
            }

            if self.pos() == before {
                break;
            }
            if self.eat_list_separator() || self.previous_token_was_newline_separator() {
                continue;
            }

            // parse-expected-arg-separator
            self.error_here(
                DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::ArgumentSeparator)),
                "expected ',' or newline between function call arguments",
            );
            break;
        }

        self.expect(&Token::ParenClose);
        Some(args)
    }

    /// Parse a call argument expression.
    fn parse_arg_expr(&mut self) -> Typed<Expr> {
        let name_span = self.peek_span().unwrap_or(self.current_span());
        let elem = self.parse_sequence_element();
        if matches!(elem.expr.value, Expr::Ref { .. }) {
            // parse-call-argument-convention
            self.error(
                DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::CallArgumentConvention)),
                "pass the argument without `ref` or `mut`; its parameter declares ownership",
                elem.expr.span_id,
            );
        }
        match elem.name {
            Some(name) => {
                let span = self.merge_span(name_span, elem.span_id);
                Typed::infer(
                    Expr::Bind(Box::new(Bind::new(
                        name,
                        name_span,
                        BindValue::Expr(Box::new(elem.expr)),
                    ))),
                    span,
                )
            }
            None => elem.expr,
        }
    }

    pub fn parse_expression(&mut self) -> Typed<Expr> {
        self.parse_expr_min_prec(0)
    }

    fn parse_expr_min_prec(&mut self, min_prec: u8) -> Typed<Expr> {
        self.parse_expr_min_prec_layout(min_prec, true)
    }

    fn parse_expr_min_prec_layout(&mut self, min_prec: u8, continuation: bool) -> Typed<Expr> {
        let mut lhs = match self.parse_prefix() {
            Some(expr) => expr,
            None => {
                let span = self.current_span();
                // parse-expected-expression
                self.error(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::Expression)),
                    "expected expression",
                    span,
                );
                return Typed::infer(Expr::AnonymousTag(self.intern("__parse_error")), span);
            }
        };

        // Track how many Indent tokens we've consumed via continuation-skipping
        // so we can consume the matching Dedent tokens after the expression ends.
        let mut consumed_indents: usize = 0;

        loop {
            self.advance_push();

            lhs = match self.apply_postfix(lhs) {
                Ok(postfixed) => {
                    self.advance_pop();
                    lhs = postfixed;
                    continue;
                }
                Err(returned) => returned,
            };

            let token = match self.peek() {
                Some(t) => *t,
                None => {
                    self.advance_drop();
                    break;
                }
            };

            if !continuation && matches!(self.raw_peek(), Some(Token::Newline)) {
                self.advance_drop();
                break;
            }

            // Feature 1: Indent-as-continuation.
            // If we see Indent and the token after it continues the current
            // expression, skip the Indent and re-evaluate.
            if matches!(token, Token::Indent) {
                if let Some(next_tok) = self.peek_past_indent() {
                    let continues = self.try_infix_op(next_tok).is_some()
                        || matches!(next_tok, Token::To | Token::Dot);
                    if continuation && continues {
                        self.skip_indents();
                        consumed_indents += 1;
                        self.advance_drop(); // pop the outer loop's push (progress is implicit)
                        continue;
                    }
                }
                self.advance_drop();
                break;
            }

            // Range operator: `to` (precedence 2, left-associative)
            if matches!(token, Token::To) {
                if 2 >= min_prec {
                    self.advance();
                    let lhs_span = lhs.span_id;
                    let rhs = self.parse_expr_min_prec_layout(3, continuation);
                    let rhs_span = rhs.span_id;
                    lhs = Typed::infer(
                        Expr::Range(Range::new(lhs, rhs)),
                        self.merge_span(lhs_span, rhs_span),
                    );
                    self.advance_pop();
                    continue;
                }
                self.advance_drop();
                break;
            }

            if matches!(token, Token::EqEq | Token::NotEq) {
                self.advance();
                let desc = comparison_token_desc(token);
                self.unexpected_token(
                    format!("unexpected token {desc}"),
                    None,
                    self.last_consumed_span(),
                );
                // Consume RHS for error recovery
                self.parse_expr_min_prec(0);
                self.advance_pop();
                continue;
            }

            if let Some((binop, prec)) = self.try_infix_op(&token)
                && prec >= min_prec
            {
                self.advance();
                let lhs_span = lhs.span_id;
                let rhs = self.parse_expr_min_prec_layout(prec + 1, continuation);
                let rhs_span = rhs.span_id;
                lhs = Typed::infer(
                    Expr::Binary(Binary::new(lhs, binop, rhs)),
                    self.merge_span(lhs_span, rhs_span),
                );
                self.advance_pop();
                continue;
            }

            self.advance_drop();
            break;
        }

        // Consume trailing Dedent tokens matching the Indents we consumed.
        for _ in 0..consumed_indents {
            self.eat(&Token::Dedent);
        }

        // Feature 3: Adjacent expression check is done at the top-level parse,
        // not inside parse_expr_min_prec, because block bodies legitimately
        // have multiple consecutive expressions (statements).
        // See parse_top_level_element for the top-level check.

        lhs
    }

    fn parse_prefix(&mut self) -> Option<Typed<Expr>> {
        match self.peek()? {
            // Only `-` is a Pratt prefix (precedence 6); @, ^, * are atoms
            Token::Minus => {
                let (_, start_span) = self.advance()?;
                let inner = self.parse_expr_min_prec(6);
                let inner_span = inner.span_id;
                Some(Typed::infer(
                    Expr::Negate(Box::new(inner)),
                    self.merge_span(start_span, inner_span),
                ))
            }
            _ => {
                if matches!(
                    self.peek(),
                    Some(Token::If | Token::When | Token::For | Token::While | Token::Return)
                ) && let Some(expr) = self.parse_loop_or_control()
                {
                    return Some(expr);
                }
                Some(self.parse_atom())
            }
        }
    }

    fn parse_atom(&mut self) -> Typed<Expr> {
        match self.peek() {
            Some(Token::Domain) => {
                let (_, start) = self.advance().unwrap();
                self.expect(&Token::Of);
                let operand = self.parse_expr_min_prec(6);
                let span = self.merge_span(start, operand.span_id);
                Typed::infer(Expr::DomainOf(Box::new(operand)), span)
            }
            Some(Token::At) => {
                self.parse_prefixed_expression("@", |inner| Expr::TakePtr(Box::new(inner)))
            }
            Some(Token::Pound) => self.parse_compile_time_query(),
            Some(Token::Star) => {
                self.parse_prefixed_expression("*", |inner| Expr::Deref(Box::new(inner)))
            }
            Some(Token::SelfInstance) => self.parse_self_ref(),
            Some(Token::Id(name)) if *name == "eat" => self.parse_removed_consume_arg_expression(),
            Some(Token::Ref) => self.parse_prefixed_expression("ref", |inner| Expr::Ref {
                inner: Box::new(inner),
                mutable: false,
                group: None,
            }),
            Some(Token::Mut) => self.parse_prefixed_expression("mut", |inner| Expr::Ref {
                inner: Box::new(inner),
                mutable: true,
                group: None,
            }),
            Some(Token::Deref) => {
                self.parse_prefixed_expression("deref", |inner| Expr::Deref(Box::new(inner)))
            }
            Some(Token::BracketOpen) => self.parse_bracket_lit(),
            Some(Token::ParenOpen) => self.parse_group(),
            Some(Token::FormatStringDelim) => self.parse_format_string_expr(),
            Some(Token::Id(_)) => {
                if let Some((Token::Id(name), name_span)) = self.peek().zip(self.peek_span())
                    && *name == "_"
                    && self.peek_at(1) == Some(&Token::Eq)
                {
                    let name = self.intern(name);
                    self.advance();
                    self.expect(&Token::Eq);
                    let value = self.parse_expression();
                    let end_span = self.last_consumed_span();
                    let bind = Bind::new(name, name_span, BindValue::Expr(Box::new(value)));
                    return Typed::infer(
                        Expr::Bind(Box::new(bind)),
                        self.merge_span(name_span, end_span),
                    );
                }

                // Fast check: Id followed directly by : or := is always a bind.
                // This avoids calling looks_like_bind (which scans ahead for complex
                // patterns like `id Tag:` and `id(...) type:`) for the simple case.
                // After Phase 3 (body fast path), this branch is rarely hit — it
                // mainly serves expression-context binds and sub-expressions.
                let next_tok = self.peek_at(1);
                if matches!(next_tok, Some(Token::Colon) | Some(Token::ColonEq))
                    || self.looks_like_bind()
                {
                    let checkpoint = self.checkpoint();
                    if let Some(bind) = self.parse_bind(|c: &mut TokenCursor| c.parse_expression())
                    {
                        let start_span = self.span_at(checkpoint);
                        self.consume_trailing_newline();
                        let end_span = self.last_consumed_span();
                        return Typed::infer(
                            Expr::Bind(Box::new(bind)),
                            self.merge_span(start_span, end_span),
                        );
                    }
                    self.rewind(checkpoint);
                }
                self.parse_id_atom()
            }
            Some(token @ (Token::Eq | Token::EqEq | Token::NotEq)) => {
                let token = *token;
                let span = self.peek_span().unwrap_or_else(|| self.current_span());
                self.advance();
                let desc = comparison_token_desc(token);
                self.unexpected_token(format!("unexpected token {desc}"), None, span);
                Typed::infer(Expr::AnonymousTag(self.intern("__parse_error")), span)
            }
            Some(Token::Tag(_)) | Some(Token::SelfTag) => self.parse_tag_atom(),
            _ => {
                if let Some(lit) = self.parse_literal() {
                    let span = lit.span_id;
                    return Typed::infer(Expr::Lit(lit.value), span);
                }
                let span = self.current_span();
                // parse-expected-expression
                self.error(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::Expression)),
                    "expected expression",
                    span,
                );
                // Advance past the unrecognised token so the caller makes progress
                self.advance();
                Typed::infer(Expr::AnonymousTag(self.intern("__parse_error")), span)
            }
        }
    }

    fn parse_removed_consume_arg_expression(&mut self) -> Typed<Expr> {
        let (_, start_span) = self
            .advance()
            .expect("peek confirmed 'eat' token before parse_removed_consume_arg_expression");
        // parse-removed-consume-arg-syntax
        self.error_here(
            DiagnosticCode::Parse(ParseCode::Removed(RemovedSyntaxCode::ConsumeArgumentSyntax)),
            "call-site consume syntax is removed: pass arguments directly",
        );
        // Consume the following expression and lower to the ordinary argument expression.
        let expression = self.parse_expression();
        Typed::infer(expression.value, start_span)
    }

    /// Heuristic: does the current position look like the start of a bind?
    /// True when we see `id :`, `id :=`, `id(...) :`, `id Tag :`, or `id Tag[...] :` patterns.
    pub(crate) fn looks_like_bind(&self) -> bool {
        // id: or id:=  → definitely a bind
        if matches!(
            self.peek_at(1),
            Some(Token::Colon | Token::ColonColon | Token::ColonEq)
        ) {
            return true;
        }
        if self.compound_rebind_at(1).is_some() {
            return true;
        }

        if self.peek_at(1) == Some(&Token::BracketOpen) {
            return self
                .skip_balanced_delimiters(1, Token::BracketOpen, Token::BracketClose)
                .is_some_and(|offset| {
                    matches!(
                        self.peek_at(offset),
                        Some(Token::Colon | Token::ColonEq | Token::Newline | Token::Dedent) | None
                    )
                });
        }

        if self.peek_at(1) == Some(&Token::Is) {
            let mut offset = 2;
            loop {
                match self.peek_at(offset) {
                    Some(Token::Colon | Token::ColonColon | Token::ColonEq) => return true,
                    Some(Token::Newline) | Some(Token::Dedent) | None => return false,
                    _ => offset += 1,
                }
            }
        }

        // id ref or id mut followed by type and `:`/`:=`  → typed ref bind
        // (e.g. `r ref Entity: ref e` or `r mut Entity: mut e`)
        if matches!(self.peek_at(1), Some(Token::Ref) | Some(Token::Mut)) {
            let mut offset = 2; // skip past id and ref/mut
            // Skip over the type expression after ref/mut
            if matches!(self.peek_at(offset), Some(Token::Tag(_))) {
                offset += 1; // skip the Tag
                // Skip optional type-argument parens on the Tag
                if self.peek_at(offset) == Some(&Token::ParenOpen) {
                    offset = match self.skip_balanced_delimiters(
                        offset,
                        Token::ParenOpen,
                        Token::ParenClose,
                    ) {
                        Some(o) => o,
                        None => return false,
                    };
                }
            } else {
                return false;
            }
            return matches!(
                self.peek_at(offset),
                Some(Token::Colon) | Some(Token::ColonEq)
            );
        }

        // id Tag or id Tag[...]
        // → typed bind with value (if `:` follows) or declaration without value (if newline follows)
        // e.g. `val Int: 5` or just `val Int`
        if matches!(self.peek_at(1), Some(Token::Tag(_))) {
            let mut offset = 2; // skip past id and Tag
            // Skip optional type-argument brackets on the Tag.
            if self.peek_at(offset) == Some(&Token::BracketOpen) {
                offset = match self.skip_balanced_delimiters(
                    offset,
                    Token::BracketOpen,
                    Token::BracketClose,
                ) {
                    Some(offset) => offset,
                    None => return false,
                };
            } else if self.peek_at(offset) == Some(&Token::ParenOpen) {
                offset = match self.skip_balanced_delimiters(
                    offset,
                    Token::ParenOpen,
                    Token::ParenClose,
                ) {
                    Some(offset) => offset,
                    None => return false,
                };
            }
            // Accept `Id Tag:` (with value) or `Id Tag` followed by newline/dedent/eof (declaration)
            return matches!(
                self.peek_at(offset),
                Some(Token::Colon)
                    | Some(Token::ColonEq)
                    | Some(Token::Newline)
                    | Some(Token::Dedent)
                    | None
            );
        }

        // id has Trait(...) [and has Trait(...)]:  → bind with inline constraints
        if matches!(self.peek_at(1), Some(Token::Has)) {
            let mut offset = 2; // skip past id and has
            loop {
                // Skip Tag(...) after has
                if matches!(self.peek_at(offset), Some(Token::Tag(_))) {
                    offset += 1;
                    if self.peek_at(offset) == Some(&Token::ParenOpen) {
                        offset = match self.skip_balanced_delimiters(
                            offset,
                            Token::ParenOpen,
                            Token::ParenClose,
                        ) {
                            Some(o) => o,
                            None => return false,
                        };
                    }
                }
                // Check for `and has` after the closing paren
                if matches!(self.peek_at(offset), Some(Token::And))
                    && matches!(self.peek_at(offset + 1), Some(Token::Has))
                {
                    offset += 2;
                    continue;
                }
                break;
            }
            return matches!(
                self.peek_at(offset),
                Some(Token::Colon) | Some(Token::ColonEq)
            );
        }

        // id(...) followed by something bind-like
        if self.peek_at(1) == Some(&Token::ParenOpen) {
            let mut depth = 0;
            let mut offset = 1;
            loop {
                match self.peek_at(offset) {
                    Some(Token::ParenOpen) => {
                        depth += 1;
                        offset += 1;
                    }
                    Some(Token::ParenClose) => {
                        depth -= 1;
                        offset += 1;
                    }
                    Some(_) => {
                        offset += 1;
                    }
                    None => return false,
                }
                if depth == 0 {
                    // After closing paren, a bind has: `:`, `:=`, Tag (return type), or Id (named return)
                    return matches!(
                        self.peek_at(offset),
                        Some(Token::Colon)
                            | Some(Token::ColonEq)
                            | Some(Token::Tag(_))
                            | Some(Token::Id(_))
                    );
                }
            }
        }

        false
    }

    pub(crate) fn compound_rebind_at(&self, offset: usize) -> Option<BinOp> {
        let operator = match self.peek_at(offset)? {
            Token::Plus => BinOp::Add,
            Token::Minus => BinOp::Subtract,
            Token::Star => BinOp::Multiply,
            Token::Slash => BinOp::Divide,
            Token::Percent => BinOp::Modulo,
            Token::Ampersand => BinOp::BitAnd,
            Token::Pipe => BinOp::BitOr,
            Token::Caret => BinOp::BitXor,
            Token::ShiftLeft => BinOp::ShiftLeft,
            Token::ShiftRight => BinOp::ShiftRight,
            _ => return None,
        };
        (self.peek_at(offset + 1) == Some(&Token::Colon)).then_some(operator)
    }

    pub(crate) fn eat_compound_rebind(&mut self) -> Option<BinOp> {
        let operator = self.compound_rebind_at(0)?;
        self.advance();
        self.advance();
        Some(operator)
    }

    fn skip_balanced_delimiters(
        &self,
        mut offset: usize,
        open: Token<'static>,
        close: Token<'static>,
    ) -> Option<usize> {
        let mut depth = 0;
        loop {
            match self.peek_at(offset) {
                Some(tok) if tok == &open => {
                    depth += 1;
                    offset += 1;
                }
                Some(tok) if tok == &close => {
                    depth -= 1;
                    offset += 1;
                }
                Some(_) => {
                    offset += 1;
                }
                None => return None,
            }
            if depth == 0 {
                return Some(offset);
            }
        }
    }

    fn parse_id_atom(&mut self) -> Typed<Expr> {
        // Indexed aggregate update: name.N:: value; indexed projection: name.N
        if matches!(self.peek(), Some(Token::Id(_)))
            && self.peek_at(1) == Some(&Token::Dot)
            && matches!(self.peek_at(2), Some(Token::Int(_)))
        {
            // Deterministic: Id.N is already confirmed by lookahead
            let Some((Token::Id(n), id_span)) = self.advance() else {
                unreachable!()
            };
            let name = self.intern(n);
            self.eat(&Token::Dot);
            let Some((Token::Int(idx), _idx_span)) = self.advance() else {
                unreachable!()
            };

            let operator = if self.eat(&Token::ColonColon) {
                Some(None)
            } else {
                self.eat_compound_rebind().map(Some)
            };
            if let Some(operator) = operator {
                let value = self.parse_expression();
                let end_span = self.last_consumed_span();
                let base = Typed::infer(
                    path_call(Spanned::new(ModPath::new(name, vec![]), id_span)),
                    id_span,
                );
                return Typed::infer(
                    Expr::AggregateSet {
                        base: Box::new(base),
                        index: ast::integer::to_usize(idx).unwrap_or(usize::MAX),
                        value: Box::new(value),
                        operator,
                    },
                    self.merge_span(id_span, end_span),
                );
            }
            // Indexed projection — return as expression
            let base = Typed::infer(
                path_call(Spanned::new(ModPath::new(name, vec![]), id_span)),
                id_span,
            );
            let end_span = self.last_consumed_span();
            return Typed::infer(
                Expr::AggregateGet {
                    base: Box::new(base),
                    index: ast::integer::to_usize(idx).unwrap_or(usize::MAX),
                },
                self.merge_span(id_span, end_span),
            );
        }

        if matches!(self.peek(), Some(Token::Id(_)))
            && self.peek_at(1) == Some(&Token::Dot)
            && matches!(self.peek_at(2), Some(Token::Id(_)))
            && (self.peek_at(3) == Some(&Token::ColonColon) || self.compound_rebind_at(3).is_some())
        {
            let Some((Token::Id(base_name), base_span)) = self.advance() else {
                unreachable!()
            };
            self.advance();
            let Some((Token::Id(field_name), _)) = self.advance() else {
                unreachable!()
            };
            let operator = if self.eat(&Token::ColonColon) {
                None
            } else {
                Some(
                    self.eat_compound_rebind()
                        .expect("compound rebind lookahead"),
                )
            };
            let value = self.parse_expression();
            let end_span = self.last_consumed_span();
            let base = Typed::infer(
                path_call(Spanned::new(
                    ModPath::new(self.intern(base_name), vec![]),
                    base_span,
                )),
                base_span,
            );
            return Typed::infer(
                Expr::RecordSet {
                    base: Box::new(base),
                    field: self.intern(field_name),
                    value: Box::new(value),
                    operator,
                },
                self.merge_span(base_span, end_span),
            );
        }

        // ── FnCall: name, name(args), name.path(args) ──
        if let Some(path) = self.parse_path() {
            let path_span = path.span_id;
            let args = self.parse_paren_args();
            let end_span = self.last_consumed_span();
            self.consume_trailing_newline();
            if args.is_some() || path.value.segments.is_empty() {
                return Typed::infer(
                    Expr::FnCall(FnCall {
                        syntax: ast::CallSyntax::Path,
                        definition: None,
                        path,
                        args,
                    }),
                    self.merge_span(path_span, end_span),
                );
            }
            let mut expr = Typed::infer(
                path_call(Spanned::new(
                    ModPath::new(path.value.root, vec![]),
                    path.value.root_span,
                )),
                path.value.root_span,
            );
            for (field, member_span) in path
                .value
                .segments
                .iter()
                .copied()
                .zip(path.value.segment_spans.iter().copied())
            {
                let field_span = expr.span_id;
                expr = Typed::infer(
                    Expr::RecordGet {
                        base: Box::new(expr),
                        field,
                    },
                    self.merge_span(field_span, member_span),
                );
            }
            return expr;
        }

        let span = self.current_span();
        // parse-expected-identifier
        self.error(
            DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::Identifier)),
            "expected identifier",
            span,
        );
        Typed::infer(Expr::AnonymousTag(self.intern("__parse_error")), span)
    }

    // ─── Tag-based Atoms: TagCall, AnonymousTag, Tag-rooted FnCall ────────────────

    fn parse_tag_atom(&mut self) -> Typed<Expr> {
        self.skip_newlines();
        // Single-pass lookahead avoids speculative parsing with rewind.
        // The common case (bare Tag or Tag(args)) skips all path parsing.
        if matches!(self.peek(), Some(Token::Tag(_) | Token::SelfTag))
            && matches!(self.peek_at(1), Some(Token::Dot))
        {
            // Qualified variant: Tag.Tag[(args)]
            if matches!(self.peek_at(2), Some(Token::Tag(_)))
                && let Some(path) = self.parse_tag_rooted_path(true)
            {
                let variant_name = *path.segments.last().unwrap_or(&path.root);
                let start_span = path.span_id;
                let args = if self.is_at(&Token::ParenOpen) {
                    self.parse_paren_args().unwrap_or_default()
                } else {
                    Vec::new()
                };
                let end_span = self.last_consumed_span();
                let span = self.merge_span(start_span, end_span);
                return Typed::infer(
                    Expr::TagCall(TagCall {
                        name: variant_name,
                        qual_path: Some(path),
                        args,
                    }),
                    span,
                );
            }

            // Tag-rooted method call: Tag.id[.id...][(args)]
            if matches!(self.peek_at(2), Some(Token::Id(_)))
                && let Some(path) = self.parse_tag_rooted_path(false)
            {
                let path_span = path.span_id;
                let args = self.parse_paren_args();
                self.consume_trailing_newline();
                let end_span = self.last_consumed_span();
                return Typed::infer(
                    Expr::FnCall(FnCall {
                        syntax: ast::CallSyntax::Path,
                        definition: None,
                        path,
                        args,
                    }),
                    self.merge_span(path_span, end_span),
                );
            }
        }

        // Simple Tag(args) → TagCall, or bare Tag → AnonymousTag
        if let Some((token, tag_span)) = self.advance()
            && let Some(name_interned) = match token {
                Token::Tag(name) => Some(self.intern(name)),
                Token::SelfTag => Some(self.intern("Self")),
                _ => None,
            }
        {
            if self.is_at(&Token::ParenOpen) {
                let args = self.parse_paren_args().unwrap_or_default();
                let end_span = self.last_consumed_span();
                let span = self.merge_span(tag_span, end_span);
                return Typed::infer(
                    Expr::TagCall(TagCall {
                        name: name_interned,
                        qual_path: None,
                        args,
                    }),
                    span,
                );
            }

            return Typed::infer(Expr::AnonymousTag(name_interned), tag_span);
        }

        let span = self.current_span();
        // parse-expected-tag
        self.error(
            DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::Tag)),
            "expected tag",
            span,
        );
        Typed::infer(Expr::AnonymousTag(self.intern("__parse_error")), span)
    }

    fn parse_sequence_element(&mut self) -> ast::SequenceElement {
        self.parse_sequence_element_with(|cursor| cursor.parse_expression())
    }

    fn parse_sequence_element_with(&mut self, expr_parser: ExprFn) -> ast::SequenceElement {
        let name = if matches!(self.peek(), Some(Token::Id(_)))
            && self.peek_at(1) == Some(&Token::Colon)
        {
            let name = self.parse_name();
            self.expect(&Token::Colon);
            name
        } else {
            None
        };
        ast::SequenceElement {
            name,
            expr: expr_parser(self),
        }
    }

    pub(crate) fn parse_sequence_value(&mut self) -> Typed<Expr> {
        let first = self.parse_sequence_element();
        if first.name.is_none() && self.peek_at(0) != Some(&Token::Comma) {
            return first.expr;
        }
        let start = first.span_id;
        let mut elems = vec![first];
        while self.peek_at(0) == Some(&Token::Comma) {
            self.advance();
            self.skip_layout();
            elems.push(self.parse_sequence_element());
        }
        self.sequence_value(elems, start)
    }

    pub(crate) fn sequence_value(
        &mut self,
        elems: Vec<ast::SequenceElement>,
        start: SpanId,
    ) -> Typed<Expr> {
        let mut names = std::collections::HashSet::new();
        for elem in &elems {
            if let Some(name) = elem.name
                && !names.insert(name)
            {
                // parse-duplicate-sequence-name
                self.error(
                    DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::DuplicateSequenceName)),
                    format!("duplicate sequence name `{name}`"),
                    elem.span_id,
                );
            }
        }
        Typed::infer(
            Expr::SequenceLiteral(SequenceLiteral::Elements(elems)),
            self.merge_span(start, self.last_consumed_span()),
        )
    }

    pub(crate) fn parse_bracket_lit(&mut self) -> Typed<Expr> {
        let start = self.current_span();
        self.advance();
        self.skip_layout();
        let mut elems = Vec::new();
        while !self.is_at(&Token::BracketClose) && self.peek().is_some() {
            let first = self.parse_sequence_element();
            if self.eat(&Token::ColonSemi) {
                if !elems.is_empty() || first.name.is_some() {
                    // parse-sequence-repeat-elements
                    self.error(
                        DiagnosticCode::Parse(ParseCode::Syntax(
                            SyntaxCode::SequenceRepeatElements,
                        )),
                        "a repeated sequence requires one unnamed initializer",
                        first.span_id,
                    );
                }
                let count = self.parse_expression();
                self.skip_layout();
                self.expect(&Token::BracketClose);
                return Typed::infer(
                    Expr::SequenceLiteral(SequenceLiteral::Repeat {
                        seed: Box::new(first.expr),
                        count: Box::new(count),
                    }),
                    self.merge_span(start, self.last_consumed_span()),
                );
            }
            elems.push(first);
            if self.is_at(&Token::BracketClose) {
                break;
            }
            if self.eat_list_separator() || self.previous_token_was_newline_separator() {
                self.skip_layout();
                continue;
            }
            // parse-expected-list-separator
            self.error_here(
                DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::ListSeparator)),
                "expected ',' or newline between sequence elements",
            );
            break;
        }
        self.skip_layout();
        self.expect(&Token::BracketClose);
        self.sequence_value(elems, start)
    }

    fn parse_group(&mut self) -> Typed<Expr> {
        let start = self.current_span();
        self.advance();
        self.skip_layout();
        if self.eat(&Token::ParenClose) {
            // parse-empty-parens-removed
            self.error(
                DiagnosticCode::Parse(ParseCode::Removed(RemovedSyntaxCode::EmptyParens)),
                "empty parentheses are not a value; use an empty nominal value or a bare return",
                start,
            );
            return Typed::infer(Expr::Invalid(start), start);
        }
        let named =
            matches!(self.peek(), Some(Token::Id(_))) && self.peek_at(1) == Some(&Token::Colon);
        let mut first = if named {
            self.parse_sequence_value()
        } else {
            self.parse_expression()
        };
        if named || self.is_at(&Token::ColonSemi) || self.is_at(&Token::Comma) {
            // parse-parenthesized-sequence
            self.error(
                DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::ParenthesizedSequence)),
                "use brackets to construct a sequence",
                start,
            );
            while self.peek().is_some() && !self.is_at(&Token::ParenClose) {
                self.advance();
            }
            self.expect(&Token::ParenClose);
            return Typed::infer(Expr::Invalid(start), start);
        }
        self.skip_layout();
        self.expect(&Token::ParenClose);
        first.span_id = self.merge_span(start, self.last_consumed_span());
        first
    }

    fn parse_compile_time_query(&mut self) -> Typed<Expr> {
        let (_, start_span) = self
            .advance()
            .expect("peek confirmed '#' token before compile-time query");
        let kind = match self.peek() {
            Some(Token::Id("todo")) => {
                self.advance();
                let span = self.merge_span(start_span, self.last_consumed_span());
                self.todo_holes.push(self.span_table().get(span));
                return Typed::infer(Expr::Todo, span);
            }
            Some(Token::Id("emit")) => {
                self.advance();
                self.expect(&Token::ParenOpen);
                self.skip_layout();
                let code = self.parse_expression();
                self.expect(&Token::Comma);
                self.skip_layout();
                let arguments = self.parse_expression();
                self.skip_layout();
                self.expect(&Token::ParenClose);
                return Typed::infer(
                    Expr::Emit {
                        code: Box::new(code),
                        arguments: Box::new(arguments),
                    },
                    self.merge_span(start_span, self.last_consumed_span()),
                );
            }
            Some(Token::Id("target")) => {
                self.advance();
                return Typed::infer(Expr::Target, start_span);
            }
            _ => self.parse_target_query_kind(),
        };
        self.expect(&Token::ParenOpen);
        let operand = self
            .parse_type_reference()
            .unwrap_or_else(|| TypeReference::Nominal(self.intern("__parse_error")));
        self.expect(&Token::ParenClose);
        let end_span = self.last_consumed_span();
        Typed::infer(
            Expr::TargetQuery { kind, operand },
            self.merge_span(start_span, end_span),
        )
    }

    pub(crate) fn parse_type_reference(&mut self) -> Option<TypeReference> {
        if self.eat(&Token::At) {
            return self
                .parse_type_reference()
                .map(|inner| TypeReference::Pointer(Box::new(inner)));
        }
        if self.eat(&Token::ParenOpen) {
            self.expect(&Token::ParenClose);
            return Some(TypeReference::Unit);
        }
        let name = match self.advance()? {
            (Token::Id(name) | Token::Tag(name), _) => self.intern(name),
            (Token::SelfTag, _) => Intern::from_ref("Self"),
            _ => return None,
        };
        if !self.eat(&Token::ParenOpen) {
            return Some(TypeReference::Nominal(name));
        }
        let mut arguments = Vec::new();
        while !self.is_at(&Token::ParenClose) {
            arguments.push(self.parse_type_reference()?);
            if !self.eat_list_separator() {
                break;
            }
        }
        self.expect(&Token::ParenClose);
        Some(TypeReference::Application { name, arguments })
    }

    fn parse_prefixed_expression(
        &mut self,
        prefix: &str,
        build: impl FnOnce(Typed<Expr>) -> Expr,
    ) -> Typed<Expr> {
        let (_, start_span) = self
            .advance()
            .unwrap_or_else(|| panic!("peek confirmed '{prefix}' token before parsing"));
        let inner = self.parse_expression();
        let end_span = inner.span_id;
        Typed::infer(build(inner), self.merge_span(start_span, end_span))
    }

    fn parse_self_ref(&mut self) -> Typed<Expr> {
        let (_, span) = self
            .advance()
            .expect("peek confirmed 'self' token before parse_self_ref");
        // Return bare SelfRef — apply_postfix handles `.field` → RecordGet.
        Typed::infer(Expr::SelfRef, span)
    }

    fn try_infix_op(&self, token: &Token) -> Option<(BinOp, u8)> {
        match token {
            // Precedence 1-2: logical
            Token::Or => Some((BinOp::LogicalOr, 1)),
            Token::And => Some((BinOp::LogicalAnd, 2)),
            // Precedence 3: bitwise
            Token::Eq => Some((BinOp::Equal, 3)),
            Token::Less => Some((BinOp::Less, 3)),
            Token::LessEq => Some((BinOp::LessOrEqual, 3)),
            Token::Greater => Some((BinOp::Greater, 3)),
            Token::GreaterEq => Some((BinOp::GreaterOrEqual, 3)),
            Token::Ampersand => Some((BinOp::BitAnd, 3)),
            Token::Pipe => Some((BinOp::BitOr, 3)),
            Token::Caret => Some((BinOp::BitXor, 3)),
            Token::ShiftLeft => Some((BinOp::ShiftLeft, 3)),
            Token::ShiftRight => Some((BinOp::ShiftRight, 3)),
            // Precedence 4: arithmetic
            Token::Plus => Some((BinOp::Add, 4)),
            Token::Minus => Some((BinOp::Subtract, 4)),
            Token::Star => Some((BinOp::Multiply, 4)),
            Token::Slash => Some((BinOp::Divide, 4)),
            Token::Percent => Some((BinOp::Modulo, 4)),
            _ => None,
        }
    }

    #[allow(clippy::result_large_err)]
    fn apply_postfix(&mut self, lhs: Typed<Expr>) -> Result<Typed<Expr>, Typed<Expr>> {
        if self.raw_peek() == Some(&Token::BracketOpen)
            && !self.previous_token_was_newline_separator()
        {
            // parse-index-requires-dot
            self.error_here(
                DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::IndexRequiresDot)),
                "element access requires a dot: write `value.[index]`",
            );
            return Err(lhs);
        }
        if matches!(self.raw_peek(), Some(Token::Newline)) && self.is_at(&Token::Dot) {
            self.skip_newlines();
        }

        // .N → AggregateGet
        if self.is_at(&Token::Dot)
            && let Some(&Token::Int(n)) = self.peek_at(1)
        {
            self.advance(); // Dot
            self.advance(); // Int
            let end_span = self.last_consumed_span();
            let lhs_span = lhs.span_id;
            return Ok(Typed::infer(
                Expr::AggregateGet {
                    base: Box::new(lhs),
                    index: ast::integer::to_usize(n).unwrap_or(usize::MAX),
                },
                self.merge_span(lhs_span, end_span),
            ));
        }

        if self.is_at(&Token::Dot) && self.peek_at(1) == Some(&Token::BracketOpen) {
            self.advance();
            self.advance();
            self.skip_layout();
            let index = self.parse_expression();
            self.skip_layout();
            if self.eat(&Token::ColonSemi) {
                self.skip_layout();
                let length = self.parse_expression();
                self.skip_layout();
                self.expect(&Token::BracketClose);
                let span = self.merge_span(lhs.span_id, self.last_consumed_span());
                return Ok(Typed::infer(
                    Expr::SliceGet {
                        buf: Box::new(lhs),
                        start: Box::new(index),
                        length: Box::new(length),
                    },
                    span,
                ));
            }
            self.expect(&Token::BracketClose);
            let operator = if self.eat(&Token::ColonColon) {
                Some(None)
            } else {
                self.eat_compound_rebind().map(Some)
            };
            if let Some(operator) = operator {
                let value = self.parse_expression();
                let span = self.merge_span(lhs.span_id, self.last_consumed_span());
                return Ok(Typed::infer(
                    Expr::BufSet {
                        buf: Box::new(lhs),
                        index: Box::new(index),
                        value: Box::new(value),
                        operator,
                    },
                    span,
                ));
            }
            let end_span = self.last_consumed_span();
            let lhs_span = lhs.span_id;
            return Ok(Typed::infer(
                Expr::BufGet {
                    buf: Box::new(lhs),
                    index: Box::new(index),
                },
                self.merge_span(lhs_span, end_span),
            ));
        }

        // .field → RecordGet or RecordSet (if followed by ':')
        if self.is_at(&Token::Dot)
            && matches!(self.peek_at(1), Some(Token::Id(_)))
            && self.peek_at(2) != Some(&Token::ParenOpen)
        {
            self.advance(); // Dot
            if let Some((Token::Id(name), field_span)) = self.advance() {
                let field = self.intern(name);
                let lhs_span = lhs.span_id;

                let operator = if self.eat(&Token::ColonColon) {
                    Some(None)
                } else {
                    self.eat_compound_rebind().map(Some)
                };
                if let Some(operator) = operator {
                    let value = self.parse_expression();
                    let end_span = self.last_consumed_span();
                    return Ok(Typed::infer(
                        Expr::RecordSet {
                            base: Box::new(lhs),
                            field,
                            value: Box::new(value),
                            operator,
                        },
                        self.merge_span(lhs_span, end_span),
                    ));
                }

                return Ok(Typed::infer(
                    Expr::RecordGet {
                        base: Box::new(lhs),
                        field,
                    },
                    self.merge_span(lhs_span, field_span),
                ));
            }
            return Err(lhs);
        }

        // .method(args) → Method call chaining
        if self.is_at(&Token::Dot) && matches!(self.peek_at(1), Some(Token::Id(_))) {
            self.advance(); // Dot
            let (method_name, method_span) = match self.advance() {
                Some((Token::Id(name), span)) => (self.intern(name), span),
                _ => return Err(lhs),
            };
            let lhs_span = lhs.span_id;
            let args = self.parse_paren_args();
            let mut all_args = vec![lhs];
            if let Some(mut call_args) = args {
                all_args.append(&mut call_args);
            }
            let end_span = self.last_consumed_span();
            return Ok(Typed::infer(
                Expr::FnCall(FnCall {
                    syntax: ast::CallSyntax::Receiver,
                    definition: None,
                    path: Spanned::new(ModPath::new(method_name, vec![]), method_span),
                    args: Some(all_args),
                }),
                self.merge_span(lhs_span, end_span),
            ));
        }

        // as Type → Cast
        if self.is_at(&Token::As) {
            self.advance(); // As
            match self.parse_type_reference() {
                Some(ty) => {
                    let end_span = self.last_consumed_span();
                    let lhs_span = lhs.span_id;
                    return Ok(Typed::infer(
                        Expr::Cast {
                            expr: Box::new(lhs),
                            ty,
                        },
                        self.merge_span(lhs_span, end_span),
                    ));
                }
                None => {
                    // parse-expected-type-name
                    self.error_here(
                        DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::TypeName)),
                        "expected type name after 'as'",
                    );
                }
            }
        }

        Err(lhs)
    }

    fn parse_loop_or_control(&mut self) -> Option<Typed<Expr>> {
        let start_span = self.current_span();

        if self.is_at(&Token::If) {
            // parse-removed-if
            self.error(
                DiagnosticCode::Parse(ParseCode::Removed(RemovedSyntaxCode::If)),
                "`if` has been removed; use 'when condition then'",
                start_span,
            );
            self.advance();
            return Some(Typed::infer(Expr::Invalid(start_span), start_span));
        }
        if let Some(ret) = self.parse_return() {
            let span = self.merge_span(start_span, self.last_consumed_span());
            return Some(Typed::infer(Expr::Return(ret), span));
        }

        if let Some(when_expr) = self.parse_when_expr(|c: &mut TokenCursor| c.parse_expression()) {
            self.consume_trailing_newline();
            let end_span = self.last_consumed_span();
            return Some(Typed::infer(
                Expr::When(when_expr),
                self.merge_span(start_span, end_span),
            ));
        }

        if let Some(loop_expr) = self.parse_loop_expr(|c: &mut TokenCursor| c.parse_expression()) {
            let end_span = self.last_consumed_span();
            return Some(Typed::infer(
                Expr::Loop(loop_expr),
                self.merge_span(start_span, end_span),
            ));
        }

        None
    }

    fn parse_format_string_expr(&mut self) -> Typed<Expr> {
        let start_span = self.peek_span().unwrap_or_else(|| self.current_span());
        self.advance(); // consume opening "

        let mut parts = Vec::new();

        loop {
            self.advance_push();
            match self.peek() {
                Some(Token::FormatStringText(s)) => {
                    let span = self.peek_span().unwrap_or_else(|| self.current_span());
                    parts.push(FormatPart::Text(s.unescape(), span));
                    self.advance();
                    self.advance_pop();
                }
                Some(Token::FormatInterpStart) => {
                    self.advance();
                    let expr = self.parse_expression();
                    let expr_span = expr.span_id;
                    // Clone expr (via .value access) — FormatPart stores Typed<Expr>
                    parts.push(FormatPart::Expr(Box::new(expr), expr_span));
                    self.expect(&Token::FormatInterpEnd);
                    self.advance_pop();
                }
                Some(Token::FormatStringDelim) => {
                    self.advance(); // consume closing "
                    self.advance_drop();
                    let end_span = self.last_consumed_span();
                    let fmt_span = self.merge_span(start_span, end_span);
                    if parts
                        .iter()
                        .any(|part| matches!(part, FormatPart::Expr(..)))
                    {
                        // TODO: restore interpolation through library-defined formatting without compiler knowledge of user types.
                        // parse-template-strings-not-supported
                        self.error(
                            DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::TemplateStringsNotSupported)),
                            "template strings are not supported yet; use byte literals and explicit library calls",
                            fmt_span,
                        );
                        return Typed::infer(Expr::FormatString(FormatString { parts }), fmt_span);
                    }
                    let text = parts
                        .into_iter()
                        .filter_map(|part| match part {
                            FormatPart::Text(text, _) => Some(text),
                            FormatPart::Expr(..) => None,
                        })
                        .collect::<String>();
                    return Typed::infer(Expr::Lit(ast::Literal::String(text)), fmt_span);
                }
                Some(Token::UnterminatedFormatString) | None => {
                    self.advance_drop();
                    let span = self.current_span();
                    // parse-unterminated-format-string
                    self.error(
                        DiagnosticCode::Parse(ParseCode::Syntax(
                            SyntaxCode::UnterminatedFormatString,
                        )),
                        "unterminated format string",
                        span,
                    );
                    return Typed::infer(
                        Expr::FormatString(FormatString { parts }),
                        self.merge_span(start_span, span),
                    );
                }
                _ => {
                    let span = self.current_span();
                    // parse-unexpected-token-in-format-string
                    self.error(
                        DiagnosticCode::Parse(ParseCode::Syntax(
                            SyntaxCode::UnexpectedTokenInFormatString,
                        )),
                        "unexpected token in format string",
                        span,
                    );
                    self.advance();
                    self.advance_pop();
                }
            }
        }
    }
}
#[cfg(test)]
#[path = "../../tests/mod_tests.rs"]
mod tests;
