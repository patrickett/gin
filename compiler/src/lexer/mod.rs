pub mod source_file;

use std::collections::VecDeque;

use crate::{
    expr::literal::Literal,
    token::{Keyword, Token},
};

use self::source_file::SourceFile;

// TODO: when done lexing
// empty vec content and remove full_path

#[derive(Debug, Clone)]
pub struct Lexer {
    line: usize,
    line_position: usize,

    content: Vec<char>,
    content_index: usize,
    full_path: String,

    buffer: String,
    queue: VecDeque<Token>,
}

impl Lexer {
    pub const fn new() -> Self {
        Self {
            line: 1,
            line_position: 0,
            buffer: String::new(),
            full_path: String::new(),
            content: Vec::new(),
            content_index: 0,
            queue: VecDeque::new(),
        }
    }

    pub fn location(&self) -> String {
        format!("{}:{}", self.full_path, self.pos())
    }

    /// Colon seperated line:column
    ///
    /// ex `3:6`
    fn pos(&self) -> String {
        format!("{}:{}", self.line, self.line_position)
    }

    pub fn saw_newline(&mut self) {
        self.line += 1;
        // goes back to start
        self.line_position = 0;
    }

    pub fn set_content(&mut self, source_file: &SourceFile) {
        self.content = source_file.content().chars().collect();
        self.full_path = source_file.full_path().to_owned();
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
        self.content_index += 1;
        self.line_position += 1;
        self.content.get(self.content_index - 1).map(|&c| c)
    }

    fn resolve_comment(&mut self) {
        self.resolve_buffer();
        let mut is_doc_comment = false;
        let second_char = self.next_char();
        if let Some('#') = second_char {
            is_doc_comment = true;
        } else if let Some(c) = second_char {
            self.buffer.push(c)
        }

        while let Some(c) = self.next_char() {
            match c {
                '\r' | '\n' => {
                    self.saw_newline();
                    let comment_text = self.buffer.trim().to_string();
                    let comment = match is_doc_comment {
                        true => Token::DocComment(comment_text),
                        false => Token::Comment(comment_text),
                    };
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
            }
            if let Some(c) = self.next_char() {
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
                    '#' => self.resolve_comment(),
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
