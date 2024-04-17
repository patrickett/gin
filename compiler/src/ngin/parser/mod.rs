pub mod module;

pub mod lexer;

use std::{collections::HashMap, str::FromStr};

use self::{
    lexer::{token::Token, Lexer},
    module::{
        definition::{DataDefiniton, Define, Function},
        expression::{Binary, Call, Comparison, Expr, Op},
        statement::{control_flow::ControlFlow, Statement},
        Node,
    },
};

use super::{gin_type::GinType, source_file::SourceFile, value::GinValue};

#[derive(Debug)]
pub enum ParseError {
    FailedParsingMultiLineFunction,
}

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

    /// Check the next token and if it matches the argument then skip it
    fn skip(&mut self, token_kind: Token) {
        if let Some(token) = self.lexer.next() {
            if token != token_kind {
                self.lexer.defer(token)
            }
        }
    }

    fn handle_multi_line_function(&mut self, name: String) -> Option<Function> {
        let starting_scope = self.scope;
        self.saw_newline();

        let mut body = Vec::new();
        while self.scope > starting_scope {
            if let Some(node) = self.next() {
                body.push(node);
            } else {
                break;
            }
        }

        Some(Function::new(name, body))
    }

    /// everything to the right of `:`
    fn handle_assignment(&mut self, name: String) -> Option<Node> {
        self.skip(Token::Space);
        let token = self.lexer.next()?;
        if token == Token::Newline {
            return match self.handle_multi_line_function(name) {
                Some(func) => Some(Node::Definition(Define::Function(func))),
                None => todo!(),
            };
        }
        self.lexer.defer(token); // return the token since we will need it later.

        // we can assume its only a single expr since if it was a func def
        // it would have been picked up above in the self.handle_multi_line_function

        let func = Function::new(name, vec![self.next()?]);
        Some(Node::Definition(Define::Function(func)))
    }

    fn handle_data_literal(&mut self) -> Option<GinValue> {
        let mut data_values: HashMap<String, Expr> = HashMap::new();

        loop {
            match self.lexer.next()? {
                Token::CurlyClose => break,
                // seperates { [field] [value] \n [field] [type] }
                Token::Newline => {
                    self.saw_newline();
                    continue;
                }
                // seperates { [field]: [value] , [field] [type] }
                Token::Comma => continue,
                Token::Space => continue,
                Token::Id(id_name) => {
                    self.skip(Token::Space);

                    if let Some(Token::Colon) = self.lexer.next() {
                        match self.next() {
                            Some(node) => match node {
                                Node::Expression(expr) => {
                                    data_values.insert(id_name, expr);
                                }
                                Node::Definition(_) => todo!(),
                                Node::Statement(_) => todo!(),
                            },
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

        Some(GinValue::Object(data_values))
    }

    fn handle_data_type(&mut self, tag_name: String) -> Option<DataDefiniton> {
        self.skip(Token::Space);

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

        let mut data_definiton = DataDefiniton::new(tag_name);
        loop {
            self.handle_indentation();
            // self.eat(Token::Space);

            let Some(token) = self.lexer.next() else {
                break;
            };

            match token {
                Token::CurlyClose => break,
                Token::Newline => continue,
                Token::Comma => continue,
                Token::Id(id_name) => {
                    self.skip(Token::Space);

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

                    data_definiton.insert(id_name, gin_type);
                }

                unknown => panic!(
                    "Unexpected token [{unknown:#?}] at {}",
                    self.lexer.location()
                ),
            }
        }

        Some(data_definiton)
    }

    fn check_for_op(&mut self, token: &Token) -> Option<Op> {
        self.skip(Token::Space);
        match token {
            Token::SlashBack => Some(Op::Bin(Binary::Div)),
            // Token::SlashForward => Some(Op::Compare),
            Token::LessThan => Some(Op::Compare(Comparison::LessThan)),
            Token::LessThanOrEqualTo => Some(Op::Compare(Comparison::LessThanOrEqualTo)),
            Token::GreaterThan => Some(Op::Compare(Comparison::GreaterThan)),
            Token::GreaterThanOrEqualTo => Some(Op::Compare(Comparison::GreaterThanOrEqualTo)),
            Token::Plus => Some(Op::Bin(Binary::Add)),
            Token::Dash => Some(Op::Bin(Binary::Sub)),
            Token::Equals => Some(Op::Compare(Comparison::Equals)),
            // Token::Ampersand => todo!(),
            Token::Star => Some(Op::Bin(Binary::Mul)),
            // Token::Percent => todo!(),
            _ => None,
        }
    }

    fn handle_literal(&mut self, literal: GinValue) -> Option<Expr> {
        self.skip(Token::Space);
        let next_token = self.lexer.next()?;
        if next_token == Token::Newline {
            return Some(Expr::Literal(literal));
        }
        if let Token::Id(_) = next_token {
            // could be associated function
            panic!("todo associated function")
        }

        if let Some(op) = self.check_for_op(&next_token) {
            let lhs = Box::new(Expr::Literal(literal));
            self.skip(Token::Space);
            let next = self.next()?;
            if let Node::Expression(r_expr) = next {
                let rhs = Box::new(r_expr);
                Some(Expr::Operation(lhs, op, rhs))
            } else {
                panic!("failed to get expr in binop")
            }
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

    fn handle_tag(&mut self, tag_name: String) -> Option<Node> {
        self.skip(Token::Space);
        match self.lexer.next()? {
            // TODO: This could also be return type tagging
            Token::CurlyOpen => match self.handle_data_type(tag_name) {
                Some(data_def) => Some(Node::Definition(Define::Data(data_def))),
                None => todo!(),
            },
            unknown => panic!(
                "Unexpected token [{unknown:#?}] at {}",
                self.lexer.location()
            ),
        }
    }

    fn handle_token(&mut self, token: Token) -> Option<Node> {
        match token {
            Token::Newline => {
                self.saw_newline();
                self.next()
            }
            Token::Literal(lit) => match self.handle_literal(lit) {
                Some(expr) => Some(Node::Expression(expr)),
                None => panic!("failed to parse literal"),
            },
            Token::Id(name) => self.handle_id(name),
            Token::Tag(name) => self.handle_tag(name),
            Token::CurlyOpen => match self.handle_data_literal() {
                Some(data_lit) => Some(Node::Expression(Expr::Literal(data_lit))),
                None => todo!(),
            },
            Token::Comment(_) => self.next(),
            Token::DocComment(_) => self.next(),
            Token::Space => self.next(),
            Token::Keyword(keyword) => match keyword {
                lexer::token::Keyword::If => {
                    if let Some(Node::Expression(cond)) = self.next() {
                        if let Expr::Operation(_, Op::Compare(_), _) = cond {
                            let if_statement = ControlFlow::If(cond, vec![], None);

                            Some(Node::Statement(Statement::ControlFlow(if_statement)))
                        } else {
                            panic!("if statement condition must be a comparsion")
                        }
                    } else {
                        panic!("")
                    }
                }
                lexer::token::Keyword::Else => todo!(),
                lexer::token::Keyword::Then => {
                    // then marks the end of expr
                    self.next()
                }
                lexer::token::Keyword::Include => todo!(),
                lexer::token::Keyword::When => todo!(),
                lexer::token::Keyword::For => todo!(),
                lexer::token::Keyword::Return => todo!(),
            },
            unknown => panic!(
                "Unexpected token [{unknown:#?}] at {}",
                self.lexer.location()
            ),
        }
    }

    fn handle_id(&mut self, name: String) -> Option<Node> {
        self.skip(Token::Space);

        let Some(token) = self.lexer.next() else {
            let call = Call::new(name, None);
            let node = Node::Expression(Expr::Call(call));
            return Some(node);
        };

        if let Some(op) = self.check_for_op(&token) {
            let Node::Expression(r_expr) = self.next()? else {
                panic!("expected rhs of operation to be expr")
            };

            let rhs = Box::new(r_expr);
            // TODO: [call arg + call]
            let lhs = Box::new(Expr::Call(Call::new(name, None)));
            let expr = Expr::Operation(lhs, op, rhs);
            return Some(Node::Expression(expr));
        }

        match token {
            Token::Id(ident) => match self.handle_id(ident) {
                Some(node) => {
                    let Node::Expression(expr) = node else {
                        panic!("cannot pass anything but expr into function call")
                    };

                    let expr = Expr::Call(Call::new(name, Some(Box::new(expr))));
                    Some(Node::Expression(expr))
                }
                None => panic!("handle_id failed"),
            },
            Token::Colon => self.handle_assignment(name),
            Token::Newline => {
                self.saw_newline();
                // println!("got here");
                Some(Node::Expression(Expr::Call(Call::new(name, None))))
            }
            Token::Comma => Some(Node::Expression(Expr::Call(Call::new(name, None)))),
            Token::Literal(lit) => {
                let expr = Some(Box::new(Expr::Literal(lit)));
                Some(Node::Expression(Expr::Call(Call::new(name, expr))))
            }
            // function call does require a space
            Token::Space => {
                let next_node = self.next();
                match next_node {
                    Some(node) => {
                        let Node::Expression(expr) = node else {
                            panic!("cannot call anything but a function")
                        };

                        let call = Expr::Call(Call::new(name, Some(Box::new(expr))));
                        Some(Node::Expression(call))
                    }
                    None => todo!(),
                }
            }
            Token::CurlyOpen => match self.handle_data_literal() {
                Some(lit) => Some(Node::Expression(Expr::Literal(lit))),
                None => todo!(),
            },

            unknown => panic!(
                "Unexpected token [{unknown:#?}] at {}",
                self.lexer.location()
            ),
        }
    }
}

impl Iterator for Parser {
    type Item = Node;

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.lexer.next()?;
        self.handle_token(next)
    }
}
