use crate::token::{LexContext, MAX_INDENT_DEPTH, Token};
use diagnostic::code::LexCode;
use diagnostic::{Diagnostic, DiagnosticCode};
use i256::I256;
use memchr::{memchr, memchr2};
use span::{Span, SpanId, SpanTable};

struct FormatStringState<'src> {
    tokens: Vec<(Token<'src>, SpanId)>,
    idx: usize,
}

impl<'src> FormatStringState<'src> {
    fn next(&mut self) -> Option<(Token<'src>, SpanId)> {
        if self.idx < self.tokens.len() {
            let item = self.tokens[self.idx];
            self.idx += 1;
            Some(item)
        } else {
            None
        }
    }
}

pub struct Lexer<'src> {
    source: &'src str,
    bytes: &'src [u8],
    pos: usize,
    pub errors: Vec<Diagnostic>,
    indent: LexContext,
    last_indent_span: SpanId,
    fmt: Option<FormatStringState<'src>>,
    span_table: SpanTable,
    blank_lines: Vec<usize>,
    /// Byte offset of the next `\n`, or `source.len()` if none remains.
    line_end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommentKind {
    Line,
    Doc,
    ModuleDoc,
}

impl<'src> Lexer<'src> {
    pub fn new(source: &'src str) -> Self {
        let bytes = source.as_bytes();
        let line_end = memchr(b'\n', bytes).unwrap_or(source.len());
        // Rough estimate: one span per ~6 bytes of source (token spans + merges).
        // This avoids repeated Vec reallocation during lexing.
        let estimated_spans = (source.len() / 6).max(64);
        Self {
            source,
            bytes,
            pos: 0,
            errors: Vec::new(),
            indent: LexContext::default(),
            last_indent_span: SpanId::INVALID,
            fmt: None,
            span_table: SpanTable::with_capacity(estimated_spans),
            blank_lines: Vec::new(),
            line_end,
        }
    }

    #[inline]
    fn current_span(&self, start: usize) -> std::ops::Range<usize> {
        start..self.pos
    }

    #[inline]
    fn insert_span(&mut self, range: std::ops::Range<usize>) -> SpanId {
        self.span_table.insert(Span::from_range(range))
    }

    fn push_error(&mut self, code: DiagnosticCode, message: &str, span: SpanId) {
        self.errors
            .push(Diagnostic::new(code, message).at_span_id(span, &self.span_table));
    }

    /// Get the span data for a given SpanId.
    pub fn get_span(&self, id: SpanId) -> Span {
        self.span_table.get(id)
    }

    /// Get a reference to the span table for resolving SpanIds.
    pub fn span_table(&self) -> &SpanTable {
        &self.span_table
    }

    /// Take the span table, leaving an empty one in its place.
    pub fn take_span_table(&mut self) -> SpanTable {
        std::mem::take(&mut self.span_table)
    }

