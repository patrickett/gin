use super::{
    ast::{
        definition::{Define, Function, Parameter, Record},
        expression::{ArithmeticExpr, Expr},
        Node,
    },
    lex::LexedFile,
    token::{Keyword, Token, TokenKind},
};
use crate::{
    compiler_error::CompilerError, gin_type::GinType, syntax::ast::expression::FunctionCall,
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
                _ => return Err(CompilerError::UnknownToken(token.clone(), None)),
            };
            nodes.push(node);
        }

        Ok(ParsedFile { nodes })
    }

    fn node(&mut self, tokens: &mut Peekable<Iter<'_, Token>>) -> Result<Node, CompilerError> {
        // starting to wonder if spaces are ever context dependent
        // newlines are for sure. but spaces currently only helpful for lexer
        self.skip_space(tokens);
        let token = self.next_token(tokens)?.clone();

        let expr = match token.kind() {
            TokenKind::Literal(lhs) => {
                let lhs = Expr::Literal(lhs.clone());
                self.skip_space(tokens);
                let token = self.next_token(tokens)?.clone();
                match token.kind() {
                    TokenKind::Plus => {
                        if let Node::Expression(rhs) = self.node(tokens)? {
                            let e = ArithmeticExpr::Add { lhs, rhs };
                            Node::Expression(Expr::Arithmetic(Box::new(e)))
                        } else {
                            return Err(CompilerError::UnknownToken(token, None));
                        }
                    }
                    TokenKind::Dash => {
                        if let Node::Expression(rhs) = self.node(tokens)? {
                            let e = ArithmeticExpr::Sub { lhs, rhs };
                            Node::Expression(Expr::Arithmetic(Box::new(e)))
                        } else {
                            return Err(CompilerError::UnknownToken(token, None));
                        }
                    }
                    TokenKind::Star => {
                        if let Node::Expression(rhs) = self.node(tokens)? {
                            let e = ArithmeticExpr::Mul { lhs, rhs };
                            Node::Expression(Expr::Arithmetic(Box::new(e)))
                        } else {
                            return Err(CompilerError::UnknownToken(token, None));
                        }
                    }
                    TokenKind::SlashForward => {
                        if let Node::Expression(rhs) = self.node(tokens)? {
                            let e = ArithmeticExpr::Div { lhs, rhs };
                            Node::Expression(Expr::Arithmetic(Box::new(e)))
                        } else {
                            return Err(CompilerError::UnknownToken(token, None));
                        }
                    }
                    TokenKind::Newline => Node::Expression(lhs),
                    _ => return Err(CompilerError::UnknownToken(token.clone(), None)),
                }
            }
            TokenKind::Id(func_name) => self.id(tokens, func_name)?,
            _ => return Err(CompilerError::UnknownToken(token.clone(), None)),
        };

        Ok(expr)
    }

    /// parse the body of a function
    fn func(
        &mut self,
        tokens: &mut Peekable<Iter<'_, Token>>,
        name: &str,
        arg: Option<Parameter>,
    ) -> Result<Function, CompilerError> {
        self.skip_space(tokens);
        let mut function = Function::new(name.to_string(), arg, None);

        if let Some(_newline) = tokens.next_if(|t| t.kind() == &TokenKind::Newline) {
            // Multi-line function
        } else {
            // note: i am just assuming anything rhs will be an expr
            // this could be wrong
            let node = self.node(tokens)?;
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
        let token = self.next_token(tokens)?.clone();
        let node = match token.kind() {
            TokenKind::Newline => {
                // fnCall, ex. doAction (no arg)
                let func_call = FunctionCall::new(id.to_string(), None);
                Node::Expression(Expr::Call(func_call))
            }
            // fn arg
            //      ^ could also have another + after
            TokenKind::Id(arg_name) => {
                if let Some(_colon) = tokens.next_if(|t| t.kind() == &TokenKind::Colon) {
                    // fn arg Colon
                    let func = self.func(
                        tokens,
                        id,
                        Some(Parameter::new(arg_name.to_string(), GinType::Nothing)),
                    )?;

                    Node::Definition(Define::Function(func))
                } else if let Node::Expression(rhs) = self.id(tokens, arg_name)? {
                    let func_call = FunctionCall::new(id.to_string(), Some(Box::new(rhs)));
                    Node::Expression(Expr::Call(func_call))
                } else {
                    return Err(CompilerError::UnknownToken(token, None));
                }
            }
            // fn + fn
            TokenKind::Plus => {
                let func_call = FunctionCall::new(id.to_string(), None);
                let lhs = Expr::Call(func_call);
                if let Node::Expression(rhs) = self.node(tokens)? {
                    let e = ArithmeticExpr::Add { lhs, rhs };
                    Node::Expression(Expr::Arithmetic(Box::new(e)))
                } else {
                    return Err(CompilerError::UnknownToken(token, None));
                }
            }

            // fn: [body]
            // doesnt support arg
            TokenKind::Colon => Node::Definition(Define::Function(self.func(tokens, id, None)?)),
            _ => return Err(CompilerError::UnknownToken(token.clone(), None)),
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
                                            record.body.insert(property_name.clone(), property_type)
                                        }
                                        Err(_) => {
                                            // panic!("type error");
                                            return Err(CompilerError::UnknownToken(
                                                token.clone(),
                                                None,
                                            ));
                                        }
                                    }
                                }
                                _ => return Err(CompilerError::UnknownToken(token.clone(), None)),
                            };
                        }
                        _ => return Err(CompilerError::UnknownToken(token.clone(), None)),
                    };
                }

                Ok(Define::Record(record))
            }
            // - Computed (generic arg + -> bool)
            TokenKind::Id(_generic_argument) => {
                self.skip_space(tokens);
                let token = self.next_token(tokens)?;
                match token.kind() {
                    TokenKind::Keyword(Keyword::Where) => todo!(),
                    _ => Err(CompilerError::UnknownToken(token.clone(), None)),
                }
            }
            // - Range
            // - Union (tagged or untagged)
            //      tagged would be Day is
            //                          - Monday
            //                          - Tuesday
            //      untagged would be
            //      Return is "hello" | 5
            _ => Err(CompilerError::UnknownToken(token.clone(), None)),
        }
    }

    /// Returning tagged data
    /// -> Result<Expr, CompilerError>
    /// ex. Data <expr>
    fn tag_return(&self) {
        todo!()
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
                _ => return Err(CompilerError::UnknownToken(token.clone(), None)),
            },
            _ => return Err(CompilerError::UnknownToken(token.clone(), None)),
        };

        Ok(node)
    }
}
