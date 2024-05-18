use self::token::{Keyword, Token, TokenKind};
use crate::{
    compiler_error::CompilerError, gin_type::number::GinNumber, source_file::SourceFile,parse
    value::GinValue,
};
use std::collections::VecDeque;

pub mod token;

// TODO: when done lexing
// empty vec content and remove full_path

pub struct Lexer {
    line: usize,
    line_position: usize,
    last_position: usize,
    content: Vec<char>,
    char_count: usize,
    full_path: String,
    buffer: String,
    queue: VecDeque<Result<Token, CompilerError>>,
}

#[derive(Debug)]
pub struct Location {
    line: usize,
    start_position: usize,
    file_path: String,
}

impl ToString for Location {
    fn to_string(&self) -> String {
        format!("{}:{}:{}", self.file_path, self.line, self.start_position)
    }
}

impl Lexer {
    pub const fn new() -> Self {
        Self {
            line: 1,
            line_position: 1,
            buffer: String::new(),
            full_path: String::new(),
            content: Vec::new(),
            char_count: 0,
            last_position: 1,
            queue: VecDeque::new(),
        }
    }

    pub fn current_location(&self) -> Location {
        Location {
            line: self.line,
            start_position: self.line_position,
            file_path: self.full_path.clone(),
        }
    }

    pub fn saw_newline(&mut self) -> Result<Token, CompilerError> {
        let t = self.resolve_buffer_then_add(TokenKind::Newline);
        self.line += 1;
        // goes back to start
        self.line_position = 1;
        self.last_position = 1;
        t
    }

    pub fn set_content(&mut self, source_file: &mut SourceFile) {
        self.content = source_file.get_content().chars().collect();
        self.full_path = source_file.full_path().to_owned();
    }

    pub fn defer(&mut self, t: Token) {
        self.queue.push_front(Ok(t))
    }

    fn create(&mut self, kind: TokenKind) -> Token {
        let start = self.last_position;
        let end = self.line_position;
        let token = Token::new(kind, self.line, start, end);
        self.last_position = end;
        token
    }

    fn resolve_buffer_then_add(&mut self, kind: TokenKind) -> Result<Token, CompilerError> {
        self.resolve_buffer();
        self.line_position -= 1;
        self.line_position += 1;
        Ok(self.create(kind))
    }

    fn resolve_range(&self, ident: &str) -> Result<TokenKind, CompilerError> {
        let parts: Vec<&str> = ident.split("..").collect();

        if parts.len() != 2 {
            return Err(CompilerError::InvalidRange(self.current_location()));
        }

        if let (Ok(start), Ok(end)) = (parts[0].parse::<usize>(), parts[1].parse::<usize>()) {
            Ok(TokenKind::Range(start, end))
        } else {
            Err(CompilerError::InvalidRange(self.current_location()))
        }
    }

    fn resolve_buffer(&mut self) {
        if self.buffer.is_empty() {
            return;
        };

        let tok = match self.buffer.as_str() {
            "+" => TokenKind::Plus,
            "-" => TokenKind::Dash,
            "=" => TokenKind::Equals,
            "&" => TokenKind::Ampersand,
            "*" => TokenKind::Star,
            "/" => TokenKind::SlashForward,
            "%" => TokenKind::Percent,
            "<" => TokenKind::LessThan,
            "<=" => TokenKind::LessThanOrEqualTo,
            ">" => TokenKind::GreaterThan,
            ">=" => TokenKind::GreaterThanOrEqualTo,
            "->" => TokenKind::RightArrow,
            "<-" => TokenKind::LeftArrow,
            "is" => TokenKind::Keyword(Keyword::Is),
            "if" => TokenKind::Keyword(Keyword::If),
            "else" => TokenKind::Keyword(Keyword::Else),
            "then" => TokenKind::Keyword(Keyword::Then),
            "include" => TokenKind::Keyword(Keyword::Include),
            "true" => TokenKind::Literal(GinValue::Bool(true)),
            "false" => TokenKind::Literal(GinValue::Bool(false)),
            "return" => TokenKind::Keyword(Keyword::Return),
            "for" => TokenKind::Keyword(Keyword::For),
            "when" => TokenKind::Keyword(Keyword::When),
            id => {
                let first_char = id.chars().next().expect("Failed to get first char");

                if first_char.is_numeric() {
                    if id.contains("..") {
                        match self.resolve_range(id) {
                            Ok(kind) => kind,
                            Err(e) => {
                                self.buffer.clear();
                                self.queue.push_back(Err(e));
                                return;
                            }
                        }
                    } else if let Ok(num) = id.parse::<GinNumber>() {
                        TokenKind::Literal(GinValue::Number(num))
                    } else {
                        todo!()
                    }
                } else if first_char.is_uppercase() {
                    TokenKind::Tag(id.to_string())
                } else {
                    TokenKind::Id(id.to_string())
                }
            }
        };

        self.buffer.clear();
        let t = self.create(tok);
        // println!("{:#?}", &t);
        self.queue.push_back(Ok(t));
    }

