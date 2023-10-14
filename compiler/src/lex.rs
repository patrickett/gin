use std::{iter::Peekable, str::Chars};

#[derive(Debug, PartialEq, Clone)]
pub enum Keyword {
    If,
    Else,
    For,
    Return,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Literal {
    String(String),
    Number(usize)
}


#[derive(Debug, PartialEq, Clone)]
pub enum Token {
    ParenOpen,
    ParenClose,
    CurlyOpen,
    CurlyClose,
    SlashBack,
    SlashForward,
    Colon,
    SemiColon,
    Comma,
    Tab,
    Space,
    Newline,
    // LineReturn,
    Comment(String),
    Id(String),
    // Number(usize),
    // String(String),

    Literal(Literal),
    LessThan,
    GreaterThan,
    RightArrow,

    Plus,
    Dash,
    Equals,
    Ampersand,
    Star,
    Percent,
    Keyword(Keyword),
}

// TODO: add { start: usize, end: usize } for LSP in the future
pub struct Lexer {
    buffer: String,
    pub tokens: Vec<Token>,
}

impl Lexer {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            tokens: Vec::new(),
        }
    }

    fn add(&mut self, tok: Token) {
        self.buffer.clear();
        self.tokens.push(tok);
    }

    fn resolve_buffer_then_add(&mut self, src: &mut Peekable<Chars>, tok: Token) {
        self.resolve_buffer(src);
        self.add(tok);
    }

    fn next(&mut self, src: &mut Peekable<Chars>)  {
        let next = src.next();
        if let Some(c) = next {
            self.buffer.push(c);
        }
        // next
    }

    fn resolve_buffer(&mut self, src: &mut Peekable<Chars>) {
        if !self.buffer.is_empty() {
            let tok = match self.buffer.as_str() {
                "+" => Token::Plus,
                "-" => Token::Dash,
                "=" => Token::Equals,
                "&" => Token::Ampersand,
                "*" => Token::Star,
                "/" => Token::SlashBack,
                "%" => Token::Percent,
                "<" => Token::LessThan,
                ">" => Token::GreaterThan,
                "->" => Token::RightArrow,
                "return" => Token::Keyword(Keyword::Return),
                "for" => Token::Keyword(Keyword::For),
                "if" => Token::Keyword(Keyword::If),
                "else" => Token::Keyword(Keyword::Else),
                id => {
                    if id.chars().all(|c| c.is_digit(10)) {
                        let num: Result<usize, _> = id.parse();
                        if let Ok(n) = num {
                            Token::Literal(Literal::Number(n))
                            // Token::Number(n)
                        } else {
                            Token::Id(id.to_string())
                        }
                    } else {
                        Token::Id(id.to_string())
                    }
                },
            };
            self.add(tok);
        }
        self.next(src);
    }

    fn resolve_comment(&mut self, src: &mut Peekable<Chars>) {
        self.resolve_buffer(src);
        loop {
            match src.peek() {
                Some('\r' | '\n') | None => break,
                _ => {}
            }
            self.next(src);
        }

        let comment_text = self.buffer.clone().to_string();
        let comment = Token::Comment(comment_text);
        self.add(comment);
    }

    fn resolve_string(&mut self, src: &mut Peekable<Chars>) {
        self.resolve_buffer(src);
        loop {
            match src.peek() {
                Some('"') | None => {
                    self.next(src);
                    break;
                }
                _ => {}
            }
            self.next(src);
        }

        let string_text = self.buffer.clone().to_string();
        let string = Token::Literal(Literal::String(string_text));
        self.add(string);
    }

    // resolve -> push tok
    pub fn lex(&mut self, src: &str) -> Vec<Token> {
        self.buffer.clear();
        self.tokens.clear();
        let mut src = src.chars().peekable();
        loop {
            match src.peek() {
                Some(c) => match c {
                    ' ' => self.resolve_buffer_then_add(&mut src, Token::Space),
                    ':' => self.resolve_buffer_then_add(&mut src, Token::Colon),
                    '\t' => self.resolve_buffer_then_add(&mut src, Token::Tab),
                    ',' => self.resolve_buffer_then_add(&mut src, Token::Comma),
                    ';' => self.resolve_buffer_then_add(&mut src, Token::SemiColon),
                    '\n' => self.resolve_buffer_then_add(&mut src, Token::Newline),
                    '{' => self.resolve_buffer_then_add(&mut src, Token::CurlyOpen),
                    '}' => self.resolve_buffer_then_add(&mut src, Token::CurlyClose),
                    '#' => self.resolve_comment(&mut src),
                    '"' => self.resolve_string(&mut src),
                    _ => {
                        // println!("_ =>");
                        self.next(&mut src);
                    }
                },
                None => break,
            }
        }
        self.resolve_buffer(&mut src);
        self.tokens.clone()
    }
}
