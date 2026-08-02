pub(crate) mod bind;
pub(crate) mod body;
pub(crate) mod body_trivia;
pub(crate) mod control;
pub(crate) mod r#import;
pub(crate) mod literal;

// TODO: Split `control.rs` into per-construct files for better co-location of
// parsing logic with its corresponding AST type. Target structure:
//
//   body.rs        ← shared: parse_body_exprs, can_start_expr, parse_pattern, parse_simple_tag
//   for_in.rs      ← pub fn parse() -> Option<ForInLoop>
//   while_loop.rs  ← pub fn parse() -> Option<WhileLoop>
//   loop_expr.rs   ← pub fn parse() -> Option<Loop>  (dispatches to for_in / while_loop)
//   if_expr.rs     ← pub fn parse() -> Option<IfExpr>
//   when_expr.rs   ← pub fn parse() -> Option<WhenExpr>
//   return_expr.rs ← pub fn parse() -> Option<Return>
//

use diagnostic::{Category, Diagnostic};
use internment::Intern;
use lexer::{Lexer, Token};

use ast::span::{SpanId, SpanTable};
use ast::{Expr, FileAst, Spanned, Typed};

use crate::cursor::{ParseError, TokenCursor};

use crate::unescape::UnescapeExt;
use ast::ModPath;
use ast::{
    AsmExpr, BinOp, Binary, Bind, BindValue, FnCall, FormatPart, FormatString, Range, TagCall,
};

