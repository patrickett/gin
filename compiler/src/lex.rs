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
    Number(usize),
}

#[derive(Debug, PartialEq, Clone)]
pub struct Token {
    pos: usize,
    // end: usize,
    kind: TokenKind,
}

impl Token {
    pub fn new(kind: TokenKind, pos: usize) -> Token {
        Self {
            pos,
            // end: start + length,
            kind,
        }
    }

    pub fn kind(&self) -> TokenKind {
        self.kind.to_owned()
    }

    pub fn pos(&self) -> usize {
        self.pos.to_owned()
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum TokenKind {
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
    Comment(String),
    Id(String),
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
    EOF,
}

pub struct Lexer {
    index: usize,
    buffer: String,
    pub tokens: Vec<Token>,
}

impl Lexer {
    pub fn new() -> Self {
        Self {
            index: 0,
            buffer: String::new(),
            tokens: Vec::new(),
        }
    }

    fn add(&mut self, tok: Token) {
        self.buffer.clear();
        self.tokens.push(tok);
    }

    fn resolve_buffer_then_add(&mut self, src: &mut Peekable<Chars>, tok: TokenKind) {
        self.resolve_buffer(src);
        self.add(Token::new(tok, self.index));
    }

    fn next(&mut self, src: &mut Peekable<Chars>) {
        let next = src.next();
        if let Some(c) = next {
            self.buffer.push(c);
            self.index += 1;
        }
        // next
    }

    fn resolve_buffer(&mut self, src: &mut Peekable<Chars>) {
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
                            // Token::Number(n)
                        } else {
                            TokenKind::Id(id.to_string())
                        }
                    } else {
                        TokenKind::Id(id.to_string())
                    }
                }
            };

            self.add(Token::new(tok, self.index));
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
        let comment = Token::new(TokenKind::Comment(comment_text), self.index);
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
        let string = Token::new(TokenKind::Literal(Literal::String(string_text)), 1);
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
                    ' ' => self.resolve_buffer_then_add(&mut src, TokenKind::Space),
                    ':' => self.resolve_buffer_then_add(&mut src, TokenKind::Colon),
                    '\t' => self.resolve_buffer_then_add(&mut src, TokenKind::Tab),
                    ',' => self.resolve_buffer_then_add(&mut src, TokenKind::Comma),
                    ';' => self.resolve_buffer_then_add(&mut src, TokenKind::SemiColon),
                    '\n' => self.resolve_buffer_then_add(&mut src, TokenKind::Newline),
                    '{' => self.resolve_buffer_then_add(&mut src, TokenKind::CurlyOpen),
                    '}' => self.resolve_buffer_then_add(&mut src, TokenKind::CurlyClose),
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
        // self.add(Token::new(TokenKind::EOF, self.index));
        self.tokens.clone()
    }
}
