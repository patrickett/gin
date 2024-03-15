use std::{
    collections::{HashMap, HashSet},
    fs,
    iter::Map,
    path::{Path, PathBuf},
    ptr::null,
    slice::Iter,
    str::FromStr,
};

use crate::{
    expr::{Define, Expr, Literal},
    gin_type::GinType,
    lex::Lexer,
    module::GinModule,
    token::{Keyword, Token},
};

const TAB_SIZE: usize = 4;

pub struct Parser {
    tokens: Vec<Token>,
    lexer: Lexer,
    line_number: usize,
    scope: usize,
}

impl Parser {
    pub const fn new() -> Self {
        Self {
            tokens: Vec::new(),
            lexer: Lexer::new(),
            scope: 0,
            line_number: 0,
        }
    }

    fn saw_newline(&mut self) {
        self.line_number += 1;
        self.scope = 0;
    }

    fn handle_indentation(&mut self) {
        loop {
            match self.lexer.next() {
                Some(token) => match token {
                    Token::Tab => self.scope += 1,
                    Token::Space => {
                        let mut space_count = 1;

                        loop {
                            match self.lexer.next() {
                                Some(tok) => match tok {
                                    Token::Space => {
                                        space_count += 1; // eat space
                                    }
                                    _ => {
                                        self.lexer.return_to_queue(tok);
                                        break;
                                    }
                                },
                                None => break,
                            }
                        }
                        self.scope = space_count / TAB_SIZE
                    }
                    _ => {
                        self.lexer.return_to_queue(token);
                        break;
                    }
                },
                None => break,
            }
        }
    }

    fn find_implicit_return_type(&self, body: &Vec<Expr>) -> GinType {
        match body.last() {
            Some(t) => match t {
                // if we get a fncall we need find its decl
                // then we return its return type
                Expr::Call(f_name_call, _) => {
                    let e = body.iter().find(|e| match e {
                        Expr::Define(Define::Function(f_name_def, _, _)) => {
                            f_name_def == f_name_call
                        }
                        _ => false,
                    });

                    if let Some(Expr::Define(Define::Function(_, _, r_type))) = e {
                        return r_type.to_owned();
                    }
                    GinType::Nothing
                }
                expr => expr.gin_type(),
            },
            None => GinType::Nothing,
        }
    }

    fn eat(&mut self, token_kind: Token) {
        if let Some(token) = self.lexer.next() {
            if token != token_kind {
                self.lexer.return_to_queue(token)
            }
        }
    }

    fn handle_multi_line_function(&mut self, name: String) -> Option<Expr> {
        let starting_scope = self.scope;
        self.saw_newline();
        self.handle_indentation();

        let mut body = Vec::new();

        while self.scope > starting_scope {
            if let Some(expr) = self.next() {
                body.push(expr);
                self.handle_indentation();
            } else {
                break;
            }
        }
        let return_type = self.find_implicit_return_type(&body);
        let xpr = Expr::Define(Define::Function(name, body, return_type));
        Some(xpr)
    }

    /// everything to the right of `:`
    fn handle_assignment(&mut self, name: String) -> Option<Expr> {
        self.eat(Token::Space);

        match self.lexer.next() {
            Some(tok) => match tok {
                Token::Newline => self.handle_multi_line_function(name),
                Token::Literal(lit) => match self.lexer.next() {
                    Some(tok) => match tok {
                        Token::Newline => {
                            self.saw_newline();
                            // self.eat(Token::Newline);

                            let expr = Expr::Literal(lit.clone());
                            // we finished the line

                            Some(Expr::Define(Define::Function(
                                name,
                                vec![expr.clone()],
                                expr.gin_type(),
                            )))
                        }
                        Token::Comma => {
                            self.eat(Token::Comma);
                            self.eat(Token::Space);
                            Some(Expr::Literal(lit))
                        }
                        Token::CurlyClose => {
                            self.eat(Token::Space);
                            Some(Expr::Literal(lit))
                        }
                        Token::Space => {
                            self.eat(Token::Space);

                            match self.lexer.next() {
                                Some(tok) => match tok {
                                    Token::CurlyClose => Some(Expr::Literal(lit)),
                                    _ => {
                                        self.lexer.return_to_queue(tok);
                                        None
                                    }
                                },
                                None => panic!(
                                    "Unexpected (None) at positon {} line {}",
                                    self.lexer.pos(),
                                    self.line_number
                                ),
                            }
                        }
                        u => {
                            println!("found u: {:#?}", u);
                            None
                        }
                    },
                    None => panic!(
                        "Unexpected (None) at positon {} line {}",
                        self.lexer.pos(),
                        self.line_number
                    ),
                },

                Token::CurlyOpen => {
                    let mut object_contents: HashMap<String, Expr> = HashMap::new();
                    self.eat(Token::Space);
                    loop {
                        match self.lexer.next() {
                            Some(token) => match token {
                                Token::Id(o_name) => {
                                    if let Some(expr) = self.handle_id(o_name.clone()) {
                                        object_contents.insert(o_name, expr);
                                    }
                                    self.eat(Token::Space);
                                }
                                Token::CurlyClose => {
                                    self.eat(Token::Space);
                                    break;
                                }
                                _ => break,
                            },
                            None => panic!(
                                "Unexpected (None) at positon {} line {}",
                                self.lexer.pos(),
                                self.line_number
                            ),
                        }
                    }

                    self.eat(Token::Newline);

                    let ex = Expr::Literal(Literal::Object(object_contents));

                    Some(Expr::Define(Define::Function(
                        name,
                        vec![ex.clone()],
                        ex.gin_type(),
                    )))
                }
                Token::BracketOpen => {
                    let mut list = Vec::new();
                    loop {
                        match self.next() {
                            Some(expr) => list.push(expr),
                            None => panic!(
                                "Unexpected (None) at positon {} line {}",
                                self.lexer.pos(),
                                self.line_number
                            ),
                        }
                    }
                }
                _ => panic!(
                    "Unexpected ({:?}) at positon {} line {} 2",
                    tok,
                    self.lexer.pos(),
                    self.line_number
                ),
            },
            None => panic!(
                "Unexpected (None) at positon {} line {}",
                self.lexer.pos(),
                self.line_number
            ),
        }
    }