    pub fn take_blank_lines(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.blank_lines)
    }

    #[inline]
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    #[inline]
    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    #[inline]
    fn advance(&mut self) -> Option<u8> {
        let b = self.bytes.get(self.pos)?;
        self.pos += 1;
        Some(*b)
    }

    #[inline]
    fn slice_from(&self, start: usize) -> &'src str {
        &self.source[start..self.pos]
    }

    fn skip_inline_whitespace(&mut self) {
        let bytes = self.bytes;
        while self.pos < bytes.len() && matches!(bytes[self.pos], b' ' | b'\t') {
            self.pos += 1;
        }
    }

    /// Recompute `line_end` if it's stale (pos has moved past it.
    #[inline]
    fn ensure_line_end(&mut self) {
        if self.line_end < self.pos {
            let bytes = self.bytes;
            self.line_end = memchr(b'\n', &bytes[self.pos..]).map_or(bytes.len(), |i| self.pos + i);
        }
    }

    fn handle_newline(&mut self, newline_start: usize) -> Option<(Token<'src>, SpanId)> {
        let range = self.current_span(newline_start);
        let span = self.insert_span(range);
        self.last_indent_span = span;
        let bytes = self.bytes;

        loop {
            let indent = self.lex_indent();

            // Cheap blank-line check — no memchr needed
            match bytes.get(self.pos) {
                None => return Some((Token::Newline, span)),
                Some(b'\n') => {
                    self.pos += 1;
                    self.blank_lines.push(self.pos);
                    continue;
                }
                _ => {}
            }

            // Non-blank line: compute line_end for comment/format-string consumers
            self.line_end = memchr(b'\n', &bytes[self.pos..]).map_or(bytes.len(), |i| self.pos + i);

            let depth = self.indent.indent_depth as usize;
            assert!(depth > 0 && depth <= MAX_INDENT_DEPTH);
            let current = self.indent.indent_stack[depth - 1];

            if indent > current
                && bytes[self.pos..].starts_with(b"and")
                && bytes.get(self.pos + 3).is_some_and(u8::is_ascii_whitespace)
            {
                return Some((Token::Newline, span));
            }

            if indent > current {
                if depth < MAX_INDENT_DEPTH {
                    self.indent.indent_stack[depth] = indent;
                    self.indent.indent_depth += 1;
                    self.indent.pending_indent = true;
                } else {
                    self.indent.indent_overflow = true;
                }
            } else if indent < current {
                let mut d = depth;
                while d > 1 && self.indent.indent_stack[d - 1] > indent {
                    d -= 1;
                }
                self.indent.pending_dedents = (depth - d) as u8;
                self.indent.indent_depth = d as u8;
            }

            return Some((Token::Newline, span));
        }
    }

    fn lex_indent(&mut self) -> u16 {
        let bytes = self.bytes;
        let mut indent = 0u16;
        while self.pos < bytes.len() {
            let width = match bytes[self.pos] {
                b' ' => 1,
                b'\t' => 4,
                _ => break,
            };
            match indent.checked_add(width) {
                Some(next) => indent = next,
                None => {
                    indent = u16::MAX;
                    self.indent.indent_overflow = true;
                }
            }
            self.pos += 1;
        }
        indent
    }

    fn lex_ident(&mut self, start: usize, allow_underscore: bool) -> (&'src str, SpanId) {
        while self.pos < self.bytes.len() {
            let byte = self.bytes[self.pos];
            let continues = matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9')
                || (allow_underscore && byte == b'_');
            if !continues {
                break;
            }
            self.pos += 1;
        }
        let text = self.slice_from(start);
        let range = self.current_span(start);
        let span = self.insert_span(range);
        (text, span)
    }

    fn lex_keyword_or_id(&mut self, start: usize) -> (Token<'src>, SpanId) {
        let (text, span) = self.lex_ident(start, true);
        (Token::from_keyword(text).unwrap_or(Token::Id(text)), span)
    }

    fn lex_tag(&mut self, start: usize) -> (Token<'src>, SpanId) {
        let (text, span) = self.lex_ident(start, false);
        (Token::from_keyword(text).unwrap_or(Token::Tag(text)), span)
    }

    fn parse_float_bytes(bytes: &[u8]) -> Option<f64> {
        let mut buf = [0u8; 64];
        let mut len = 0;
        for &b in bytes {
            if b == b'_' {
                continue;
            }
            if len >= buf.len() {
                return None;
            }
            buf[len] = b;
            len += 1;
        }
        fast_float::parse(&buf[..len]).ok()
    }

    fn lex_number(&mut self, start: usize) -> (Token<'src>, SpanId) {
        let is_hex = self.bytes[start] == b'0'
            && matches!(self.peek(), Some(b'x' | b'X'))
            && self.peek_at(1).is_some_and(|b| b.is_ascii_hexdigit());
        let (value, had_underscore) = if is_hex {
            self.pos += 1;
            self.consume_digits(16, Some(I256::from(0)))
        } else {
            let first = I256::from(self.bytes[start] - b'0');
            self.consume_digits(10, Some(first))
        };

        if !is_hex
            && self.peek() == Some(b'.')
            && self.peek_at(1).is_some_and(|b| b.is_ascii_digit())
        {
            self.pos += 1;
            let (_, fraction_had_underscore) = self.consume_digits(10, None);
            let range = self.current_span(start);
            let span = self.insert_span(range);
            if had_underscore || fraction_had_underscore {
                // lex-invalid-float
                self.push_error(
                    DiagnosticCode::Lex(LexCode::InvalidFloat),
                    "float literal has invalid underscore placement",
                    span,
                );
            }
            return match Self::parse_float_bytes(&self.bytes[start..self.pos]) {
                Some(v) => (Token::Float(v), span),
                None => {
                    // lex-invalid-float
                    self.push_error(
                        DiagnosticCode::Lex(LexCode::InvalidFloat),
                        "float literal out of range",
                        span,
                    );
                    (Token::Float(0.0), span)
                }
            };
        }

        let range = self.current_span(start);
        let span = self.insert_span(range);
        if had_underscore {
            // lex-invalid-integer
            self.push_error(
                DiagnosticCode::Lex(LexCode::InvalidInteger),
                "integer literal has invalid underscore placement",
                span,
            );
        }
        match value {
            Some(v) => (Token::Int(v), span),
            None => {
                // lex-invalid-integer
                self.push_error(
                    DiagnosticCode::Lex(LexCode::InvalidInteger),
                    "integer literal out of range",
                    span,
                );
                (Token::Int(I256::from(0)), span)
            }
        }
    }

    /// Consumes a digit run of `radix`, continuing from `value`.
    ///
    /// Returns the accumulated value (`None` after overflow) and whether an
    /// underscore violates placement (consecutive or trailing).
    fn consume_digits(&mut self, radix: u8, mut value: Option<I256>) -> (Option<I256>, bool) {
        let bytes = self.bytes;
        let mut had_underscore = false;
        let mut last_was_underscore = false;
        while self.pos < bytes.len() {
            let byte = bytes[self.pos];
            let digit = match byte {
                b'0'..=b'9' => u64::from(byte - b'0'),
                b'a'..=b'f' => u64::from(byte - b'a' + 10),
                b'A'..=b'F' => u64::from(byte - b'A' + 10),
                b'_' => {
                    if last_was_underscore {
                        had_underscore = true;
                    }
                    last_was_underscore = true;
                    self.pos += 1;
                    continue;
                }
                _ => break,
            };
            if digit >= u64::from(radix) {
                break;
            }
            if let Some(current) = value {
                value = current
                    .checked_mul(I256::from(radix))
                    .and_then(|current| current.checked_add(I256::from(digit as u8)));
            }
            last_was_underscore = false;
            self.pos += 1;
        }
        if last_was_underscore {
            had_underscore = true;
        }
        (value, had_underscore)
    }

    // TODO: Implement raw/multiline string literals with `\\` prefix syntax (Zig-style).
    //
    // Design: Each line of a raw string is a separate token prefixed with `\\`.
    // This means `\n` is always whitespace and the lexer can operate line-by-line.
    // It avoids the indentation problems of Rust's `r##""##` and nesting issues
    // with raw string delimiters. Example:
    //     const raw =
    //         \\Roses are red
    //         \\  Violets are blue
    //         \\
    //     ;
    // Key properties: no escape sequences needed, `\\` itself doesn't need escaping,
    // each line is a separate lexer token, and unclosed raw strings can't corrupt
    // subsequent lexical structure.
    // Ref: https://matklad.github.io/2025/08/09/zigs-lovely-syntax.html#String-Literals

    fn lex_string(&mut self, start: usize) -> (Token<'src>, SpanId) {
        let bytes = self.bytes;

        let end = memchr2(b'\'', b'\n', &bytes[self.pos..]).map_or(bytes.len(), |i| self.pos + i);

        if end < bytes.len() && bytes[end] == b'\'' {
            self.pos = end + 1;
            let text = self.slice_from(start);
            let range = self.current_span(start);
            let span = self.insert_span(range);
            return (Token::String(&text[1..text.len() - 1]), span);
        }

        self.pos = end;
        let text = self.slice_from(start);
        let range = self.current_span(start);
        let span = self.insert_span(range);
        // lex-unclosed-string
        self.push_error(
            DiagnosticCode::Lex(LexCode::UnclosedString),
            "unclosed string literal",
            span,
        );
        (Token::UnterminatedString(&text[1..]), span)
    }

    fn lex_format_string(&mut self, open_span: SpanId) {
        let mut tokens = match self.fmt.take() {
            Some(state) => {
                let mut v = state.tokens;
                v.clear();
                v
            }
            None => Vec::new(),
        };

        tokens.push((Token::FormatStringDelim, open_span));

        loop {
            let text_start = self.pos;
            let bytes = self.bytes;

            self.ensure_line_end();

            let special = if self.pos < self.line_end {
                memchr2(b'"', b'\\', &bytes[self.pos..self.line_end])
            } else {
                None
            };

            self.pos = match special {
                Some(i) => self.pos + i,
                None => self.line_end,
            };

            if self.pos > text_start {
                let span = self.insert_span(text_start..self.pos);
                tokens.push((Token::FormatStringText(self.slice_from(text_start)), span));
            }

            match self.peek() {
                Some(b'"') => {
                    let quote_start = self.pos;
                    self.pos += 1;
                    let span = self.insert_span(quote_start..self.pos);
                    tokens.push((Token::FormatStringDelim, span));
                    break;
                }
                Some(b'\\') => {
                    let esc_start = self.pos;
                    self.pos += 1;
                    if self.peek() == Some(b'(') {
                        self.pos += 1;
                        let span = self.insert_span(esc_start..self.pos);
                        tokens.push((Token::FormatInterpStart, span));
                        self.lex_format_interp(&mut tokens);
                    } else if let Some(character) = self.source[self.pos..].chars().next() {
                        self.pos += character.len_utf8();
                        let span = self.insert_span(esc_start..self.pos);
                        tokens.push((Token::FormatStringText(self.slice_from(esc_start)), span));
                    }
                }
                Some(b'(') => {
                    let interp_start = self.pos;
                    self.pos += 1;
                    let span = self.insert_span(interp_start..self.pos);
                    tokens.push((Token::FormatInterpStart, span));
                    self.lex_format_interp(&mut tokens);
                }
                _ => {
                    let unterm_start = self.pos;
                    if self.peek() == Some(b'\n') {
                        self.pos += 1;
                    }
                    let span = self.insert_span(unterm_start..self.pos);
                    tokens.push((Token::UnterminatedFormatString, span));
                    // lex-unclosed-string
                    self.push_error(
                        DiagnosticCode::Lex(LexCode::UnclosedString),
                        "unclosed string literal",
                        open_span,
                    );
                    break;
                }
            }
        }

        self.fmt = Some(FormatStringState { tokens, idx: 0 });
    }

    fn lex_format_interp(&mut self, tokens: &mut Vec<(Token<'src>, SpanId)>) {
        let mut paren_depth: u32 = 0;

        loop {
            match self.lex_simple_token() {
                None => {
                    let eof_span = self.insert_span(self.pos..self.pos);
                    tokens.push((Token::UnterminatedFormatString, eof_span));
                    // lex-unclosed-string
                    self.push_error(
                        DiagnosticCode::Lex(LexCode::UnclosedString),
                        "unclosed string literal",
                        eof_span,
                    );
                    return;
                }
                Some((Token::ParenClose, span)) => {
                    if paren_depth == 0 {
                        tokens.push((Token::FormatInterpEnd, span));
                        return;
                    }
                    paren_depth -= 1;
                    tokens.push((Token::ParenClose, span));
                }
                Some((Token::FormatStringDelim, span)) => {
                    tokens.push((Token::UnterminatedFormatString, span));
                    // lex-unclosed-string
                    self.push_error(
                        DiagnosticCode::Lex(LexCode::UnclosedString),
                        "unclosed string literal",
                        span,
                    );
                    return;
                }
                Some((Token::Newline | Token::Indent | Token::Dedent, _)) => {}
                Some((Token::ParenOpen, span)) => {
                    paren_depth += 1;
                    tokens.push((Token::ParenOpen, span));
                }
                Some(tok) => {
                    tokens.push(tok);
                }
            }
        }
    }

    fn lex_comment(
        &mut self,
        start: usize,
        prefix_len: usize,
        kind: CommentKind,
    ) -> (Token<'src>, SpanId) {
        self.pos += prefix_len;
        self.ensure_line_end();
        self.pos = self.line_end;
        let text = self.slice_from(start);
        let range = self.current_span(start);
        let span = self.insert_span(range);

        match kind {
            CommentKind::Line => (Token::Comment(text), span),
            CommentKind::Doc => (Token::DocComment(text), span),
            CommentKind::ModuleDoc => (Token::ModuleDocComment(text), span),
        }
    }

    fn lex_two_char(
        &mut self,
        start: usize,
        second: u8,
        two_char: Token<'src>,
        fallback: Token<'src>,
    ) -> Option<(Token<'src>, SpanId)> {
        if self.peek() == Some(second) {
            self.pos += 1;
            Some((two_char, self.insert_span(self.current_span(start))))
        } else {
            Some((fallback, self.insert_span(self.current_span(start))))
        }
    }

    fn single_char_token(b: u8) -> Option<Token<'src>> {
        Some(match b {
            b'+' => Token::Plus,
            b'*' => Token::Star,
            b'%' => Token::Percent,
            b'\\' => Token::SlashOr,
            b'^' => Token::Caret,
            b'|' => Token::Pipe,
            b'@' => Token::At,
            b'#' => Token::Pound,
            b'?' => Token::Question,
            b';' => Token::ColonSemi,
            b'(' => Token::ParenOpen,
            b')' => Token::ParenClose,
            b'[' => Token::BracketOpen,
            b']' => Token::BracketClose,
            b'{' => Token::CurlyOpen,
            b'}' => Token::CurlyClose,
            b'&' => Token::Ampersand,
            b',' => Token::Comma,
            _ => return None,
        })
    }

    fn lex_simple_token(&mut self) -> Option<(Token<'src>, SpanId)> {
        loop {
            self.skip_inline_whitespace();

            let start = self.pos;
            let b = self.advance()?;

            if let Some(tok) = Self::single_char_token(b) {
                return Some((tok, self.insert_span(self.current_span(start))));
            }

            match b {
                b'\n' => return self.handle_newline(start),

                b'a'..=b'z' | b'_' => return Some(self.lex_keyword_or_id(start)),
                b'A'..=b'Z' => return Some(self.lex_tag(start)),
                b'0'..=b'9' => return Some(self.lex_number(start)),

                b'\'' => return Some(self.lex_string(start)),
                b'"' => {
                    let span = self.insert_span(self.current_span(start));
                    return Some((Token::FormatStringDelim, span));
                }

                b'-' => {
                    return match (self.peek(), self.peek_at(1)) {
                        (Some(b'-'), Some(b'|')) => {
                            Some(self.lex_comment(start, 2, CommentKind::ModuleDoc))
                        }
                        (Some(b'-'), Some(b'-')) => {
                            Some(self.lex_comment(start, 2, CommentKind::Doc))
                        }
                        (Some(b'-'), _) => Some(self.lex_comment(start, 1, CommentKind::Line)),
                        _ => Some((Token::Minus, self.insert_span(self.current_span(start)))),
                    };
                }

                b'<' => {
                    return match self.peek() {
                        Some(b'<') => {
                            self.pos += 1;
                            Some((Token::ShiftLeft, self.insert_span(self.current_span(start))))
                        }
                        Some(b'=') => {
                            self.pos += 1;
                            Some((Token::LessEq, self.insert_span(self.current_span(start))))
                        }
                        Some(b'-') => {
                            self.pos += 1;
                            Some((Token::ArrowLeft, self.insert_span(self.current_span(start))))
                        }
                        _ => Some((Token::Less, self.insert_span(self.current_span(start)))),
                    };
                }

                b'>' => {
                    return match self.peek() {
                        Some(b'>') => {
                            self.pos += 1;
                            Some((
                                Token::ShiftRight,
                                self.insert_span(self.current_span(start)),
                            ))
                        }
                        Some(b'=') => {
                            self.pos += 1;
                            Some((Token::GreaterEq, self.insert_span(self.current_span(start))))
                        }
                        _ => Some((Token::Greater, self.insert_span(self.current_span(start)))),
                    };
                }

                b'=' => return self.lex_two_char(start, b'=', Token::EqEq, Token::Eq),
                b'/' => return self.lex_two_char(start, b'=', Token::NotEq, Token::Slash),
                b':' => {
                    return match self.peek() {
                        Some(b':') => {
                            self.lex_two_char(start, b':', Token::ColonColon, Token::Colon)
                        }
                        _ => self.lex_two_char(start, b'=', Token::ColonEq, Token::Colon),
                    };
                }

                b'.' => {
                    return {
                        if self.peek() == Some(b'.') && self.peek_at(1) == Some(b'.') {
                            self.pos += 2;
                            Some((Token::Infer, self.insert_span(self.current_span(start))))
                        } else {
                            Some((Token::Dot, self.insert_span(self.current_span(start))))
                        }
                    };
                }

                _ => {
                    while self.peek().is_some_and(|b| b & 0xC0 == 0x80) {
                        self.pos += 1;
                    }
                    let range = self.current_span(start);
                    let span = self.insert_span(range);
                    // lex-unexpected-character
                    self.push_error(
                        DiagnosticCode::Lex(LexCode::UnexpectedCharacter),
                        "unexpected character",
                        span,
                    );
                    continue;
                }
            }
        }
    }

    fn lex_single_token(&mut self) -> Option<(Token<'src>, SpanId)> {
        let tok = self.lex_simple_token()?;
        if matches!(tok.0, Token::FormatStringDelim) {
            self.lex_format_string(tok.1);
            self.fmt.as_mut().and_then(|s| s.next())
        } else {
            Some(tok)
        }
    }

    fn next_with_indent(&mut self) -> Option<(Token<'src>, SpanId)> {
        if let Some(ref mut state) = self.fmt
            && let Some(item) = state.next()
        {
            return Some(item);
        }

        if self.indent.pending_dedents > 0 {
            self.indent.pending_dedents -= 1;
            return Some((Token::Dedent, self.last_indent_span));
        }
        if self.indent.pending_indent {
            self.indent.pending_indent = false;
            return Some((Token::Indent, self.last_indent_span));
        }

        let item = self.lex_single_token()?;
        if self.indent.indent_overflow {
            self.indent.indent_overflow = false;
            // lex-overflow-indent
            self.push_error(
                DiagnosticCode::Lex(LexCode::OverflowIndent),
                "indentation overflow",
                item.1,
            );
        }
        Some(item)
    }

    fn next_token(&mut self) -> Option<(Token<'src>, SpanId)> {
        if let Some(tok) = self.next_with_indent() {
            return Some(tok);
        }

        self.last_indent_span = self.insert_span(self.pos..self.pos);
        let dedent_count = self.indent.indent_depth as usize - 1;
        if dedent_count > 0 {
            self.indent.indent_depth = 1;
            self.indent.pending_dedents = dedent_count as u8;
            self.next_with_indent()
        } else {
            None
        }
    }

    pub fn next_raw(&mut self) -> Option<(Token<'src>, SpanId)> {
        self.next_token()
    }
}

impl<'src> Iterator for Lexer<'src> {
    type Item = (Token<'src>, SpanId);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let item = self.next_raw()?;
            if !matches!(item.0, Token::Comment(_)) {
                return Some(item);
            }
        }
    }
}
