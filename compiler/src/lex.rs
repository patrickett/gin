use std::{collections::VecDeque, iter::Peekable, str::Chars};

use crate::token::{Keyword, Literal, Token, TokenKind};

#[derive(Clone)]
pub struct Lexer {
    token_count: usize,
    source_index: usize,
    buffer: String,
    queue: VecDeque<Token>,
    source_content: String,
}

impl Lexer {
    pub fn new() -> Self {
        Self {
            token_count: 0,
            source_index: 0,
            buffer: String::new(),
            source_content: String::new(),
            queue: VecDeque::new(),
        }
    }

    pub fn set_source_content(&mut self, source_content: String) {
        self.source_content = source_content;
    }

    pub fn return_to_queue(&mut self, t: Token) {
        self.queue.push_front(t)
    }

    fn add(&mut self, tok: Token) {
        self.buffer.clear();
        self.queue.push_back(tok);
    }

    fn resolve_buffer_then_add(&mut self, tok: TokenKind) {
        self.resolve_buffer();
        self.queue.push_back(Token::new(tok, self.source_index));
    }

    fn resolve_buffer(&mut self) {
        if !self.buffer.is_empty() {
            let tok = match self.buffer.as_str() {
                "+" => TokenKind::Plus,
                "-" => TokenKind::Dash,
                "=" => TokenKind::Equals,
                "&" => TokenKind::Ampersand,
                "*" => TokenKind::Ampersand,
                "/" => TokenKind::Ampersand,
                "%" => TokenKind::Ampersand,
                "<" => TokenKind::Ampersand,
                ">" => TokenKind::Ampersand,
                "->" => TokenKind::Ampersand,
                "return" => TokenKind::Keyword(Keyword::Return),
                "for" => TokenKind::Keyword(Keyword::For),
                "if" => TokenKind::Keyword(Keyword::If),
                "else" => TokenKind::Keyword(Keyword::Else),
                id => {
                    if id.chars().all(|c| c.is_digit(10)) {
                        let num: Result<usize, _> = id.parse();
                        if let Ok(n) = num {
                            TokenKind::Literal(Literal::Number(n))
                        } else {
                            TokenKind::Id(id.to_string())
                        }
                    } else {
                        TokenKind::Id(id.to_string())
                    }
                }
            };

            self.buffer.clear();
            self.queue.push_back(Token::new(tok, self.source_index));
        }
    }

    fn next_char(&mut self) -> Option<char> {
        if self.source_index < self.source_content.len() {
            self.source_index += 1;
            self.source_content.chars().nth(self.source_index - 1)
        } else {
            None
        }
    }

    fn resolve_comment(&mut self) {
        self.resolve_buffer();
        loop {
            match self.next_char() {
                Some('\r' | '\n') | None => break,
                Some(c) => self.buffer.push(c),
            }
        }

        let comment_text = self.buffer.clone().to_string();
        let comment = Token::new(TokenKind::Comment(comment_text), self.source_index);
        self.add(comment);
    }

    fn resolve_string(&mut self) {
        self.resolve_buffer();
        loop {
            match self.next_char() {
                Some('"') | None => break,
                Some(c) => self.buffer.push(c),
            }
        }

        let string_text = self.buffer.clone().to_string();
        let string = Token::new(
            TokenKind::Literal(Literal::String(string_text)),
            self.source_index,
        );
        self.add(string);
    }
}

impl Iterator for Lexer {
    type Item = Token;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.queue.len() > 0 {
                self.token_count += 1;
                return self.queue.pop_front();
            }

            if let Some(c) = self.next_char() {
                match c {
                    ' ' => self.resolve_buffer_then_add(TokenKind::Space),
                    ':' => self.resolve_buffer_then_add(TokenKind::Colon),
                    '\t' => self.resolve_buffer_then_add(TokenKind::Tab),
                    ',' => self.resolve_buffer_then_add(TokenKind::Comma),
                    ';' => self.resolve_buffer_then_add(TokenKind::SemiColon),
                    '\n' => self.resolve_buffer_then_add(TokenKind::Newline),
                    '{' => self.resolve_buffer_then_add(TokenKind::CurlyOpen),
                    '}' => self.resolve_buffer_then_add(TokenKind::CurlyClose),
                    '#' => self.resolve_comment(),
                    '"' => self.resolve_string(),
                    ch => {
                        self.buffer.push(ch);
                    }
                }
            } else {
                if !self.buffer.is_empty() {
                    self.resolve_buffer();
                    continue;
                }

                return None;
            }
        }
    }
}
