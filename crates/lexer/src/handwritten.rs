//! Byte-oriented raw scanning; all loops advance within the input.
use crate::{Token, TokenKind};
use memchr::{memchr, memchr2};
use span::Span;

/// Allocation-free scanner with no conversion, diagnostic, or layout state.
pub struct Lexer<'src> {
    source: &'src str,
    bytes: &'src [u8],
    pos: usize,
}

impl<'src> Lexer<'src> {
    /// # Panics
    /// Panics when source offsets cannot be represented by `span::Span`.
    pub fn new(source: &'src str) -> Self {
        assert!(
            u32::try_from(source.len()).is_ok(),
            "source exceeds span range"
        );
        Self {
            source,
            bytes: source.as_bytes(),
            pos: 0,
        }
    }

    /// Resume scanning at `position`, which must not be behind the scanner and must be a character
    /// boundary. A consumer that reads the text of a format string itself, which is no business
    /// of a raw scanner, uses this to step over it.
    ///
    /// # Panics
    /// Panics when `position` is behind the scanner, past the source, or inside a character.
    pub fn skip_to(&mut self, position: usize) {
        assert!(position >= self.pos && position <= self.source.len());
        assert!(self.source.is_char_boundary(position));
        self.pos = position;
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    fn lex_name(&mut self, start: usize, tag: bool) -> TokenKind {
        assert_eq!(self.pos, start + 1);
        assert!(self.bytes[start].is_ascii_alphabetic() || self.bytes[start] == b'_');
        if tag {
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_alphanumeric() {
                self.pos += 1;
            }
        } else {
            while self.pos < self.bytes.len() {
                let byte = self.bytes[self.pos];
                if !byte.is_ascii_alphanumeric() && byte != b'_' {
                    break;
                }
                self.pos += 1;
            }
        }
        TokenKind::from_keyword(&self.source[start..self.pos]).unwrap_or(if tag {
            TokenKind::Tag
        } else {
            TokenKind::Id
        })
    }

    /// The preceding byte is a digit; underscores must separate digits.
    fn consume_digits(&mut self, hex: bool) -> bool {
        assert!(self.pos > 0);
        assert!(self.bytes[self.pos - 1].is_ascii_hexdigit());
        let mut malformed = false;
        let mut underscore = false;
        while self.pos < self.bytes.len() {
            let byte = self.bytes[self.pos];
            if byte == b'_' {
                malformed |= underscore;
                underscore = true;
            } else if byte.is_ascii_digit() || (hex && byte.is_ascii_hexdigit()) {
                underscore = false;
            } else {
                break;
            }
            self.pos += 1;
        }
        malformed || underscore
    }

    fn lex_number(&mut self, start: usize) -> TokenKind {
        assert_eq!(self.pos, start + 1);
        assert!(self.bytes[start].is_ascii_digit());
        let hex = self.bytes[start] == b'0'
            && matches!(self.peek(), Some(b'x' | b'X'))
            && self.peek_at(1).is_some_and(|byte| byte.is_ascii_hexdigit());
        if hex {
            self.pos += 2; // Prefix and first guaranteed digit.
        }
        let malformed = self.consume_digits(hex);
        if !hex
            && self.peek() == Some(b'.')
            && self.peek_at(1).is_some_and(|byte| byte.is_ascii_digit())
        {
            self.pos += 2;
            let fraction_malformed = self.consume_digits(false);
            if malformed || fraction_malformed {
                TokenKind::MalformedFloat
            } else {
                TokenKind::Float
            }
        } else if malformed {
            TokenKind::MalformedInt
        } else {
            TokenKind::Int
        }
    }

    fn lex_string(&mut self) -> TokenKind {
        let end = memchr2(b'\'', b'\n', &self.bytes[self.pos..])
            .map_or(self.bytes.len(), |index| self.pos + index);
        if end < self.bytes.len() && self.bytes[end] == b'\'' {
            self.pos = end + 1;
            TokenKind::String
        } else {
            self.pos = end;
            TokenKind::UnterminatedString
        }
    }

    fn lex_comment(&mut self) -> TokenKind {
        let marker = self.peek_at(1);
        self.pos = memchr(b'\n', &self.bytes[self.pos..])
            .map_or(self.bytes.len(), |index| self.pos + index);
        match marker {
            Some(b'|') => TokenKind::ModuleDocComment,
            Some(b'-') => TokenKind::DocComment,
            _ => TokenKind::Comment,
        }
    }

    fn single_char_token(byte: u8) -> Option<TokenKind> {
        Some(match byte {
            b'\n' => TokenKind::Newline,
            b'"' => TokenKind::FormatStringDelim,
            b'+' => TokenKind::Plus,
            b'*' => TokenKind::Star,
            b'%' => TokenKind::Percent,
            b'\\' => TokenKind::SlashOr,
            b'^' => TokenKind::Caret,
            b'|' => TokenKind::Pipe,
            b'@' => TokenKind::At,
            b'#' => TokenKind::Pound,
            b'?' => TokenKind::Question,
            b';' => TokenKind::ColonSemi,
            b'(' => TokenKind::ParenOpen,
            b')' => TokenKind::ParenClose,
            b'[' => TokenKind::BracketOpen,
            b']' => TokenKind::BracketClose,
            b'{' => TokenKind::CurlyOpen,
            b'}' => TokenKind::CurlyClose,
            b'&' => TokenKind::Ampersand,
            b',' => TokenKind::Comma,
            _ => return None,
        })
    }

    fn lex_operator(&mut self, byte: u8) -> Option<TokenKind> {
        let pair = match (byte, self.peek()) {
            (b'<', Some(b'<')) => Some(TokenKind::ShiftLeft),
            (b'<', Some(b'=')) => Some(TokenKind::LessEq),
            (b'<', Some(b'-')) => Some(TokenKind::ArrowLeft),
            (b'>', Some(b'>')) => Some(TokenKind::ShiftRight),
            (b'>', Some(b'=')) => Some(TokenKind::GreaterEq),
            (b'=', Some(b'=')) => Some(TokenKind::EqEq),
            (b'/', Some(b'=')) => Some(TokenKind::NotEq),
            (b':', Some(b':')) => Some(TokenKind::ColonColon),
            (b':', Some(b'=')) => Some(TokenKind::ColonEq),
            _ => None,
        };
        if let Some(kind) = pair {
            self.pos += 1;
            return Some(kind);
        }
        Some(match byte {
            b'<' => TokenKind::Less,
            b'>' => TokenKind::Greater,
            b'=' => TokenKind::Eq,
            b'/' => TokenKind::Slash,
            b':' => TokenKind::Colon,
            b'-' => TokenKind::Minus,
            b'.' => {
                if self.peek() == Some(b'.') && self.peek_at(1) == Some(b'.') {
                    self.pos += 2;
                    TokenKind::Infer
                } else {
                    TokenKind::Dot
                }
            }
            _ => return None,
        })
    }

    /// Return the next raw token, including comments and every newline.
    pub fn next_raw(&mut self) -> Option<Token> {
        assert!(self.pos <= self.source.len());
        assert!(self.source.is_char_boundary(self.pos));
        let before = self.pos;
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.pos += 1;
        }
        let start = self.pos;
        let byte = self.peek()?;
        self.pos += 1;
        let kind = match byte {
            b'a'..=b'z' | b'_' => self.lex_name(start, false),
            b'A'..=b'Z' => self.lex_name(start, true),
            b'0'..=b'9' => self.lex_number(start),
            b'\'' => self.lex_string(),
            b'-' if self.peek() == Some(b'-') => self.lex_comment(),
            _ => {
                if let Some(kind) = Self::single_char_token(byte) {
                    kind
                } else if let Some(kind) = self.lex_operator(byte) {
                    kind
                } else {
                    let scalar = self.source[start..].chars().next().expect("nonempty input");
                    self.pos = start + scalar.len_utf8();
                    TokenKind::Unknown
                }
            }
        };
        assert!(self.pos > before);
        assert!(start < self.pos);
        assert!(self.pos <= self.source.len());
        assert!(self.source.is_char_boundary(start));
        assert!(self.source.is_char_boundary(self.pos));
        let span = Span::from_range(start..self.pos);
        assert!(!span.is_empty());
        assert_eq!(span.end(), self.pos);
        Some(Token { kind, span })
    }
}

impl Iterator for Lexer<'_> {
    type Item = Token;
    fn next(&mut self) -> Option<Self::Item> {
        self.next_raw()
    }
}
