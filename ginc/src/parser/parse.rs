use super::{
    ast::{
        definition::{Define, Function, Parameter},
        expression::{ArithmeticExpr, Expr},
        Node,
    },
    lex::LexedFile,
    lexer::token::{Token, TokenKind},
};
use crate::{
    compiler_error::CompilerError,
    gin_type::GinType,
    parser::{ast::definition::Record, lexer::token::Keyword},
};
use std::{iter::Peekable, slice::Iter, str::FromStr};

pub struct SimpleParser;

pub struct ParsedFile {
    pub nodes: Vec<Node>,
}

impl<'a> SimpleParser {
    /// Check the next token and if it matches the argument then skip it
    fn skip(&mut self, tokens: &mut Peekable<Iter<'_, Token>>, kind: TokenKind) {
        if let Some(token) = tokens.peek() {
            let tkind = token.kind();
            // println!("tried to skip {} saw {}", kind, tkind);
            if tkind == &kind {
                // println!("skipped. matched {} with {}", kind, tkind);
                tokens.next();
            }
        }
    }

    fn skip_space(&mut self, tokens: &mut Peekable<Iter<'_, Token>>) {
        self.skip(tokens, TokenKind::Space)
    }

    pub fn parse(&mut self, lexed_file: &LexedFile) -> Result<ParsedFile, CompilerError> {
        let mut tokens = lexed_file.tokens.iter().peekable();

        let mut nodes: Vec<Node> = Vec::new();
        while let Some(token) = tokens.next() {
            let node = match token.kind() {
                // skip comments for now
                TokenKind::Comment(_) => continue,
                TokenKind::DocComment(_) => continue,
                TokenKind::Newline => continue,
                TokenKind::Space => continue,
                TokenKind::Id(id) => self.id(&mut tokens, id)?,
                TokenKind::Tag(tag) => self.tag(tag, &mut tokens)?,
                _ => return Err(self.unknown_token(token.clone())),
            };
            nodes.push(node);
        }

        Ok(ParsedFile { nodes })
    }

    fn expr(&mut self, tokens: &mut Peekable<Iter<'_, Token>>) -> Result<Expr, CompilerError> {
        // starting to wonder if spaces are ever context dependent
        // newlines are for sure. but spaces currently only helpful for lexer
        self.skip_space(tokens);
        let token = self.next_token(tokens)?;

        let expr = match token.kind() {
            TokenKind::Literal(lhs) => {
                let lhs = Expr::Literal(lhs.clone());
                self.skip_space(tokens);
                let token = self.next_token(tokens)?;
                match token.kind() {
                    TokenKind::Plus => {
                        let rhs = self.expr(tokens)?;
                        let e = ArithmeticExpr::Add { lhs, rhs };
                        Expr::Arithmetic(Box::new(e))
                    }
                    TokenKind::Dash => {
                        let rhs = self.expr(tokens)?;
                        let e = ArithmeticExpr::Sub { lhs, rhs };
                        Expr::Arithmetic(Box::new(e))
                    }
                    TokenKind::Star => {
                        let rhs = self.expr(tokens)?;
                        let e = ArithmeticExpr::Mul { lhs, rhs };
                        Expr::Arithmetic(Box::new(e))
                    }
                    TokenKind::SlashForward => {
                        let rhs = self.expr(tokens)?;
                        let e = ArithmeticExpr::Div { lhs, rhs };
                        Expr::Arithmetic(Box::new(e))
                    }
                    TokenKind::Newline => lhs,

                    _ => return Err(self.unknown_token(token.clone())),
                }
            }
            TokenKind::Id(func_call) => {
                let token = self.next_token(tokens)?;
                let expr = match token.kind() {
                    // fn arg
                    //      ^ could also have another + after
                    TokenKind::Id(argument) => todo!(),
                    // fn + fn
                    TokenKind::Plus => {
                        let rhs = self.expr(tokens)?;
                    }

                    _ => return Err(self.unknown_token(token.clone())),
                };

                todo!()
            }
            _ => return Err(self.unknown_token(token.clone())),
        };

        Ok(expr)
    }

    fn multi_line_func(
        &mut self,
        tokens: &mut Peekable<Iter<'_, Token>>,
    ) -> Result<Node, CompilerError> {
        // continue until we don't start with a tab
        todo!()
    }

    /// parse the body of a function
    fn func(
        &mut self,
        tokens: &mut Peekable<Iter<'_, Token>>,
        name: &str,
        arg: Option<Parameter>,
    ) -> Result<Function, CompilerError> {
        self.skip_space(tokens);
        let mut function = Function::new(name.to_string(), arg, GinType::Nothing);

        if let Some(_newline) = tokens.next_if(|t| t.kind() == &TokenKind::Newline) {
            // Multi-line function
        } else {
            // note: i am just assuming anything rhs will be an expr
            // this could be wrong
            let expr = self.expr(tokens)?;
            let node = Node::Expression(expr);
            function.body.push(node);
        }
        Ok(function)
    }

    fn id(
        &mut self,
        tokens: &mut Peekable<Iter<'_, Token>>,
        id: &str,
    ) -> Result<Node, CompilerError> {
        self.skip_space(tokens);
        let token = self.next_token(tokens)?;
        let node = match token.kind() {
            TokenKind::Colon => Node::Definition(Define::Function(self.func(tokens, id, None)?)),
            _ => return Err(self.unknown_token(token.clone())),
        };

        Ok(node)
    }

    fn next_token(
        &'a self,
        tokens: &'a mut Peekable<Iter<'_, Token>>,
    ) -> Result<&Token, CompilerError> {
        let Some(token) = tokens.next() else {
            return Err(CompilerError::UnexpectedEOF);
        };
        Ok(token)
    }

    /// Defining a tag
    fn tag_define(
        &mut self,
        tag: &str,
        tokens: &mut Peekable<Iter<'_, Token>>,
    ) -> Result<Define, CompilerError> {
        self.skip_space(tokens);
        let token = self.next_token(tokens)?;

        match token.kind() {
            // - Record / shape of data
            TokenKind::CurlyOpen => {
                let mut record = Record::new(tag.to_string());
                while let Some(token) = tokens.next() {
                    self.skip_space(tokens);
                    match token.kind() {
                        TokenKind::CurlyClose => break,
                        TokenKind::Comma => continue,
                        TokenKind::Newline => continue,
                        TokenKind::Tab => continue,
                        TokenKind::Id(property_name) => {
                            self.skip_space(tokens);
                            let token = self.next_token(tokens)?;
                            match token.kind() {
                                TokenKind::Newline => continue,
                                TokenKind::Tag(property_type) => {
                                    match GinType::from_str(property_type) {
                                        Ok(property_type) => {
                                            // println!(
                                            //     "inserted {{ {}: {:#?} }}",
                                            //     &property_name, property_type
                                            // );
                                            record.insert(property_name.clone(), property_type)
                                        }
                                        Err(_) => {
                                            // panic!("type error");
                                            return Err(self.unknown_token(token.clone()));
                                        }
                                    }
                                }
                                _ => return Err(self.unknown_token(token.clone())),
                            };
                        }
                        _ => return Err(self.unknown_token(token.clone())),
                    };
                }

                Ok(Define::Record(record))
            }
            // - Computed (generic arg + -> bool)
            TokenKind::Id(generic_argument) => {
                self.skip_space(tokens);
                let token = self.next_token(tokens)?;
                match token.kind() {
                    TokenKind::Keyword(Keyword::Where) => todo!(),
                    _ => return Err(self.unknown_token(token.clone())),
                }
            }
            // - Range
            // - Union (tagged or untagged)
            //      tagged would be Day is
            //                          - Monday
            //                          - Tuesday
            //      untagged would be
            //      Return is "hello" | 5
            _ => Err(self.unknown_token(token.clone())),
        }
    }

    /// Returning tagged data
    /// -> Result<Expr, CompilerError>
    /// ex. Data <expr>
    fn tag_return(&self) {
        todo!()
    }

    fn unknown_token(&self, token: Token) -> CompilerError {
        // let mut tokens = tokens;
        // let token = self.get_token(&mut tokens).unwrap();
        // let path = self.path();
        let path = Some("".to_string()); // TODO: in the future find the actual path
        CompilerError::UnknownToken(token, path)
    }

    fn tag(
        &mut self,
        tag: &str,
        tokens: &mut Peekable<Iter<'_, Token>>,
    ) -> Result<Node, CompilerError> {
        let mut tokens = tokens;
        self.skip_space(tokens);
        let token = self.next_token(tokens)?;
        let node = match token.kind() {
            // range literal
            TokenKind::Range(_, _) => todo!(),
            // tuple literal?
            TokenKind::ParenOpen => todo!(),
            // object literal ex. Person { name: "john" }
            TokenKind::CurlyOpen => todo!(),
            // list literal
            TokenKind::BracketOpen => todo!(),
            // method call/calls ex.  Data file.get path
            TokenKind::Id(_id) => todo!(),
            // subtype ... return People List Person
            TokenKind::Tag(_) => todo!(),
            // lit ex. Name "john"
            TokenKind::Literal(_) => todo!(),
            TokenKind::Keyword(keyword) => match keyword {
                // This is the only case where we are defining something
                // everything else is treated as returning tagged data
                Keyword::Is => Node::Definition(self.tag_define(tag, &mut tokens)?),
                _ => return Err(self.unknown_token(token.clone())),
            },
            _ => return Err(self.unknown_token(token.clone())),
        };

        Ok(node)
    }
}