pub type ExprFn = fn(&mut TokenCursor) -> Typed<Expr>;

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
        tokens.extend(lexer.by_ref());
        let lex_errors = std::mem::take(&mut lexer.errors);
        let mut span_table = lexer.take_span_table();
        let (mut ast, hw_parse_errors) = Self::parse_tokens_with_errors(&tokens, &mut span_table);
        ast.span_table = span_table;

        // Convert cursor/lex errors to diagnostics and store on the AST.
        ast.parse_warnings.extend(lex_errors);
        for err in &hw_parse_errors {
            if let Some(hint) = err.message().strip_prefix("__gin_hint__:") {
                match hint.trim() {
                    "unnecessary-comma" => {
                        ast.parse_warnings.push(
                            Diagnostic::new("parse-unnecessary-comma", "unnecessary comma")
                                .with_category(Category::Help)
                                .with_help(
                                    "this comma is not needed since Gin uses newlines \
                                 to separate items; you can remove it",
                                )
                                .at_span_id(err.span(), &ast.span_table),
                        );
                    }
                    hint => {
                        ast.parse_warnings.push(
                            Diagnostic::new("parse-indented-return", hint)
                                .with_category(Category::Help)
                                .at_span_id(err.span(), &ast.span_table),
                        );
                    }
                }
            } else {
                ast.parse_warnings.push(
                    Diagnostic::new(err.code(), err.message().to_string())
                        .at_span_id(err.span(), &ast.span_table),
                );
            }
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
        while !self.is_at(&Token::ParenClose) {
            args.push(self.parse_arg_expr());

            if self.is_at(&Token::ParenClose) {
                break;
            }

            if self.eat_list_separator() || self.previous_token_was_newline_separator() {
                continue;
            }

            self.error(
                "parse-expected-arg-separator",
                "expected ',' or newline between function call arguments",
                self.current_span(),
            );
            break;
        }

        self.expect(&Token::ParenClose);
        Some(args)
    }

    /// Parse an argument expression, recognizing `eat expr` (consume) prefix.
    fn parse_arg_expr(&mut self) -> Typed<Expr> {
        if self.eat(&Token::Eat) {
            let inner = self.parse_expression();
            let span_id = inner.span_id;
            Typed::infer(Expr::ConsumeArg(Box::new(inner)), span_id)
        } else if matches!(self.peek(), Some(Token::Id(_)))
            && self.peek_at(1) == Some(&Token::Eq)
        {
            let (name, name_span) = match self.advance() {
                Some((Token::Id(name), span)) => (self.intern(name), span),
                _ => unreachable!(),
            };
            self.advance();
            let rhs = self.parse_expression();
            let end_span = self.last_consumed_span();
            Typed::infer(
                Expr::Bind(Box::new(Bind::new(
                    name,
                    name_span,
                    BindValue::Expr(Box::new(rhs)),
                ))),
                self.merge_span(name_span, end_span),
            )
        } else {
            self.parse_expression()
        }
    }

    pub fn parse_expression(&mut self) -> Typed<Expr> {
        self.parse_expr_min_prec(0)
    }

    fn parse_expr_min_prec(&mut self, min_prec: u8) -> Typed<Expr> {
        let mut lhs = match self.parse_prefix() {
            Some(expr) => expr,
            None => {
                let span = self.current_span();
                self.error("parse-expected-expression", "expected expression", span);
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

            // Feature 1: Indent-as-continuation.
            // If we see Indent and the token after it is an infix operator,
            // skip the Indent (treat it as expression continuation) and
            // re-evaluate with the operator.
            if matches!(token, Token::Indent) {
                if let Some(next_tok) = self.peek_past_indent() {
                    let is_infix = self.try_infix_op(next_tok).is_some()
                        || self.try_comparison_op(next_tok).is_some()
                        || matches!(next_tok, Token::Infer);
                    if is_infix {
                        self.skip_indents();
                        consumed_indents += 1;
                        self.advance_drop(); // pop the outer loop's push (progress is implicit)
                        continue;
                    }
                }
                self.advance_drop();
                break;
            }

            // Range operator: `...` (precedence 2, left-associative)
            if matches!(token, Token::Infer) {
                if 2 >= min_prec {
                    self.advance();
                    let lhs_span = lhs.span_id;
                    let rhs = self.parse_expr_min_prec(3);
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

            if matches!(token, Token::Eq | Token::EqEq | Token::NotEq) {
                self.advance();
                let desc = match token {
                    Token::Eq => "`=`",
                    Token::EqEq => "`==`",
                    Token::NotEq => "`/=`",
                    _ => unreachable!(),
                };
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

            // Comparison operators: desugar to function calls (`a < b` → `lt(a, b)`)
            if let Some((fn_name, prec)) = self.try_comparison_op(&token)
                && prec >= min_prec
            {
                self.advance();
                let lhs_span = lhs.span_id;
                let rhs = self.parse_expr_min_prec(prec + 1);
                let rhs_span = rhs.span_id;
                let path = ModPath::new(self.intern(fn_name), vec![]);
                let fn_call = FnCall {
                    path: ast::Spanned::new(path, lhs_span),
                    args: Some(vec![lhs, rhs]),
                };
                lhs = Typed::infer(Expr::FnCall(fn_call), self.merge_span(lhs_span, rhs_span));
                self.advance_pop();
                continue;
            }

            if let Some((binop, prec)) = self.try_infix_op(&token)
                && prec >= min_prec
            {
                self.advance();
                let lhs_span = lhs.span_id;
                let rhs = self.parse_expr_min_prec(prec + 1);
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
                    Some(Token::If) | Some(Token::When) | Some(Token::For) | Some(Token::While)
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
            Some(Token::At) => self.parse_take_ptr(),
            Some(Token::Star) => self.parse_deref(),
            Some(Token::SelfInstance) => self.parse_self_ref(),
            Some(Token::Ref) => self.parse_ref_take(),
            Some(Token::Mut) => self.parse_mut_take(),
            Some(Token::Deref) => self.parse_deref_expr(),
            Some(Token::Eat) => self.parse_eat_expr(),
            Some(Token::BracketOpen) => self.parse_list_lit(),
            Some(Token::ParenOpen) => self.parse_tuple_lit_or_alloc_or_group(),
            Some(Token::FormatStringDelim) => self.parse_format_string_expr(),
            Some(Token::Asm) => self.parse_asm_expr(),
            Some(Token::Id(_)) => {
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
            Some(Token::Eq) | Some(Token::EqEq) | Some(Token::NotEq) => {
                let token = self.peek().copied();
                let span = self.peek_span().unwrap_or_else(|| self.current_span());
                self.advance();
                let desc = match token {
                    Some(Token::Eq) => "`=`",
                    Some(Token::EqEq) => "`==`",
                    Some(Token::NotEq) => "`/=`",
                    _ => unreachable!(),
                };
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
                self.error("parse-expected-expression", "expected expression", span);
                // Advance past the unrecognised token so the caller makes progress
                self.advance();
                Typed::infer(Expr::AnonymousTag(self.intern("__parse_error")), span)
            }
        }
    }

    /// Heuristic: does the current position look like the start of a bind?
    /// True when we see `id :`, `id :=`, `id(...) :`, `id Tag :`, or `id Tag[...] :` patterns.
    pub(crate) fn looks_like_bind(&self) -> bool {
        // id: or id:=  → definitely a bind
        if matches!(self.peek_at(1), Some(Token::Colon) | Some(Token::ColonEq)) {
            return true;
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
        // BufSet: name.(index): value or BufGet: name.(index)
        if matches!(self.peek(), Some(Token::Id(_)))
            && self.peek_at(1) == Some(&Token::Dot)
            && self.peek_at(2) == Some(&Token::ParenOpen)
        {
            // Deterministic: Id.( is already confirmed by lookahead
            let Some((Token::Id(n), id_span)) = self.advance() else {
                unreachable!()
            };
            let name = self.intern(n);
            self.eat(&Token::Dot);
            self.eat(&Token::ParenOpen);
            let index = self.parse_expression();
            self.expect(&Token::ParenClose);

            if self.is_at(&Token::Colon) || self.is_at(&Token::ColonEq) {
                // BufSet
                let is_const = self.eat(&Token::ColonEq);
                if !is_const {
                    self.eat(&Token::Colon);
                }
                let value = self.parse_expression();
                let end_span = self.last_consumed_span();
                let base = Typed::infer(
                    Expr::FnCall(FnCall {
                        path: Spanned::new(ModPath::new(name, vec![]), id_span),
                        args: None,
                    }),
                    id_span,
                );
                return Typed::infer(
                    Expr::BufSet {
                        buf: Box::new(base),
                        index: Box::new(index),
                        value: Box::new(value),
                    },
                    self.merge_span(id_span, end_span),
                );
            }
            // BufGet — return as expression
            let base = Typed::infer(
                Expr::FnCall(FnCall {
                    path: Spanned::new(ModPath::new(name, vec![]), id_span),
                    args: None,
                }),
                id_span,
            );
            let end_span = self.last_consumed_span();
            return Typed::infer(
                Expr::BufGet {
                    buf: Box::new(base),
                    index: Box::new(index),
                },
                self.merge_span(id_span, end_span),
            );
        }

        // TupleSet: name.N: value or TupleGet: name.N
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

            if self.is_at(&Token::Colon) || self.is_at(&Token::ColonEq) {
                // TupleSet
                let is_const = self.eat(&Token::ColonEq);
                if !is_const {
                    self.eat(&Token::Colon);
                }
                let value = self.parse_expression();
                let end_span = self.last_consumed_span();
                let base = Typed::infer(
                    Expr::FnCall(FnCall {
                        path: Spanned::new(ModPath::new(name, vec![]), id_span),
                        args: None,
                    }),
                    id_span,
                );
                return Typed::infer(
                    Expr::TupleSet {
                        base: Box::new(base),
                        index: idx as usize,
                        value: Box::new(value),
                    },
                    self.merge_span(id_span, end_span),
                );
            }
            // TupleGet — return as expression
            let base = Typed::infer(
                Expr::FnCall(FnCall {
                    path: Spanned::new(ModPath::new(name, vec![]), id_span),
                    args: None,
                }),
                id_span,
            );
            let end_span = self.last_consumed_span();
            return Typed::infer(
                Expr::TupleGet {
                    base: Box::new(base),
                    index: idx as usize,
                },
                self.merge_span(id_span, end_span),
            );
        }

        if matches!(self.peek(), Some(Token::Id(_)))
            && self.peek_at(1) == Some(&Token::Dot)
            && matches!(self.peek_at(2), Some(Token::Id(_)))
            && matches!(self.peek_at(3), Some(Token::Colon | Token::ColonEq))
        {
            let Some((Token::Id(base_name), base_span)) = self.advance() else {
                unreachable!()
            };
            self.advance();
            let Some((Token::Id(field_name), _)) = self.advance() else {
                unreachable!()
            };
            if !self.eat(&Token::ColonEq) {
                self.advance();
            }
            let value = self.parse_expression();
            let end_span = self.last_consumed_span();
            let base = Typed::infer(
                Expr::FnCall(FnCall {
                    path: Spanned::new(ModPath::new(self.intern(base_name), vec![]), base_span),
                    args: None,
                }),
                base_span,
            );
            return Typed::infer(
                Expr::RecordSet {
                    base: Box::new(base),
                    field: self.intern(field_name),
                    value: Box::new(value),
                },
                self.merge_span(base_span, end_span),
            );
        }

        // ── FnCall: name, name(args), name.path(args) ──
        // Dotted paths without `(` are field access (`a.x`), not qualified symbols (`a.x`).
        if let Some(path) = self.parse_path() {
            let path_span = path.span_id;
            let args = self.parse_paren_args();
            self.consume_trailing_newline();
            let end_span = self.last_consumed_span();
            if args.is_some() || path.value.segments.is_empty() {
                return Typed::infer(
                    Expr::FnCall(FnCall { path, args }),
                    self.merge_span(path_span, end_span),
                );
            }
            let mut expr = Typed::infer(
                Expr::FnCall(FnCall {
                    path: Spanned::new(ModPath::new(path.value.root, vec![]), path_span),
                    args: None,
                }),
                path_span,
            );
            for field in path.value.segments {
                let field_span = expr.span_id;
                expr = Typed::infer(
                    Expr::RecordGet {
                        base: Box::new(expr),
                        field,
                    },
                    self.merge_span(field_span, end_span),
                );
            }
            return expr;
        }

        let span = self.current_span();
        self.error("parse-expected-identifier", "expected identifier", span);
        Typed::infer(Expr::AnonymousTag(self.intern("__parse_error")), span)
    }

    // ─── Tag-based Atoms: TagCall, AnonymousTag, Tag-rooted FnCall ────────────────

    fn parse_tag_atom(&mut self) -> Typed<Expr> {
        // Single-pass lookahead avoids speculative parsing with rewind.
        // The common case (bare Tag or Tag(args)) skips all path parsing.
        if matches!(self.peek(), Some(Token::Tag(_) | Token::SelfTag))
            && matches!(self.peek_at(1), Some(Token::Dot))
        {
            // Qualified variant: Tag.Tag[(args)]
            if matches!(self.peek_at(2), Some(Token::Tag(_)))
                && let Some(path) = self.parse_tag_variant_path()
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
                && let Some(path) = self.parse_tag_path()
            {
                let path_span = path.span_id;
                let args = self.parse_paren_args();
                self.consume_trailing_newline();
                let end_span = self.last_consumed_span();
                return Typed::infer(
                    Expr::FnCall(FnCall { path, args }),
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
        self.error("parse-expected-tag", "expected tag", span);
        Typed::infer(Expr::AnonymousTag(self.intern("__parse_error")), span)
    }

    fn parse_list_lit(&mut self) -> Typed<Expr> {
        let start_span = self.peek_span().unwrap_or_else(|| self.current_span());
        self.advance(); // consume [

        self.skip_layout();

        // Empty list
        if self.is_at(&Token::BracketClose) {
            self.advance();
            let end_span = self.last_consumed_span();
            return Typed::infer(
                Expr::List(Vec::new()),
                self.merge_span(start_span, end_span),
            );
        }

        let mut elems = Vec::new();
        loop {
            if self.is_at(&Token::BracketClose) {
                break;
            }

            elems.push(self.parse_expression());

            if self.is_at(&Token::BracketClose) {
                break;
            }

            if self.eat_list_separator() {
                continue;
            }

            self.error(
                "parse-expected-list-separator",
                "expected ',' or newline between list elements",
                self.current_span(),
            );
            break;
        }

        self.skip_layout();
        self.expect(&Token::BracketClose);
        let end_span = self.last_consumed_span();
        Typed::infer(Expr::List(elems), self.merge_span(start_span, end_span))
    }

    /// `(` followed by `id:` — record literal for compile-time defaults and similar.
    fn is_record_literal_start(&self) -> bool {
        let mut offset = 0;
        loop {
            match self.peek_at(offset) {
                Some(Token::Newline) | Some(Token::Indent) | Some(Token::Dedent) => {
                    offset += 1;
                }
                Some(Token::Id(_)) => {
                    return matches!(self.peek_at(offset + 1), Some(Token::Colon));
                }
                _ => return false,
            }
        }
    }

    fn parse_record_literal_after_open_paren(&mut self, start_span: SpanId) -> Typed<Expr> {
        let mut fields: Vec<Typed<Expr>> = Vec::new();
        loop {
            self.skip_layout();
            if self.is_at(&Token::ParenClose) {
                break;
            }
            let (field_name, field_span) = match self.peek() {
                Some(Token::Id(n)) => {
                    let name = self.intern(n);
                    let span = self.peek_span().unwrap_or(start_span);
                    self.advance();
                    (name, span)
                }
                _ => {
                    self.error(
                        "parse-expected-field-name",
                        "expected record field name",
                        self.current_span(),
                    );
                    break;
                }
            };
            if self.expect(&Token::Colon).is_none() {
                break;
            }
            let value = self.parse_expression();
            let bind = Bind::new(field_name, field_span, BindValue::Expr(Box::new(value)));
            fields.push(Typed::infer(Expr::Bind(Box::new(bind)), field_span));

            if self.is_at(&Token::ParenClose) {
                break;
            }
            if !self.eat(&Token::Comma) {
                // No comma — newline is the separator (handled by skip_newlines
                // at the top of the next iteration).
                continue;
            }
        }

        self.skip_newlines();
        self.skip_indents();
        self.expect(&Token::ParenClose);
        let end_span = self.last_consumed_span();
        let named_fields: Vec<(Intern<String>, Typed<Expr>)> = fields
            .into_iter()
            .filter_map(|f| {
                if let Expr::Bind(ref b) = f.value {
                    Some((b.name, f))
                } else {
                    None
                }
            })
            .collect();
        Typed::infer(
            Expr::RecordLit(named_fields),
            self.merge_span(start_span, end_span),
        )
    }

    fn parse_tuple_lit_or_alloc_or_group(&mut self) -> Typed<Expr> {
        let start_span = self.peek_span().unwrap_or_else(|| self.current_span());
        self.advance(); // consume (

        self.skip_layout();

        if self.is_record_literal_start() {
            self.skip_newlines();
            self.skip_indents();
            return self.parse_record_literal_after_open_paren(start_span);
        }

        // Empty parens → unit or alloc placeholder
        if self.is_at(&Token::ParenClose) {
            self.advance();
            let end_span = self.last_consumed_span();
            return Typed::infer(
                Expr::TupleLit(Vec::new()),
                self.merge_span(start_span, end_span),
            );
        }

        let first = self.parse_expression();
        self.skip_newlines();

        // TupleAlloc: (init; size) — size must be compile-time-known
        if self.eat(&Token::ColonSemi) {
            let size = self.parse_expression();
            self.expect(&Token::ParenClose);
            let end_span = self.last_consumed_span();
            return Typed::infer(
                Expr::TupleAlloc {
                    init: Box::new(first),
                    size: Box::new(size),
                },
                self.merge_span(start_span, end_span),
            );
        }

        // Single element with no separator → grouped expression
        if self.is_at(&Token::ParenClose) {
            self.advance();
            return first;
        }

        // Multiple elements — comma or newline separated
        let mut elems = vec![first];
        loop {
            if self.is_at(&Token::ParenClose) {
                break;
            }

            if !(self.eat_list_separator() || self.previous_token_was_newline_separator()) {
                self.error(
                    "parse-expected-tuple-separator",
                    "expected ',' or newline between tuple elements",
                    self.current_span(),
                );
                break;
            }

            if self.is_at(&Token::ParenClose) {
                break;
            }

            elems.push(self.parse_expression());
        }

        self.expect(&Token::ParenClose);
        let end_span = self.last_consumed_span();
        Typed::infer(Expr::TupleLit(elems), self.merge_span(start_span, end_span))
    }

    fn parse_take_ptr(&mut self) -> Typed<Expr> {
        let (_, start_span) = self
            .advance()
            .expect("peek confirmed '@' token before parse_take_ptr"); // @
        let inner = self.parse_expression();
        let end_span = inner.span_id;
        Typed::infer(
            Expr::TakePtr(Box::new(inner)),
            self.merge_span(start_span, end_span),
        )
    }

    fn parse_deref(&mut self) -> Typed<Expr> {
        let (_, start_span) = self
            .advance()
            .expect("peek confirmed '*' token before parse_deref"); // *
        let inner = self.parse_expression();
        let end_span = inner.span_id;
        Typed::infer(
            Expr::Deref(Box::new(inner)),
            self.merge_span(start_span, end_span),
        )
    }

    fn parse_ref_take(&mut self) -> Typed<Expr> {
        let (_, start_span) = self
            .advance()
            .expect("peek confirmed 'ref' token before parse_ref_take");
        let inner = self.parse_expression();
        let end_span = inner.span_id;
        Typed::infer(
            Expr::Ref {
                inner: Box::new(inner),
                mutable: false,
                group: None,
            },
            self.merge_span(start_span, end_span),
        )
    }

    fn parse_mut_take(&mut self) -> Typed<Expr> {
        let (_, start_span) = self
            .advance()
            .expect("peek confirmed 'mut' token before parse_mut_take");
        let inner = self.parse_expression();
        let end_span = inner.span_id;
        Typed::infer(
            Expr::Ref {
                inner: Box::new(inner),
                mutable: true,
                group: None,
            },
            self.merge_span(start_span, end_span),
        )
    }

    fn parse_deref_expr(&mut self) -> Typed<Expr> {
        let (_, start_span) = self
            .advance()
            .expect("peek confirmed 'deref' token before parse_deref_expr");
        let inner = self.parse_expression();
        let end_span = inner.span_id;
        Typed::infer(
            Expr::Deref(Box::new(inner)),
            self.merge_span(start_span, end_span),
        )
    }

    fn parse_eat_expr(&mut self) -> Typed<Expr> {
        let (_, start_span) = self
            .advance()
            .expect("peek confirmed 'eat' token before parse_eat_expr");
        let inner = self.parse_expression();
        let end_span = inner.span_id;
        Typed::infer(
            Expr::Eat(Box::new(inner)),
            self.merge_span(start_span, end_span),
        )
    }

    fn parse_self_ref(&mut self) -> Typed<Expr> {
        let (_, span) = self
            .advance()
            .expect("peek confirmed 'self' token before parse_self_ref");
        // Return bare SelfRef — apply_postfix handles `.field` → RecordGet.
        Typed::infer(Expr::SelfRef, span)
    }

    /// Map a comparison token to the function name it desugars to.
    ///
    /// Comparison operators are NOT represented as `BinOp` variants.
    /// Instead, `a == b` desugars to `eq(a, b)`, `a < b` to `lt(a, b)`, etc.
    /// These function calls go through normal trait method resolution.
    fn try_comparison_op(&self, token: &Token) -> Option<(&'static str, u8)> {
        match token {
            Token::Less => Some(("lt", 3)),
            Token::LessEq => Some(("le", 3)),
            Token::Greater => Some(("gt", 3)),
            Token::GreaterEq => Some(("ge", 3)),
            _ => None,
        }
    }

    fn try_infix_op(&self, token: &Token) -> Option<(BinOp, u8)> {
        match token {
            // Precedence 1-2: logical
            Token::Or => Some((BinOp::LogicalOr, 1)),
            Token::And => Some((BinOp::LogicalAnd, 2)),
            // Precedence 3: bitwise
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
        // .N → TupleGet
        if self.is_at(&Token::Dot)
            && let Some(&Token::Int(n)) = self.peek_at(1)
        {
            self.advance(); // Dot
            self.advance(); // Int
            let end_span = self.last_consumed_span();
            let lhs_span = lhs.span_id;
            return Ok(Typed::infer(
                Expr::TupleGet {
                    base: Box::new(lhs),
                    index: n as usize,
                },
                self.merge_span(lhs_span, end_span),
            ));
        }

        // .(expr) → BufGet
        if self.is_at(&Token::Dot) && self.peek_at(1) == Some(&Token::ParenOpen) {
            self.advance(); // Dot
            self.advance(); // ParenOpen
            let index = self.parse_expression();
            self.expect(&Token::ParenClose);
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

                // Check for field write: p.field: value
                if self.is_at(&Token::Colon) {
                    self.advance(); // Colon
                    let value = self.parse_expression();
                    let end_span = self.last_consumed_span();
                    return Ok(Typed::infer(
                        Expr::RecordSet {
                            base: Box::new(lhs),
                            field,
                            value: Box::new(value),
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
            let method_name = match self.advance() {
                Some((Token::Id(name), _)) => self.intern(name),
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
                    path: Spanned::new(ModPath::new(method_name, vec![]), lhs_span),
                    args: Some(all_args),
                }),
                self.merge_span(lhs_span, end_span),
            ));
        }

        // as Type → Cast
        if self.is_at(&Token::As) {
            self.advance(); // As
            match self.advance() {
                Some((Token::Tag(name), _)) => {
                    let ty = self.intern(name);
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
                _ => {
                    self.error(
                        "parse-expected-type-name",
                        "expected type name after 'as'",
                        self.current_span(),
                    );
                }
            }
        }

        Err(lhs)
    }

    fn parse_loop_or_control(&mut self) -> Option<Typed<Expr>> {
        let start_span = self.current_span();

        if let Some(if_expr) = self.parse_if_expr(|c: &mut TokenCursor| c.parse_expression()) {
            let end_span = self.last_consumed_span();
            return Some(Typed::infer(
                Expr::If(if_expr),
                self.merge_span(start_span, end_span),
            ));
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
                    parts.push(FormatPart::Text(s.unescape()));
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
                    return Typed::infer(Expr::FormatString(FormatString { parts }), fmt_span);
                }
                Some(Token::UnterminatedFormatString) | None => {
                    self.advance_drop();
                    let span = self.current_span();
                    self.error(
                        "parse-unterminated-format-string",
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
                    self.error(
                        "parse-unexpected-token-in-format-string",
                        "unexpected token in format string",
                        span,
                    );
                    self.advance();
                    self.advance_pop();
                }
            }
        }
    }

    fn parse_asm_expr(&mut self) -> Typed<Expr> {
        let start_span = self.peek_span().unwrap_or_else(|| self.current_span());
        self.advance(); // eat Asm

        // expect (
        if !self.eat(&Token::ParenOpen) {
            self.error(
                "parse-expected-paren",
                "expected '(' after 'asm'",
                self.current_span(),
            );
            return Typed::infer(Expr::AnonymousTag(self.intern("__parse_error")), start_span);
        }

        // Parse the spec expression (an AsmSpec value constructed with :=)
        let spec_expr = self.parse_expression();

        // Parse optional runtime operand values
        let mut operand_values = Vec::new();
        while self.eat(&Token::Comma) {
            // Skip indent/dedent tokens that appear after newlines
            while matches!(self.peek(), Some(Token::Indent) | Some(Token::Dedent)) {
                self.advance();
            }
            if self.is_at(&Token::ParenClose) {
                break;
            }
            operand_values.push(self.parse_expression());
        }

        // expect )
        if !self.eat(&Token::ParenClose) {
            self.error(
                "parse-expected-paren",
                "expected ')' after asm operands",
                self.current_span(),
            );
            return Typed::infer(Expr::AnonymousTag(self.intern("__parse_error")), start_span);
        }

        // Consume any trailing dedent tokens that were produced by the indent-aware lexer
        // for arguments inside the `(...)`. These dedents should not leak out to the enclosing
        // body parser.
        while matches!(self.peek(), Some(Token::Dedent)) {
            self.advance();
        }

        let end_span = self.last_consumed_span();

        Typed::infer(
            Expr::Asm(AsmExpr {
                template: Intern::new(String::new()),
                operands: Vec::new(),
                clobbers: Vec::new(),
                operand_values,
                spec_expr: Some(Box::new(spec_expr)),
            }),
            self.merge_span(start_span, end_span),
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::query::SourceParseExt;

    fn has_unexpected_token_symptom(output: &crate::query::ParseOutput, token: &str) -> bool {
        output
            .symptoms
            .iter()
            .any(|d| d.message == format!("unexpected token {token}"))
    }

    #[test]
    fn standalone_equals_is_unexpected_token() {
        let output = "main:\n  =\n    return".parse_source_full();
        assert!(has_unexpected_token_symptom(&output, "`=`"));
    }

    #[test]
    fn standalone_double_equals_is_unexpected_token() {
        let output = "main:\n  ==\n    return".parse_source_full();
        assert!(has_unexpected_token_symptom(&output, "`==`"));
    }

    #[test]
    fn standalone_not_equals_is_unexpected_token() {
        let output = "main:\n  /=\n    return".parse_source_full();
        assert!(has_unexpected_token_symptom(&output, "`/=`"));
    }

    #[test]
    fn standalone_equals_after_blank_lines_uses_equals_span() {
        let output = "main:\n  =\n    return".parse_source_full();
        assert!(has_unexpected_token_symptom(&output, "`=`"));
    }

    #[test]
    fn infix_equals_is_unexpected_token() {
        let output = "x: a = b\n".parse_source_full();
        assert!(has_unexpected_token_symptom(&output, "`=`"));
    }

    #[test]
    fn infix_double_equals_is_unexpected_token() {
        let output = "x: a == b\n".parse_source_full();
        assert!(has_unexpected_token_symptom(&output, "`==`"));
    }

    #[test]
    fn infix_not_equals_is_unexpected_token() {
        let output = "x: a /= b\n".parse_source_full();
        assert!(has_unexpected_token_symptom(&output, "`/=`"));
    }

    #[test]
    fn less_than_still_works() {
        let output = "x: a < b\n".parse_source_full();
        assert!(!has_unexpected_token_symptom(&output, "`<`"));
    }

    #[test]
    fn greater_than_still_works() {
        let output = "x: a > b\n".parse_source_full();
        assert!(!has_unexpected_token_symptom(&output, "`>`"));
    }

    #[test]
    fn less_than_or_equal_still_works() {
        let output = "x: a <= b\n".parse_source_full();
        assert!(!has_unexpected_token_symptom(&output, "`<=`"));
    }

    #[test]
    fn greater_than_or_equal_still_works() {
        let output = "x: a >= b\n".parse_source_full();
        assert!(!has_unexpected_token_symptom(&output, "`>=`"));
    }
}
