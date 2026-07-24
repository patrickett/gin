use std::cell::Cell;

use ast::span::{HasSpanId, Span, SpanId, SpanTable};
use internment::Intern;
use lexer::Token;

#[derive(Debug, Clone)]
pub struct ParseError {
    code: &'static str,
    message: String,
    span: SpanId,
}

impl ParseError {
    fn new(code: &'static str, message: impl Into<String>, span: SpanId) -> Self {
        Self {
            code,
            message: message.into(),
            span,
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn span(&self) -> SpanId {
        self.span
    }

    pub fn hint(message: impl Into<String>, span: SpanId) -> Self {
        Self::new("parse-hint", message, span)
    }

    pub fn expected_token(message: impl Into<String>, span: SpanId) -> Self {
        Self::new("parse-expected-token", message, span)
    }

    pub fn unexpected_token(message: impl Into<String>, span: SpanId) -> Self {
        Self::new("parse-unexpected-token", message, span)
    }

    pub fn invalid_import_target(message: impl Into<String>, span: SpanId) -> Self {
        Self::new("parse-invalid-import-target", message, span)
    }

    pub fn expected_import_source(message: impl Into<String>, span: SpanId) -> Self {
        Self::new("parse-expected-import-source", message, span)
    }

    pub fn empty_import_bundle(message: impl Into<String>, span: SpanId) -> Self {
        Self::new("parse-empty-import-bundle", message, span)
    }
}

impl HasSpanId for ParseError {
    fn span_id(&self) -> SpanId {
        self.span
    }
}

pub struct TokenCursor<'src, 't> {
    tokens: &'t [(Token<'src>, SpanId)],
    span_table: &'t mut SpanTable,
    pos: usize,
    last_consumed_pos: Option<usize>,
    sig_pos_cache: Cell<Option<usize>>,
    /// Stack of saved positions for [`Self::advance_pop`] progress assertions.
    /// See the section "Loop progress assertions" below.
    advance_stack: Vec<usize>,
    cancel_check: Option<&'t (dyn Fn() + 't)>,
    pub errors: Vec<ParseError>,
}

