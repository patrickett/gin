use std::{collections::HashMap, str::FromStr};

use crate::{
    expr::{define::Define, literal::Literal, Binary, Expr, Op},
    gin_type::GinType,
    lexer::{source_file::SourceFile, Lexer},
    token::Token,
};

#[derive(Debug, Clone)]
pub struct Parser {
    lexer: Lexer,
    scope: usize,
}

impl Parser {
    pub const fn new() -> Self {
        Self {
            lexer: Lexer::new(),
            scope: 0,
        }
    }

    pub fn set_content(&mut self, source_file: &SourceFile) {
        self.lexer.set_content(source_file)
    }

    fn saw_newline(&mut self) {
        self.scope = 0;
    }

    fn handle_indentation(&mut self) {
        while let Some(token) = self.lexer.next() {
            match token {
                Token::Tab => self.scope += 1,
                Token::Space => {
                    let mut space_count = 1;

                    while let Some(tok) = self.lexer.next() {
                        match tok {
                            Token::Space => space_count += 1, // eat space
                            _ => {
                                self.lexer.return_to_queue(tok);
                                break;
                            }
                        }
                    }

                    self.scope = space_count / 2
                }
                _ => {
                    self.lexer.return_to_queue(token);
                    break;
                }
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
        Some(Expr::Define(Define::Function(name, body, return_type)))
    }

    fn handle_op(&mut self, lhs: Expr, op: Op) -> Option<Expr> {
        let Some(rhs) = self.next() else {
            panic!("failed to get rhs operation: {op:#?}")
        };

        Some(Expr::Opertation(Box::new(lhs), op, Box::new(rhs)))
    }

    fn handle_lit_op(&mut self, lhs: Literal, op: Op) -> Option<Expr> {
        let Some(expr) = self.next() else {
            panic!("failed to get rhs operation: {op:#?}")
        };

        Some(Expr::Opertation(
            Box::new(Expr::Literal(lhs)),
            op,
            Box::new(expr),
        ))
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
                            // [id] [arg?] (method on type)
                            // [+|-|/|*] [expr]
                            if let Some(fn_or_op) = self.lexer.next() {
                                match fn_or_op {
                                    Token::Id(_) => todo!(),
                                    Token::Plus => {
                                        self.eat(Token::Space);
                                        let op = self
                                            .handle_lit_op(lit, Op::Bin(Binary::Add))
                                            .expect("failed to get binop");

                                        Some(Expr::Define(Define::Function(
                                            name,
                                            vec![op.clone()],
                                            op.gin_type(),
                                        )))
                                    }
                                    Token::Star => {
                                        self.eat(Token::Space);

                                        let op = self
                                            .handle_lit_op(lit, Op::Bin(Binary::Mul))
                                            .expect("failed to get binop");

                                        Some(Expr::Define(Define::Function(
                                            name,
                                            vec![op.clone()],
                                            op.gin_type(),
                                        )))
                                    }
                                    Token::Dash => {
                                        self.eat(Token::Space);

                                        let op = self
                                            .handle_lit_op(lit, Op::Bin(Binary::Sub))
                                            .expect("failed to get binop");

                                        Some(Expr::Define(Define::Function(
                                            name,
                                            vec![op.clone()],
                                            op.gin_type(),
                                        )))
                                    }
                                    Token::SlashForward => {
                                        self.eat(Token::Space);

                                        let op = self
                                            .handle_lit_op(lit, Op::Bin(Binary::Div))
                                            .expect("failed to get binop");

                                        Some(Expr::Define(Define::Function(
                                            name,
                                            vec![op.clone()],
                                            op.gin_type(),
                                        )))
                                    }
                                    unknown => panic!(
                                        "Unexpected token [{unknown:#?}] at {}",
                                        self.lexer.location()
                                    ),
                                }
                            } else {
                                println!("getting here");
                                None
                            }
                            // match self.lexer.next() {
                            //     Some(tok) => match tok {
                            //         Token::CurlyClose => Some(Expr::Literal(lit)),
                            //         tk => {
                            //             self.lexer.return_to_queue(tk);
                            //             self.next()
                            //         }
                            //     },
                            //     None => panic!(
                            //         "Unexpected (None) at positon {} line {}",
                            //         self.lexer.pos(),
                            //         self.line_number
                            //     ),
                            // }
                        }
                        Token::Plus => {
                            let op = self
                                .handle_lit_op(lit, Op::Bin(Binary::Add))
                                .expect("failed to get binop");

                            Some(Expr::Define(Define::Function(
                                name,
                                vec![op.clone()],
                                op.gin_type(),
                            )))
                        }
                        Token::Star => {
                            let op = self
                                .handle_lit_op(lit, Op::Bin(Binary::Mul))
                                .expect("failed to get binop");

                            Some(Expr::Define(Define::Function(
                                name,
                                vec![op.clone()],
                                op.gin_type(),
                            )))
                        }
                        Token::Dash => {
                            let op = self
                                .handle_lit_op(lit, Op::Bin(Binary::Sub))
                                .expect("failed to get binop");

                            Some(Expr::Define(Define::Function(
                                name,
                                vec![op.clone()],
                                op.gin_type(),
                            )))
                        }
                        Token::SlashForward => {
                            let op = self
                                .handle_lit_op(lit, Op::Bin(Binary::Div))
                                .expect("failed to get binop");

                            Some(Expr::Define(Define::Function(
                                name,
                                vec![op.clone()],
                                op.gin_type(),
                            )))
                        }
                        unknown => panic!(
                            "Unexpected token [{unknown:#?}] at {}",
                            self.lexer.location()
                        ),
                    },
                    None => panic!("Unexpected (None) at {}", self.lexer.location()),
                },

                Token::CurlyOpen => {
                    let mut object_contents: HashMap<String, Expr> = HashMap::new();
                    self.eat(Token::Space);
                    while let Some(token) = self.lexer.next() {
                        match token {
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
                        }
                    }
                    // loop {
                    //     match self.lexer.next() {
                    //         Some(token) => ,
                    //         None => panic!("Unexpected (None) at {}", self.lexer.pos()),
                    //     }
                    // }

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
                            None => panic!("Unexpected (None) at {}", self.lexer.location()),
                        }
                    }
                }
                Token::Id(id_name) => self.handle_id(id_name),
                unknown => panic!(
                    "Unexpected token [{unknown:#?}] at {}",
                    self.lexer.location()
                ),
            },
            None => panic!("Unexpected (None) at {}", self.lexer.location()),
        }
    }

    // already have seen the curlyopen
    // because this is a different context we have
    // to manually iterate for the items in the data defintion
    fn handle_data_type(&mut self) -> Option<Expr> {
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
                    "Unexpected token [{unknown:#?}] at {}",
                    self.lexer.location()
                ),
            }
        }

        Some(Expr::Define(Define::DataContent(data_content)))
    }

    fn handle_token(&mut self, token: Token) -> Option<Expr> {
        match token {
            Token::Newline => {
                self.saw_newline();
                self.handle_indentation();
                self.next()
            }
            Token::Literal(lit) => Some(Expr::Literal(lit)),
            Token::Id(name) => self.handle_id(name),
            Token::CurlyOpen => self.handle_data_type(),
            Token::Comment(_) => self.next(),
            Token::DocComment(_) => self.next(),
            unknown => panic!(
                "Unexpected token [{unknown:#?}] at {}",
                self.lexer.location()
            ),
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
                "Unexpected token [{unknown:#?}] at {}",
                self.lexer.location()
            ),
        }
    }

    // pub fn build_expr(&self) -> Option<Expr> {
    //     let next = self.lexer.next()?;
    // }
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
