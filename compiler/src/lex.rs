use std::collections::VecDeque;

use crate::{
    expr::literal::Literal,
    token::{Keyword, Token},
};

#[derive(Clone)]
pub struct Lexer {
    token_count: usize,
    position: usize,
    buffer: String,
    queue: VecDeque<Token>,
    source_content: String,
}

impl Lexer {
    pub const fn new() -> Self {
        Self {
            token_count: 0,
            position: 0,
            buffer: String::new(),
            source_content: String::new(),
            queue: VecDeque::new(),
        }
    }

    pub fn pos(&self) -> usize {
        self.position
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

    fn resolve_buffer_then_add(&mut self, tok: Token) {
        self.resolve_buffer();
        self.queue.push_back(tok);
    }

    fn resolve_buffer(&mut self) {
        if !self.buffer.is_empty() {
            let tok = match self.buffer.as_str() {
                "+" => Token::Plus,
                "-" => Token::Dash,
                "=" => Token::Equals,
                "&" => Token::Ampersand,
                "*" => Token::Star,
                "/" => Token::SlashForward,
                "%" => Token::Percent,
                "<" => Token::LessThan,
                "<=" => Token::LessThanOrEqualTo,
                ">" => Token::GreaterThan,
                ">=" => Token::GreaterThanOrEqualTo,
                "->" => Token::RightArrow,
                "<-" => Token::LeftArrow,
                "include" => Token::Keyword(Keyword::Include),
                "true" => Token::Literal(Literal::Bool(true)),
                "false" => Token::Literal(Literal::Bool(false)),
                "return" => Token::Keyword(Keyword::Return),
                "for" => Token::Keyword(Keyword::For),
                "if" => Token::Keyword(Keyword::If),
                "else" => Token::Keyword(Keyword::Else),
                id => {
                    if id.chars().all(|c| c.is_digit(10)) {
                        let num: Result<usize, _> = id.parse();
                        if let Ok(n) = num {
                            Token::Literal(Literal::Number(n))
                        } else {
                            Token::Id(id.to_string())
                        }
                    } else {
                        Token::Id(id.to_string())
                    }
                }
            };

            self.buffer.clear();
            self.queue.push_back(tok);
        }
    }

    fn next_char(&mut self) -> Option<char> {
        if self.position > self.source_content.len() {
            return None;
        }
        let next_char = self.source_content.chars().nth(self.position);
        self.position += 1;
        next_char
    }

    fn resolve_comment(&mut self) {
        self.resolve_buffer();
        loop {
            match self.next_char() {
                Some('\r' | '\n') | None => {
                    let comment_text = self.buffer.clone().to_string();
                    let comment = Token::Comment(comment_text);
                    self.add(comment);
                    break;
                }
                Some(c) => self.buffer.push(c),
            }
        }
    }

    fn resolve_doc_comment(&mut self) {
        self.resolve_buffer();
        loop {
            match self.next_char() {
                Some('\r' | '\n') | None => {
                    let comment_text = self.buffer.clone().to_string();
                    let comment = Token::Comment(comment_text);
                    self.add(comment);
                    break;
                }
                Some(c) => self.buffer.push(c),
            }
        }
    }

    fn resolve_template_string(&mut self) {
        self.resolve_buffer();
        loop {
            match self.next_char() {
                Some('`') | None => break,
                Some(c) => self.buffer.push(c),
            }
        }

        let string_text = self.buffer.clone().to_string();
        let string = Token::Literal(Literal::TemplateString(string_text));
        self.add(string);
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
        let string = Token::Literal(Literal::String(string_text));
        self.add(string);
    }
}

impl Iterator for Lexer {
    type Item = Token;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.queue.len() > 0 {
                self.token_count += 1;
                let x = self.queue.pop_front();
                // println!("{:#?}", x);
                return x;
            }

            if let Some(c) = self.next_char() {
                match c {
                    ' ' => self.resolve_buffer_then_add(Token::Space),
                    ':' => self.resolve_buffer_then_add(Token::Colon),
                    '\t' => self.resolve_buffer_then_add(Token::Tab),
                    ',' => self.resolve_buffer_then_add(Token::Comma),
                    ';' => self.resolve_buffer_then_add(Token::SemiColon),
                    '\n' => self.resolve_buffer_then_add(Token::Newline),
                    '[' => self.resolve_buffer_then_add(Token::BracketOpen),
                    ']' => self.resolve_buffer_then_add(Token::BracketClose),
                    '{' => self.resolve_buffer_then_add(Token::CurlyOpen),
                    '}' => self.resolve_buffer_then_add(Token::CurlyClose),
                    '#' => {
                        let peek = self.source_content.chars().nth(self.position);
                        if let Some('#') = peek {
                            self.resolve_doc_comment()
                        } else {
                            self.resolve_comment()
                        }
                    }
                    '"' => self.resolve_string(),
                    '`' => self.resolve_template_string(),
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
