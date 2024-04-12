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
        self.handle_indentation();
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
                                self.lexer.defer(tok);
                                break;
                            }
                        }
                    }

                    self.scope = space_count / 2
                }
                _ => {
                    self.lexer.defer(token);
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
                self.lexer.defer(token)
            }
        }
    }

    fn handle_multi_line_function(&mut self, name: String) -> Option<Expr> {
        let starting_scope = self.scope;
        self.saw_newline();
        // self.handle_indentation();
        let mut body = Vec::new();

        while self.scope > starting_scope {
            if let Some(expr) = self.next() {
                body.push(expr);
                // self.handle_indentation();
            } else {
                break;
            }
        }
        let return_type = self.find_implicit_return_type(&body);
        Some(Expr::Define(Define::Function(name, body, return_type)))
    }

    /// everything to the right of `:`
    fn handle_assignment(&mut self, name: String) -> Option<Expr> {
        self.eat(Token::Space);

        let token = self.lexer.next()?;

        if token == Token::Newline {
            return self.handle_multi_line_function(name);
        }
        // return the token since we will need it later.
        self.lexer.defer(token);

        // we can assume its only a single expr since if it was a func def
        // it would have been picked up above in the self.handle_multi_line_function
        let expr = self.next()?;
        // println!("{:#?}", expr);
        let ret = &expr.gin_type();

        Some(Expr::Define(Define::Function(
            name,
            vec![expr],
            ret.to_owned(),
        )))
    }

    fn handle_data_literal(&mut self) -> Option<Expr> {
        println!("handle_data_literal");
        let mut data_values: HashMap<String, Expr> = HashMap::new();

        loop {
            match self.lexer.next()? {
                Token::CurlyClose => break,
                // seperates { [field] [type] \n [field] [type] }
                Token::Newline => {
                    self.saw_newline();
                    continue;
                }
                // seperates { [field] [type] , [field] [type] }
                Token::Comma => continue,
                Token::Space => continue,
                Token::Id(id_name) => {
                    self.eat(Token::Space);

                    if let Some(Token::Colon) = self.lexer.next() {
                        match self.next() {
                            Some(expr) => {
                                data_values.insert(id_name, expr);
                            }
                            None => panic!("{}", self.lexer.location()),
                        }
                    } else {
                        // might be a shorthand {name}
                        todo!()
                    }
                }
                unknown => panic!(
                    "Unexpected token [{unknown:#?}] at {}",
                    self.lexer.location()
                ),
            }
        }

        Some(Expr::Literal(Literal::Data(data_values)))
    }

    // already have seen the curlyopen
    // because this is a different context we have
    // to manually iterate for the items in the data defintion
    fn handle_data_type(&mut self, tag_name: String) -> Option<Expr> {
        self.eat(Token::Space);

        match self.lexer.next() {
            Some(tok) => match tok {
                Token::Newline => {
                    self.saw_newline();
                }
                _ => self.lexer.defer(tok),
            },
            None => panic!("finish writing your data type"),
        }

        // this is a bit eager, but we are going to assume we only
        // have one pair of curlies and there is no nesting yet.

        let mut data_content: HashMap<String, GinType> = HashMap::new();
        loop {
            self.handle_indentation();
            // self.eat(Token::Space);

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

                    let Some(Token::Tag(token_type)) = self.lexer.next() else {
                        // TODO: check if it is defined within an availble scope
                        // if it is this might be shorthand fill
                        //
                        // we might add a Fill type here that can be filled on another pass
                        // with more context from the document

                        panic!(
                            "failed to declare type on data field {id_name} at pos: {}",
                            self.lexer.location()
                        )
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

        Some(Expr::Define(Define::Data(tag_name, data_content)))
    }

    fn check_for_op(&mut self, token: &Token) -> Option<Op> {
        self.eat(Token::Space);

        match token {
            Token::SlashBack => Some(Op::Bin(Binary::Div)),
            Token::SlashForward => todo!(),
            Token::LessThan => todo!(),
            Token::LessThanOrEqualTo => todo!(),
            Token::GreaterThan => todo!(),
            Token::GreaterThanOrEqualTo => todo!(),
            Token::Plus => Some(Op::Bin(Binary::Add)),
            Token::Dash => Some(Op::Bin(Binary::Sub)),
            Token::Equals => todo!(),
            Token::Ampersand => todo!(),
            Token::Star => Some(Op::Bin(Binary::Mul)),
            Token::Percent => todo!(),
            _ => None,
        }
    }

    fn handle_literal(&mut self, literal: Literal) -> Option<Expr> {
        self.eat(Token::Space);
        let next_token = self.lexer.next()?;
        if next_token == Token::Newline {
            return Some(Expr::Literal(literal));
        } else if let Token::Id(_) = next_token {
            // could be associated function
            panic!("todo associated function")
        }

        if let Some(op) = self.check_for_op(&next_token) {
            let lhs = Box::new(Expr::Literal(literal));
            self.eat(Token::Space);
            let r_expr = self.next()?;
            let rhs = Box::new(r_expr);

            Some(Expr::Operation(lhs, op, rhs))
        } else {
            match next_token {
                Token::CurlyClose => {
                    self.lexer.defer(next_token);
                    Some(Expr::Literal(literal))
                }
                Token::Comma => {
                    self.lexer.defer(next_token);
                    Some(Expr::Literal(literal))
                }
                t => {
                    panic!("error: {:#?}", t)
                }
            }
        }
    }

    fn handle_tag(&mut self, tag_name: String) -> Option<Expr> {
        self.eat(Token::Space);

        match self.lexer.next()? {
            Token::CurlyOpen => self.handle_data_type(tag_name),
            _ => panic!("asd"),
        }
    }

    fn handle_token(&mut self, token: Token) -> Option<Expr> {
        match token {
            Token::Newline => {
                self.saw_newline();
                // self.handle_indentation();
                self.next()
            }
            Token::Literal(lit) => self.handle_literal(lit),
            Token::Id(name) => self.handle_id(name),
            Token::Tag(name) => self.handle_tag(name),

            Token::CurlyOpen => self.handle_data_literal(),
            Token::Comment(_) => self.next(),
            Token::DocComment(_) => self.next(),
            Token::Space => {
                // we should be able to assume we are not checking indent here
                // skip and go next
                self.next()
            }
            unknown => panic!(
                "Unexpected token [{unknown:#?}] at {}",
                self.lexer.location()
            ),
        }
    }

    fn handle_id(&mut self, id_name: String) -> Option<Expr> {
        self.eat(Token::Space);

        let Some(token) = self.lexer.next() else {
            return Some(Expr::Call(id_name, None));
        };

        if let Some(op) = self.check_for_op(&token) {
            let r_expr = self.next()?;
            let rhs = Box::new(r_expr);

            // TODO: [call arg + call]
            let lhs = Box::new(Expr::Call(id_name, None));
            return Some(Expr::Operation(lhs, op, rhs));
        }

        match token {
            Token::Id(ident) => {
                let expr = self.handle_id(ident)?;
                Some(Expr::Call(id_name, Some(Box::new(expr))))
            }
            Token::Colon => self.handle_assignment(id_name),
            Token::Newline => {
                self.saw_newline();
                Some(Expr::Call(id_name, None))
            }
            Token::Comma => Some(Expr::Call(id_name, None)),
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
            Token::CurlyOpen => self.handle_data_literal(),

            unknown => panic!(
                "Unexpected token [{unknown:#?}] at {}",
                self.lexer.location()
            ),
        }
    }
}

impl Iterator for Parser {
    type Item = Expr;

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.lexer.next()?;
        self.handle_token(next)
    }
}