    fn handle_data_type(&mut self) -> Option<Expr> {
        // already have seen the curlyopen
        // because this is a different context we have
        // to manually iterate for the items in the data defintion

        // eats potential newline
        match self.lexer.next() {
            Some(tok) => match tok {
                Token::Newline => {
                    self.saw_newline();
                }
                _ => self.lexer.return_to_queue(tok),
            },
            None => panic!("finish writing your data type"),
        }

        // this is a bit eager, but we are going to assume we only
        // have one pair of curlies and there is no nesting yet.

        let mut data_content: HashMap<String, GinType> = HashMap::new();
        loop {
            self.handle_indentation();

            let Some(token) = self.lexer.next() else {
                break;
            };

            match token {
                Token::CurlyClose => break,
                // seperates { [field] [type] \n [field] [type] }
                Token::Newline => {
                    // TODO: hint add comma,
                    continue;
                }
                // seperates { [field] [type] , [field] [type] }
                Token::Comma => continue,
                Token::Id(id_name) => {
                    self.eat(Token::Space);

                    let Some(Token::Id(token_type)) = self.lexer.next() else {
                        panic!("failed to declare type on data field {id_name}")
                    };

                    let gin_type = GinType::from_str(&token_type.as_str())
                        .expect("parsed gin type from token_type");

                    data_content.insert(id_name, gin_type);
                }
                unknown => panic!(
                    "Unexpected {unknown:#?} at position: {} line: {}",
                    self.lexer.pos(),
                    self.line_number
                ),
            }
        }

        Some(Expr::Define(Define::DataContent(data_content)))
    }

    fn handle_token(&mut self, token: Token) -> Option<Expr> {
        match token {
            Token::Keyword(keyword) => match keyword {
                Keyword::Include => todo!(),
                Keyword::If => todo!(),
                Keyword::Else => todo!(),
                Keyword::For => todo!(),
                Keyword::Return => todo!(),
            },
            Token::Newline => {
                self.saw_newline();
                self.handle_indentation();
                self.next()
            }
            Token::Literal(lit) => Some(Expr::Literal(lit)),
            Token::Id(name) => self.handle_id(name),
            Token::CurlyOpen => self.handle_data_type(),
            _ => self.next(),
        }
    }

    fn handle_id(&mut self, id_name: String) -> Option<Expr> {
        let Some(token) = self.lexer.next() else {
            return Some(Expr::Call(id_name, None));
        };

        match token {
            Token::Colon => self.handle_assignment(id_name),
            Token::Newline => {
                self.saw_newline();
                Some(Expr::Call(id_name, None))
            }
            Token::Literal(lit) => {
                let expr = Some(Box::new(Expr::Literal(lit)));
                Some(Expr::Call(id_name, expr))
            }
            // function call does require a space
            Token::Space => {
                let next_expr = self.next();
                if let Some(Expr::Define(Define::DataContent(dc))) = &next_expr {
                    return Some(Expr::Define(Define::Data(id_name, dc.to_owned())));
                }

                let expr = next_expr.map(|v| Box::new(v));

                Some(Expr::Call(id_name, expr))
            }
            Token::CurlyOpen => {
                if let Some(Expr::Define(Define::DataContent(dc))) = self.handle_data_type() {
                    return Some(Expr::Define(Define::Data(id_name, dc)));
                }
                panic!("Failed to get data_content for {id_name}")
            }
            unknown => panic!(
                "Unexpected ({unknown:?}) at positon {} line {}",
                self.lexer.pos(),
                self.line_number
            ),
        }
    }

    pub fn start(&mut self, path: &Path) -> GinModule {
        let file_contents = fs::read_to_string(path).expect("unable to read path to string");
        self.lexer.set_source_content(file_contents);
        GinModule::new(path.to_path_buf(), self.collect())
    }
}

impl Iterator for Parser {
    type Item = Expr;

    fn next(&mut self) -> Option<Self::Item> {
        match self.lexer.next() {
            Some(tok) => self.handle_token(tok),
            None => None,
        }
    }
}