impl<'src, 't> TokenCursor<'src, 't> {
    pub fn new(tokens: &'t [(Token<'src>, SpanId)], span_table: &'t mut SpanTable) -> Self {
        Self {
            tokens,
            span_table,
            pos: 0,
            last_consumed_pos: None,
            sig_pos_cache: Cell::new(None),
            advance_stack: Vec::new(),
            cancel_check: None,
            errors: Vec::new(),
        }
    }

    /// Builder: install a cancellation hook to be invoked once per parse-loop
    /// iteration. The hook should panic (typically with `salsa::Cancelled`) to
    /// abort the parse mid-flight — the hook itself never returns a value.
    ///
    /// # Cooperative cancellation
    ///
    /// Layer 1 (progress assertions) guarantees the parser terminates on any
    /// input. Layer 2 (this hook) makes it *interruptible* — when a new edit
    /// arrives mid-parse, the salsa write triggers cancellation, and the very
    /// next loop iteration unwinds out of the parser via the hook's panic.
    /// Without it, an in-flight parse on stale input runs to completion before
    /// the new edit's compute can start.
    pub fn with_cancel_check(mut self, check: &'t (dyn Fn() + 't)) -> Self {
        self.cancel_check = Some(check);
        self
    }

    /// Invoke the cancellation hook (if installed). Called from
    /// [`Self::advance_pop`]; if the hook panics, the panic propagates up
    /// through the parser stack to the caller of `parse_*`.
    #[inline]
    fn check_cancellation(&self) {
        if let Some(check) = self.cancel_check {
            check();
        }
    }

    #[inline(always)]
    fn invalidate_cache(&self) {
        self.sig_pos_cache.set(None);
    }

    fn significant_pos(&self) -> usize {
        if let Some(cached) = self.sig_pos_cache.get() {
            return cached;
        }
        let mut p = self.pos;
        while p < self.tokens.len() && matches!(self.tokens[p].0, Token::Newline) {
            p += 1;
        }
        self.sig_pos_cache.set(Some(p));
        p
    }

    #[inline(always)]
    pub fn peek(&self) -> Option<&Token<'src>> {
        let pos = self.significant_pos();
        self.tokens.get(pos).map(|(t, _)| t)
    }

    #[inline(always)]
    pub fn peek_span(&self) -> Option<SpanId> {
        let pos = self.significant_pos();
        self.tokens.get(pos).map(|(_, s)| *s)
    }

    /// Skip leading newlines, consume one token. Does NOT skip trailing
    /// newlines so that consume_trailing_newline can extend spans.
    #[inline(always)]
    pub fn advance(&mut self) -> Option<(Token<'src>, SpanId)> {
        let pos = self.significant_pos(); // uses cache, no redundant scan
        let item = self.tokens.get(pos)?;
        self.last_consumed_pos = Some(pos);
        self.pos = pos + 1;
        self.invalidate_cache();
        Some((item.0, item.1))
    }

    /// If the raw token at the current position is a newline, consume it so
    /// that last_consumed_span includes the newline span.
    #[inline(always)]
    pub fn consume_trailing_newline(&mut self) {
        if self.pos < self.tokens.len() && matches!(self.tokens[self.pos].0, Token::Newline) {
            self.last_consumed_pos = Some(self.pos);
            self.pos += 1;
            while self.pos < self.tokens.len() && matches!(self.tokens[self.pos].0, Token::Newline)
            {
                self.pos += 1;
            }
            self.invalidate_cache();
        }
    }

    #[inline(always)]
    pub fn is_at(&self, token: &Token<'src>) -> bool {
        self.peek() == Some(token)
    }

    #[inline(always)]
    pub fn eat(&mut self, token: &Token<'src>) -> bool {
        if self.is_at(token) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub fn expect(&mut self, token: &Token<'src>) -> Option<(Token<'src>, SpanId)> {
        if self.is_at(token) {
            self.advance()
        } else {
            let span = self.peek_span().unwrap_or(SpanId::INVALID);
            let found = self
                .peek()
                .map(|t| format!("{t:?}"))
                .unwrap_or_else(|| "end of input".to_string());
            self.errors.push(ParseError::expected_token(
                format!("expected {:?}, found {found}", token),
                span,
            ));
            None
        }
    }

    #[inline(always)]
    pub fn peek_at(&self, offset: usize) -> Option<&Token<'src>> {
        self.tokens.get(self.pos + offset).map(|(t, _)| t)
    }

    #[inline(always)]
    pub fn is_eof(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    #[inline(always)]
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Get actual span data from a SpanId using the span_table
    pub fn get_span(&self, id: SpanId) -> Span {
        self.span_table.get(id)
    }

    #[inline(always)]
    pub fn span_at(&self, pos: usize) -> SpanId {
        self.tokens
            .get(pos)
            .map(|(_, s)| *s)
            .unwrap_or_else(|| self.eof_span())
    }

    #[inline(always)]
    pub fn span_table(&self) -> &SpanTable {
        self.span_table
    }

    #[inline(always)]
    pub fn span_table_mut(&mut self) -> &mut SpanTable {
        self.span_table
    }

    /// Merge two span IDs into a new span ID that covers both.
    /// The merged span is automatically added to the span table.
    pub fn merge_span(&mut self, a: SpanId, b: SpanId) -> SpanId {
        self.span_table.merge(a, b)
    }

    #[inline(always)]
    pub fn current_span(&self) -> SpanId {
        self.span_at(self.pos)
    }

    pub fn last_consumed_span(&self) -> SpanId {
        match self.last_consumed_pos {
            Some(p) => self.tokens[p].1,
            None => self.eof_span(),
        }
    }

    pub fn advance_raw(&mut self) -> Option<(Token<'src>, SpanId)> {
        let item = self.tokens.get(self.pos)?;
        self.last_consumed_pos = Some(self.pos);
        self.pos += 1;
        self.invalidate_cache();
        Some((item.0, item.1))
    }

    pub fn blank_line_between_last_consumed_and_raw_peek(&self) -> bool {
        let Some(last_pos) = self.last_consumed_pos else {
            return false;
        };
        let Some((_, next_span_id)) = self.tokens.get(self.pos) else {
            return false;
        };
        let last_span = self.span_table.get(self.tokens[last_pos].1);
        let next_span = self.span_table.get(*next_span_id);
        next_span.start() > last_span.end()
    }

    pub fn eof_span(&self) -> SpanId {
        self.tokens
            .last()
            .map(|(_, id)| {
                // Return a zero-length span at the end of the last span
                let _span = self.span_table.get(*id);
                // We need to insert a new span for the EOF position
                // For now, return INVALID since we can't insert into span_table here
                SpanId::INVALID
            })
            .unwrap_or(SpanId::INVALID)
    }

    /// Peek at the raw next token without skipping any intervening tokens.
    /// This is the only way to detect newline tokens that `peek()` would skip.
    #[inline(always)]
    pub fn raw_peek(&self) -> Option<&Token<'src>> {
        self.tokens.get(self.pos).map(|(t, _)| t)
    }

    /// Skip layout tokens — newlines, indents, and dedents — that can appear
    /// between items in a delimited list (parens, brackets, etc.).
    /// Returns `true` if at least one token was consumed.
    pub fn skip_layout(&mut self) -> bool {
        let start = self.pos;
        let tokens = self.tokens;
        while self.pos < tokens.len()
            && matches!(
                tokens[self.pos].0,
                Token::Newline | Token::Indent | Token::Dedent
            )
        {
            self.pos += 1;
        }
        let did_skip = self.pos > start;
        if did_skip {
            self.invalidate_cache();
        }
        did_skip
    }

    /// Returns true when expression parsing already consumed the newline that
    /// should separate the last item from the next significant token.
    pub fn previous_token_was_newline_separator(&mut self) -> bool {
        let Some(last_pos) = self.last_consumed_pos else {
            return false;
        };

        let next_pos = self.significant_pos();
        if last_pos >= next_pos {
            return false;
        }

        let has_newline =
            (last_pos..next_pos).any(|pos| matches!(self.tokens[pos].0, Token::Newline));
        if has_newline {
            self.skip_layout();
        }
        has_newline
    }

    /// Eat a list separator: either a comma (followed by optional layout)
    /// or layout containing at least one newline. Returns `true` if a separator
    /// was consumed.
    ///
    /// This is the core of newline-as-separator in delimited lists:
    /// `(a, b)` and `(a\nb)` are both valid, but `(a b)` is not.
    ///
    /// When a comma is consumed but a newline (before or after the comma) or a
    /// closing delimiter already provides separation, the comma is flagged as
    /// unnecessary via a `__gin_hint__:unnecessary-comma` parse error.
    pub fn eat_list_separator(&mut self) -> bool {
        // Check for newlines between the current raw position and where a
        // comma would be found (after skipping layout). This detects commas
        // that appear on a line following a newline-based separator.
        let sig_pos = self.significant_pos();
        let newline_before = self
            .tokens
            .get(sig_pos)
            .is_some_and(|(t, _)| *t == Token::Comma)
            && (self.pos..sig_pos).any(|i| matches!(self.tokens[i].0, Token::Newline));

        // Try comma first
        if self.eat(&Token::Comma) {
            let saw_layout = self.skip_layout();

            // A comma is unnecessary when:
            // - it is followed by layout (newline after the comma), OR
            // - a newline preceded the comma (comma after newline-based separator), OR
            // - the next significant token is a closing delimiter (trailing comma)
            if saw_layout
                || newline_before
                || self.is_at(&Token::ParenClose)
                || self.is_at(&Token::BracketClose)
            {
                let span = self.last_consumed_span();
                self.errors
                    .push(ParseError::hint("__gin_hint__:unnecessary-comma", span));
            }

            return true;
        }

        // Try newline-based layout (one or more newlines + optional Indent/Dedent)
        let start = self.pos;
        let mut has_newline = false;
        let tokens = self.tokens;
        while self.pos < tokens.len() {
            match &tokens[self.pos].0 {
                Token::Newline => {
                    has_newline = true;
                    self.pos += 1;
                }
                Token::Indent | Token::Dedent => {
                    self.pos += 1;
                }
                _ => break,
            }
        }

        if has_newline {
            self.invalidate_cache();
            true
        } else {
            self.pos = start;
            false
        }
    }

    /// Try to consume a statement-level separator: a comma (the inline separator)
    /// or one or more newlines (the layout separator). Returns `true` if a
    /// separator was consumed.
    ///
    /// Used by `parse_top_level_element` and `parse_body_exprs` to enforce
    /// separators between declarations and statements.
    pub fn try_consume_statement_separator(&mut self) -> bool {
        // Try comma first — this is the explicit inline separator.
        if self.eat(&Token::Comma) {
            self.skip_newlines();
            return true;
        }

        // Try newline(s) — this is the free layout separator.
        let start = self.pos;
        let tokens = self.tokens;
        let mut consumed_newline = false;
        while self.pos < tokens.len() {
            match &tokens[self.pos].0 {
                Token::Newline => {
                    consumed_newline = true;
                    self.pos += 1;
                }
                Token::Indent | Token::Dedent => {
                    self.pos += 1;
                }
                _ => break,
            }
        }
        if consumed_newline {
            self.invalidate_cache();
            true
        } else {
            self.pos = start;
            false
        }
    }

    /// Peek at the next token past any intervening newlines AND indents,
    /// without consuming them. Used for Indent-as-continuation in expressions.
    #[inline]
    pub fn peek_past_indent(&self) -> Option<&Token<'src>> {
        let mut p = self.pos;
        let tokens = self.tokens;
        while p < tokens.len() && matches!(tokens[p].0, Token::Newline | Token::Indent) {
            p += 1;
        }
        tokens.get(p).map(|(t, _)| t)
    }

    /// Skip past all leading newline and indent tokens.
    /// Used when Indent should be treated as expression continuation.
    #[inline]
    pub fn skip_indents(&mut self) {
        let tokens = self.tokens;
        while self.pos < tokens.len()
            && matches!(tokens[self.pos].0, Token::Newline | Token::Indent)
        {
            self.pos += 1;
        }
        self.invalidate_cache();
    }

    #[inline(always)]
    pub fn skip_newlines(&mut self) {
        let tokens = self.tokens;
        while self.pos < tokens.len() && matches!(tokens[self.pos].0, Token::Newline) {
            self.pos += 1;
        }
        self.invalidate_cache();
    }

    pub fn error(&mut self, code: &'static str, message: impl Into<String>, span: SpanId) {
        self.errors.push(ParseError::new(code, message, span));
    }

    pub fn hint(&mut self, message: impl Into<String>, span: SpanId) {
        self.errors.push(ParseError::hint(message, span));
    }

    pub fn expected_token(&mut self, message: impl Into<String>, span: SpanId) {
        self.errors.push(ParseError::expected_token(message, span));
    }

    pub fn unexpected_token(
        &mut self,
        message: impl Into<String>,
        _suggestion: Option<String>,
        span: SpanId,
    ) {
        self.errors
            .push(ParseError::unexpected_token(message, span));
    }

    pub fn invalid_import_target(&mut self, message: impl Into<String>, span: SpanId) {
        self.errors
            .push(ParseError::invalid_import_target(message, span));
    }

    pub fn expected_import_source(&mut self, message: impl Into<String>, span: SpanId) {
        self.errors
            .push(ParseError::expected_import_source(message, span));
    }

    pub fn empty_import_bundle(&mut self, message: impl Into<String>, span: SpanId) {
        self.errors
            .push(ParseError::empty_import_bundle(message, span));
    }

    pub fn checkpoint(&self) -> usize {
        self.pos
    }

    pub fn rewind(&mut self, pos: usize) {
        self.pos = pos;
        self.invalidate_cache();
    }

    // Materializes the implicit "this loop body always consumes a token"
    // contract that recursive-descent parsers depend on. Wrap each parser
    // production called inside a loop:
    //
    //   loop {
    //       cursor.advance_push();
    //       match parse_thing(cursor) {
    //           Some(x) => { cursor.advance_pop(); /* consume x */ }
    //           None    => { cursor.advance_drop(); break; }
    //       }
    //   }
    //
    // `advance_pop` panics if the cursor did not move past the saved position.
    // `advance_drop` discards the saved position without checking — use it on
    // the "no match, exit loop" branch, where not advancing is expected.
    //
    // Rationale: an infinite parse loop is by far the worst failure mode for
    // an editor — silent, blocks the LSP, and recovery is impossible without
    // killing the process. Asserting progress at the call site converts that
    // class of bug into a panic with a precise stack trace that `catch_unwind`
    // in the LSP can surface as an LSP error, and that tests can lock in.
    // Ref: https://matklad.github.io/2025/12/28/parsing-advances.html

    /// Record the current position so a later [`Self::advance_pop`] can assert
    /// the cursor moved past it.
    #[inline]
    pub fn advance_push(&mut self) {
        self.advance_stack.push(self.pos);
    }

    /// Pop the most recent [`Self::advance_push`] and panic if the cursor did
    /// not advance past it. Use after a parser production succeeded inside a
    /// loop body.
    ///
    /// After the progress assertion succeeds, this also invokes the
    /// cancellation hook (if installed). This is the canonical place for the
    /// check: every productive parser iteration funnels through here, so the
    /// hook fires at most once per loop iteration but at least once per
    /// iteration of every loop guarded by a push/pop pair.
    #[inline]
    #[track_caller]
    pub fn advance_pop(&mut self) {
        let prev = self
            .advance_stack
            .pop()
            .expect("advance_pop called without matching advance_push");
        assert!(
            self.pos > prev,
            "parse loop made no progress at pos={prev} \
             (token={:?}); this is a parser bug — every iteration must consume at least one token",
            self.tokens.get(prev).map(|(t, _)| t),
        );
        self.check_cancellation();
    }

    /// Pop the most recent [`Self::advance_push`] without asserting progress.
    /// Use on the "no match, exit loop" branch where not advancing is intended.
    #[inline]
    pub fn advance_drop(&mut self) {
        self.advance_stack
            .pop()
            .expect("advance_drop called without matching advance_push");
    }

    #[inline(always)]
    pub fn intern(&mut self, s: &'src str) -> Intern<String> {
        Intern::<String>::from_ref(s)
    }
}
