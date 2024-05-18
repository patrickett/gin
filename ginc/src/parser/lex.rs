use super::lexer::token::{Keyword, Token, TokenKind};
use crate::compiler_error::CompilerError;
use crate::gin_type::number::GinNumber;
use crate::value::GinValue;
use std::fs::File;
use std::io::Read;
use std::iter::Peekable;
use std::str::Chars;

pub struct SimpleLexer {
    buffer: String,
    line: usize,
    start: usize,
}

pub struct LexedFile {
    pub tokens: Vec<Token>,
}

impl SimpleLexer {
    pub fn new() -> Self {
        Self {
            start: 1,
            line: 1,
            buffer: String::new(),
        }
    }

    fn get_source(&self, path: &String) -> Result<String, CompilerError> {
        let mut file = File::open(path).map_err(CompilerError::IO)?;

        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .map_err(CompilerError::IO)?;

        Ok(contents)
    }

    fn push(&mut self, tokens: &mut Vec<Token>, kind: TokenKind) {
        let length = kind.len();
        let token = Token::new(kind, self.line, self.start, self.start + length);
        tokens.push(token);
    }

    fn space(&mut self, tokens: &mut Vec<Token>, source: &mut Peekable<Chars>) {
        let mut space_count = 1;

        while let Some(_) = source.next_if_eq(&' ') {
            self.start += 1;
            space_count += 1;
        }

        let tab_count = space_count / 2;
        if tab_count > 1 {
            for _ in 0..tab_count {
                self.push(tokens, TokenKind::Tab);
            }
        } else {
            // note: disabled until i know spaces are needed in some context
            // self.push(tokens, TokenKind::Space)
        }
    }

    fn newline(&mut self, tokens: &mut Vec<Token>) {
        self.line += 1;
        self.start = 1;
        self.push(tokens, TokenKind::Newline);
    }

    fn comment(&mut self, tokens: &mut Vec<Token>, source: &mut Peekable<Chars>) {
        let mut comment_string = String::new();
        let mut saw_newline = false;

        while let Some(ch) = source.next() {
            self.start += 1;
            match ch {
                '\n' => {
                    saw_newline = true;
                    break;
                }
                c => comment_string.push(c),
            }
        }

        self.push(tokens, TokenKind::Comment(comment_string));

        // while ending the comment its still good to put it in + line_count
        if saw_newline {
            self.newline(tokens);
        }
    }

    // check the buffer and push any tokens if possible
    fn buffer(&mut self, tokens: &mut Vec<Token>) {
        if !self.buffer.is_empty() {
            match self.buffer.as_str() {
                "where" => self.push(tokens, TokenKind::Keyword(Keyword::Where)),
                "is" => self.push(tokens, TokenKind::Keyword(Keyword::Is)),
                word => {
                    // if first_char.is_numeric() {
                    //     if id.contains("..") {
                    //         match self.resolve_range(id) {
                    //             Ok(kind) => kind,
                    //             Err(e) => {
                    //                 self.buffer.clear();
                    //                 self.queue.push_back(Err(e));
                    //                 return;
                    //             }
                    //         }
                    //     } else if let Ok(num) = id.parse::<GinNumber>() {
                    //         TokenKind::Literal(GinValue::Number(num))
                    //     } else {
                    //         todo!()
                    //     }
                    // } else if first_char.is_uppercase() {
                    //     TokenKind::Tag(id.to_string())
                    // } else {
                    //     TokenKind::Id(id.to_string())
                    // }
                    //
                    if let Some(first_char) = word.chars().next() {
                        if first_char.is_numeric() {
                            if word.contains("..") {
                                todo!() // range
                            } else if let Ok(number) = word.parse::<GinNumber>() {
                                self.push(tokens, TokenKind::Literal(GinValue::Number(number)))
                            } else {
                                todo!()
                            }
                        } else if first_char.is_ascii_uppercase() {
                            self.push(tokens, TokenKind::Tag(word.to_owned()))
                        } else {
                            self.push(tokens, TokenKind::Id(word.to_owned()))
                        }
                    } else {
                        panic!("why am i here")
                    }
                }
            }
            self.buffer.clear()
        }
    }

    pub fn lex(&mut self, path: &String) -> Result<LexedFile, CompilerError> {
        let content = self.get_source(path)?;
        let mut source = content.chars().peekable();

        let mut tokens: Vec<Token> = Vec::new();

        while let Some(ch) = source.next() {
            // println!("index: {}, char: {}", self.start, &ch);
            self.start += 1;
            match ch {
                ':' => {
                    self.buffer(&mut tokens);
                    self.push(&mut tokens, TokenKind::Colon)
                }
                '/' => {
                    self.buffer(&mut tokens);
                    self.push(&mut tokens, TokenKind::SlashBack)
                }
                '-' => {
                    self.buffer(&mut tokens);
                    self.push(&mut tokens, TokenKind::Dash)
                }
                '*' => {
                    self.buffer(&mut tokens);
                    self.push(&mut tokens, TokenKind::Star)
                }
                '+' => {
                    self.buffer(&mut tokens);
                    self.push(&mut tokens, TokenKind::Plus)
                }
                ' ' => {
                    self.buffer(&mut tokens);
                    self.space(&mut tokens, &mut source);
                }
                '{' => self.push(&mut tokens, TokenKind::CurlyOpen),
                '}' => self.push(&mut tokens, TokenKind::CurlyClose),
                '\t' => self.push(&mut tokens, TokenKind::Tab),
                '#' => self.comment(&mut tokens, &mut source),
                ',' => {
                    self.buffer(&mut tokens);
                    self.push(&mut tokens, TokenKind::Comma);
                }
                '\n' => {
                    self.buffer(&mut tokens);
                    self.newline(&mut tokens);
                }
                c => self.buffer.push(c),
            }
        }

        Ok(LexedFile { tokens })
    }
}
