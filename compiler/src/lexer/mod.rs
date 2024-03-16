pub mod source_file;

use std::collections::VecDeque;

use crate::{
    expr::literal::Literal,
    lexer::source_file::SourceFile,
    token::{Keyword, Token},
};

#[derive(Debug, Clone)]
pub struct Lexer {
    file_content: Option<SourceFile>,
    line: usize,
    line_position: usize,
    // char_index: usize,
    buffer: String,
    queue: VecDeque<Token>,
    // content: String,
}

impl Lexer {
    pub const fn new() -> Self {
        Self {
            file_content: None,
            line: 1,
            line_position: 0,
            // char_index: 0,
            buffer: String::new(),
            // content: String::new(),
            queue: VecDeque::new(),
        }
    }

    /// Colon seperated line:column
    ///
    /// ex `3:6`
    pub fn pos(&self) -> String {
        format!("{}:{}", self.line, self.line_position)
    }

    pub fn saw_newline(&mut self) {
        self.line += 1;
        // goes back to start
        self.line_position = 0;
    }

    pub fn set_content(&mut self, sf: SourceFile) {
        self.file_content = Some(sf);
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
        let fc = self
            .file_content
            .as_mut()
            .expect("should have file_content set");
        self.line_position += 1;
        fc.next()
    }

    fn peek_char(&mut self) -> Option<char> {
        let fc = self
            .file_content
            .as_mut()
            .expect("should have file_content set");
        fc.nth(fc.index() + 1)
    }

    fn resolve_comment(&mut self) {
        self.resolve_buffer();
        while let Some(c) = self.next_char() {
            match c {
                '\n' | '\r' => {
                    self.saw_newline();
                    let comment_text = self.buffer.clone().to_string();
                    let comment = Token::Comment(comment_text);
                    self.add(comment);
                    break;
                }
                c => self.buffer.push(c),
            }
        }
    }

    fn resolve_doc_comment(&mut self) {
        self.resolve_buffer();
        while let Some(c) = self.next_char() {
            match c {
                '\n' | '\r' => {
                    self.saw_newline();
                    let comment_text = self.buffer.clone().to_string();
                    let comment = Token::DocComment(comment_text);
                    self.add(comment);
                    break;
                }
                c => self.buffer.push(c),
            }
        }
    }

    fn resolve_template_string(&mut self) {
        self.resolve_buffer();
        while let Some(c) = self.next_char() {
            match c {
                '`' => break,
                c => self.buffer.push(c),
            }
        }
        let string = Token::Literal(Literal::TemplateString(self.buffer.to_string()));
        self.add(string);
    }

    fn resolve_string(&mut self) {
        self.resolve_buffer();
        while let Some(c) = self.next_char() {
            match c {
                '"' => break,
                c => self.buffer.push(c),
            }
        }
        let string = Token::Literal(Literal::String(self.buffer.to_string()));
        self.add(string);
    }
}

impl Iterator for Lexer {
    type Item = Token;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.queue.len() > 0 {
                return self.queue.pop_front();
            } else if let Some(c) = self.next_char() {
                match c {
                    '+' => self.resolve_buffer_then_add(Token::Plus),
                    '-' => self.resolve_buffer_then_add(Token::Dash),
                    '*' => self.resolve_buffer_then_add(Token::Star),
                    '/' => self.resolve_buffer_then_add(Token::SlashForward),
                    ' ' => self.resolve_buffer_then_add(Token::Space),
                    ':' => self.resolve_buffer_then_add(Token::Colon),
                    '\t' => self.resolve_buffer_then_add(Token::Tab),
                    ',' => self.resolve_buffer_then_add(Token::Comma),
                    ';' => self.resolve_buffer_then_add(Token::SemiColon),
                    '\n' => {
                        self.saw_newline();
                        self.resolve_buffer_then_add(Token::Newline)
                    }
                    '[' => self.resolve_buffer_then_add(Token::BracketOpen),
                    ']' => self.resolve_buffer_then_add(Token::BracketClose),
                    '{' => self.resolve_buffer_then_add(Token::CurlyOpen),
                    '}' => self.resolve_buffer_then_add(Token::CurlyClose),
                    '#' => {
                        if let Some('#') = self.peek_char() {
                            self.resolve_doc_comment()
                        } else {
                            self.resolve_comment()
                        }
                    }
                    '"' => self.resolve_string(),
                    '`' => self.resolve_template_string(),
                    ch => self.buffer.push(ch),
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
