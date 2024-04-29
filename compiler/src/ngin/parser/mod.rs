pub mod ast;
pub mod lexer;

use std::{collections::HashMap, str::FromStr};

use self::{
    ast::{
        definition::Define,
        expression::{ArithmeticExpr, Expr, RelationalExpr},
        Node,
    },
    lexer::{
        token::{Keyword, Token, TokenKind},
        Lexer,
    },
};

use super::{
    compiler_error::CompilerError, gin_type::GinType, source_file::SourceFile, value::GinValue,
};

/// The Parsers job is to convert the Token stream into an AST
/// not to perform semantic analysis.
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

    pub fn set_content(&mut self, source_file: &mut SourceFile) {
        self.lexer.set_content(source_file)
    }

    fn saw_newline(&mut self) {
        self.scope = 0;
        self.handle_indentation();
    }

    fn handle_indentation(&mut self) {
        while let Some(Ok(token)) = self.lexer.next() {
            match token.kind() {
                TokenKind::Tab => self.scope += 1,
                TokenKind::Space => {
                    let mut space_count = 1;

                    while let Some(Ok(tok)) = self.lexer.next() {
                        match tok.kind() {
                            TokenKind::Space => space_count += 1, // eat space
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
    fn skip(&mut self, kind: TokenKind) {
        if let Some(Ok(token)) = self.lexer.next() {
            if token.kind() != &kind {
                self.lexer.defer(token)
            }
        }
    }

    fn handle_multi_line_function(&mut self, name: String) -> Option<Define> {
        let starting_scope = self.scope;
        self.saw_newline();

        let mut body = Vec::new();
        while self.scope > starting_scope {
            if let Some(Ok(node)) = self.next() {
                body.push(node);
            } else {
                break;
            }
        }

        Some(Define::Function {
            name,
            body,
            returns: GinType::Nothing,
        })
    }

    /// everything to the right of `:`
    fn handle_assignment(&mut self, name: String) -> Option<Result<Node, CompilerError>> {
        self.skip(TokenKind::Space);
        let token = self.lexer.next()?;
        match token {
            Ok(tok) => {
                if tok.kind() == &TokenKind::Newline {
                    return match self.handle_multi_line_function(name) {
                        Some(def) => Some(Ok(Node::Definition(def))),
                        None => todo!(),
                    };
                }
                self.lexer.defer(tok); // return the token since we will need it later.

                // we can assume its only a single expr since if it was a func def
                // it would have been picked up above in the self.handle_multi_line_function

                let nt = self.next()?;
                match nt {
                    Ok(n) => {
                        let func = Define::Function {
                            name,
                            body: vec![n],
                            returns: GinType::Nothing,
                        };

                        Some(Ok(Node::Definition(func)))
                    }
                    Err(e) => Some(Err(e)),
                }
            }
            Err(e) => Some(Err(e)),
        }
    }

    fn handle_data_literal(&mut self) -> Option<Result<GinValue, CompilerError>> {
        let mut data_values: HashMap<String, Expr> = HashMap::new();

        loop {
            // TODO: replace this gaurd with a match - we are loosing the error here
            let Ok(token) = self.lexer.next()? else {
                return None;
            };

            match token.kind() {
                TokenKind::CurlyClose => break,
                // seperates { [field] [value] \n [field] [type] }
                TokenKind::Newline => {
                    self.saw_newline();
                    continue;
                }
                // seperates { [field]: [value] , [field] [type] }
                TokenKind::Comma => continue,
                TokenKind::Space => continue,
                TokenKind::Id(id_name) => {
                    self.skip(TokenKind::Space);
                    let Ok(nt) = self.lexer.next()? else {
                        return None;
                    };

                    if nt.kind() == &TokenKind::Colon {
                        match self.next()? {
                            Ok(n) => match n {
                                Node::Expression(expr) => {
                                    data_values.insert(id_name.to_owned(), expr);
                                }
                                Node::Definition(_) => todo!(),
                                Node::Statement(_) => todo!(),
                            },
                            Err(e) => return Some(Err(e)),
                        }
                    } else {
                        // might be a shorthand {name}
                        todo!()
                    }
                }
                unknown => {
                    return Some(Err(CompilerError::UnknownToken(
                        self.lexer.current_location(),
                        token,
                    )))
                }
            }
        }

        Some(Ok(GinValue::Object(data_values)))
    }

    fn handle_data_type(&mut self, name: String) -> Option<Result<Define, CompilerError>> {
        self.skip(TokenKind::Space);

        match self.lexer.next() {
            Some(Ok(tok)) => match tok.kind() {
                TokenKind::Newline => {
                    self.saw_newline();
                }
                _ => self.lexer.defer(tok),
            },
            Some(Err(e)) => return Some(Err(e)),
            None => panic!("finish writing your data type"),
        }

        // this is a bit eager, but we are going to assume we only
        // have one pair of curlies and there is no nesting yet.

        let mut body = HashMap::new();

        loop {
            self.handle_indentation();
            // self.eat(Token::Space);

            let Some(Ok(token)) = self.lexer.next() else {
                break;
            };

            match token.kind() {
                TokenKind::CurlyClose => break,
                TokenKind::Newline => continue,
                TokenKind::Comma => continue,
                TokenKind::Id(id_name) => {
                    self.skip(TokenKind::Space);
                    let nt = self.lexer.next()?;

                    match nt {
                        Ok(inner_token) => match token.kind() {
                            TokenKind::Tag(token_type) => {
                                let gin_type = GinType::from_str(&token_type.as_str())
                                    .expect("parsed gin type from token_type");

                                body.insert(id_name.to_owned(), gin_type);
                            }
                            unknown => {
                                return Some(Err(CompilerError::UnknownToken(
                                    self.lexer.current_location(),
                                    inner_token,
                                )))
                            }
                        },

                        Err(e) => return Some(Err(e)),
                    }

                    // let TokenKind::Tag(token_type) = nt.kind() else {
                    //     // TODO: check if it is defined within an availble scope
                    //     // if it is this might be shorthand fill
                    //     //
                    //     // we might add a Fill type here that can be filled on another pass
                    //     // with more context from the document
                    //
                    //     panic!(
                    //         "failed to declare type on data field {id_name} at pos: {}",
                    //         self.lexer.location()
                    //     )
                    // };
                }

                unknown => {
                    return Some(Err(CompilerError::UnknownToken(
                        self.lexer.current_location(),
                        token,
                    )))
                }
            }
        }

        Some(Ok(Define::Record { name, body }))
    }

    fn handle_potential_binop(
        &mut self,
        token: &Token,
        lhs: Expr,
    ) -> Option<Result<Expr, CompilerError>> {
        if let Ok(Node::Expression(rhs)) = self.next()? {
            let expr = match token.kind() {
                TokenKind::Plus => {
                    let a = ArithmeticExpr::Add { lhs, rhs };
                    Expr::Arithmetic(Box::new(a))
                }
                TokenKind::Dash => {
                    let a = ArithmeticExpr::Sub { lhs, rhs };
                    Expr::Arithmetic(Box::new(a))
                }
                TokenKind::Star => {
                    let a = ArithmeticExpr::Mul { lhs, rhs };
                    Expr::Arithmetic(Box::new(a))
                }
                TokenKind::Percent => todo!(),
                TokenKind::SlashForward => {
                    let a = ArithmeticExpr::Div { lhs, rhs };
                    Expr::Arithmetic(Box::new(a))
                }
                TokenKind::LessThan => {
                    let r = RelationalExpr::LessThan { lhs, rhs };
                    Expr::Relational(Box::new(r))
                }
                TokenKind::LessThanOrEqualTo => {
                    let r = RelationalExpr::LessThanOrEqualTo { lhs, rhs };
                    Expr::Relational(Box::new(r))
                }
                TokenKind::GreaterThan => {
                    let r = RelationalExpr::GreaterThan { lhs, rhs };
                    Expr::Relational(Box::new(r))
                }
                TokenKind::GreaterThanOrEqualTo => {
                    let r = RelationalExpr::GreaterThanOrEqualTo { lhs, rhs };
                    Expr::Relational(Box::new(r))
                }
                TokenKind::Equals => {
                    let r = RelationalExpr::Equals { lhs, rhs };
                    Expr::Relational(Box::new(r))
                }

                TokenKind::CurlyClose | TokenKind::Comma => lhs,
                _unknown => {
                    // println!("got unknown: {:#?}", unknown);
                    return None;
                }
            };

            Some(Ok(expr))
        } else {
            panic!("rhs of binop was not expr")
        }
    }

    fn handle_literal(&mut self, literal: GinValue) -> Option<Result<Expr, CompilerError>> {
        self.skip(TokenKind::Space);
        let next_token = self.lexer.next()?;
        match next_token {
            Ok(nt) => match nt.kind() {
                TokenKind::Newline => Some(Ok(Expr::Literal(literal))),
                other => self.handle_potential_binop(&nt, Expr::Literal(literal)),
            },
            Err(e) => Some(Err(e)),
        }

        // if let TokenKind::Id(_) = next_token.kind() {
        //     // could be associated function
        //     panic!("todo associated function")
        // }

        // if let Some(op) = self.check_for_op(&next_token) {
        //     let lhs = Box::new(Expr::Literal(literal));
        //     self.skip(TokenKind::Space);
        //     let next = self.next()?;
        //     if let Node::Expression(r_expr) = next {
        //         let rhs = Box::new(r_expr);
        //         Some(Expr::Operation(lhs, op, rhs))
        //     } else {
        //         panic!("failed to get expr in binop")
        //     }
        // } else {
        //     match next_token.kind() {
        //         TokenKind::CurlyClose => {
        //             self.lexer.defer(next_token);
        //             Some(Expr::Literal(literal))
        //         }
        //         TokenKind::Comma => {
        //             self.lexer.defer(next_token);
        //             Some(Expr::Literal(literal))
        //         }
        //         t => {
        //             panic!("error: {:#?}", t)
        //         }
        //     }
        // }
    }

    fn handle_tag(&mut self, tag_name: String) -> Option<Result<Node, CompilerError>> {
        self.skip(TokenKind::Space);
        let nt = self.lexer.next()?;
        match nt {
            Ok(token) => match token.kind() {
                TokenKind::Keyword(Keyword::Is) => {
                    self.skip(TokenKind::Space);
                    // defining a tag-type
                    let nt = self.lexer.next()?;
                    match nt {
                        Ok(ref tok) => {
                            if tok.kind() == &TokenKind::CurlyOpen {
                                match self.handle_data_type(tag_name) {
                                    Some(Ok(def)) => Some(Ok(Node::Definition(def))),
                                    Some(Err(e)) => Some(Err(e)),
                                    None => todo!(),
                                }
                            } else {
                                todo!("{:#?}", nt)
                            }
                        }
                        Err(e) => Some(Err(e)),
                    }
                }
                _ => Some(Err(CompilerError::UnknownToken(
                    self.lexer.current_location(),
                    token,
                ))), // TODO: This could also be return type tagging
            },
            Err(e) => Some(Err(e)),
        }
    }

    fn handle_token(&mut self, token: Token) -> Option<Result<Node, CompilerError>> {
        match token.kind() {
            TokenKind::Newline => {
                self.saw_newline();
                self.next()
            }
            TokenKind::Literal(lit) => match self.handle_literal(lit.to_owned()) {
                Some(Ok(expr)) => Some(Ok(Node::Expression(expr))),
                Some(Err(e)) => Some(Err(e)),
                None => panic!("failed to parse literal"),
            },
            TokenKind::Id(name) => self.handle_id(name.to_owned()),
            TokenKind::Tag(name) => self.handle_tag(name.to_owned()),
            TokenKind::CurlyOpen => match self.handle_data_literal() {
                Some(Ok(data_lit)) => Some(Ok(Node::Expression(Expr::Literal(data_lit)))),
                Some(Err(e)) => Some(Err(e)),
                None => todo!(),
            },
            TokenKind::Comment(_) => self.next(),
            TokenKind::DocComment(_) => self.next(),
            TokenKind::Space => self.next(),
            TokenKind::Keyword(keyword) => match keyword {
                lexer::token::Keyword::If => {
                    if let Ok(Node::Expression(_cond)) = self.next()? {
                        // if let Expr::Operation(_, Op::Compare(_), _) = cond {
                        //     let if_statement = ControlFlow::If(cond, vec![], None);
                        //
                        //     Some(Node::Statement(Statement::ControlFlow(if_statement)))
                        // } else {
                        panic!("if statement condition must be a comparsion")
                        // }
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
                lexer::token::Keyword::Is => todo!(),
            },
            unknown => Some(Err(CompilerError::UnknownToken(
                self.lexer.current_location(),
                token,
            ))),
        }
    }

    fn handle_id(&mut self, name: String) -> Option<Result<Node, CompilerError>> {
        self.skip(TokenKind::Space);

        let Some(Ok(token)) = self.lexer.next() else {
            let call = Expr::Call { name, arg: None };
            let node = Node::Expression(call);
            return Some(Ok(node));
        };

        // if let Some(op) = self.check_for_op(&token) {
        //     let Node::Expression(r_expr) = self.next()? else {
        //         panic!("expected rhs of operation to be expr")
        //     };
        //
        //     let rhs = Box::new(r_expr);
        //     // TODO: [call arg + call]
        //     let lhs = Box::new(Expr::Call(Call::new(name, None)));
        //     let expr = Expr::Operation(lhs, op, rhs);
        //     return Some(Node::Expression(expr));
        // }

        match token.kind() {
            TokenKind::Id(ident) => match self.handle_id(ident.to_owned()) {
                Some(Err(e)) => Some(Err(e)),
                Some(Ok(node)) => {
                    let Node::Expression(expr) = node else {
                        panic!("cannot pass anything but expr into function call")
                    };

                    let expr = Expr::Call {
                        name,
                        arg: Some(Box::new(expr)),
                    };
                    Some(Ok(Node::Expression(expr)))
                }
                None => panic!("handle_id failed"),
            },
            TokenKind::Colon => self.handle_assignment(name),
            TokenKind::Newline => {
                self.saw_newline();
                let call = Expr::Call { name, arg: None };
                Some(Ok(Node::Expression(call)))
            }
            TokenKind::Comma => {
                let call = Expr::Call { name, arg: None };
                Some(Ok(Node::Expression(call)))
            }
            TokenKind::Literal(lit) => {
                let expr = Some(Box::new(Expr::Literal(lit.to_owned())));
                let call = Expr::Call { name, arg: expr };
                Some(Ok(Node::Expression(call)))
            }
            // function call does require a space
            TokenKind::Space => {
                let next_node = self.next();
                match next_node {
                    Some(node) => {
                        let Ok(Node::Expression(expr)) = node else {
                            // TODO: this location is probably wrong
                            // it needs to get embeded in the ast to keep its position correct
                            return Some(Err(CompilerError::CannotCallNonExpr(
                                self.lexer.current_location(),
                            )));
                        };

                        let call = Expr::Call {
                            name,
                            arg: Some(Box::new(expr)),
                        };
                        Some(Ok(Node::Expression(call)))
                    }
                    None => todo!(),
                }
            }
            TokenKind::CurlyOpen => match self.handle_data_literal() {
                Some(lit) => match lit {
                    Ok(literal) => Some(Ok(Node::Expression(Expr::Literal(literal)))),
                    Err(e) => Some(Err(e)),
                },

                None => todo!(),
            },
            TokenKind::RightArrow => {
                // optional return type
                let f = self.next();
                // println!("{:#?}", f);
                None
            }
            unknown => Some(Err(CompilerError::UnknownToken(
                self.lexer.current_location(),
                token,
            ))),
        }
    }
}

// Option<Result<Node, CompilerError>>
// ^ Some means there is more source code
// ^ None means no more source code
//         ^ Ok means good source code
//         ^ Err means bad source code

impl Iterator for Parser {
    type Item = Result<Node, CompilerError>;

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.lexer.next()?;
        match next {
            Ok(token) => self.handle_token(token),
            Err(compiler_error) => Some(Err(compiler_error)),
        }
    }
}