    fn next_char(&mut self) -> Option<char> {
        self.char_count += 1;
        self.line_position += 1;
        self.content.get(self.char_count - 1).map(|&c| c)
    }

    fn resolve_comment(&mut self) -> Result<Token, CompilerError> {
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
                    let comment_text = self.buffer.trim().to_string();
                    let comment = match is_doc_comment {
                        true => TokenKind::DocComment(comment_text),
                        false => TokenKind::Comment(comment_text),
                    };
                    let c = self.create(comment);
                    self.buffer.clear();
                    let nwl = self.saw_newline()?; // -> T & T
                    self.queue.push_back(Ok(nwl));
                    return Ok(c);
                }
                c => self.buffer.push(c),
            }
        }

        panic!("comment loop ran zero times");
    }

    fn resolve_template_string(&mut self) -> Result<Token, CompilerError> {
        self.resolve_buffer();
        while let Some(c) = self.next_char() {
            match c {
                '`' => break,
                c => self.buffer.push(c),
            }
        }
        let string = TokenKind::Literal(GinValue::TemplateString(self.buffer.to_string()));
        let s = self.create(string);
        self.buffer.clear();
        Ok(s)
    }

    fn resolve_string(&mut self) -> Result<Token, CompilerError> {
        self.resolve_buffer();

        while let Some(c) = self.next_char() {
            match c {
                '"' => break,
                c => self.buffer.push(c),
            }
        }
        let string = TokenKind::Literal(GinValue::String(self.buffer.to_string()));
        let s = self.create(string);
        self.buffer.clear();
        Ok(s)
    }
}

impl Iterator for Lexer {
    type Item = Result<Token, CompilerError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.queue.len() > 0 {
                return self.queue.pop_front();
            }

            if let Some(c) = self.next_char() {
                match c {
                    ' ' => return Some(self.resolve_buffer_then_add(TokenKind::Space)),
                    ':' => return Some(self.resolve_buffer_then_add(TokenKind::Colon)),
                    '\t' => return Some(self.resolve_buffer_then_add(TokenKind::Tab)),
                    ',' => return Some(self.resolve_buffer_then_add(TokenKind::Comma)),
                    ';' => return Some(self.resolve_buffer_then_add(TokenKind::SemiColon)),
                    '\n' => return Some(self.saw_newline()),
                    '[' => return Some(self.resolve_buffer_then_add(TokenKind::BracketOpen)),
                    ']' => return Some(self.resolve_buffer_then_add(TokenKind::BracketClose)),
                    '{' => return Some(self.resolve_buffer_then_add(TokenKind::CurlyOpen)),
                    '}' => return Some(self.resolve_buffer_then_add(TokenKind::CurlyClose)),
                    '#' => return Some(self.resolve_comment()),
                    '"' => return Some(self.resolve_string()),
                    '`' => return Some(self.resolve_template_string()),
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
