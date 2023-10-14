use core::panic;
use std::{collections::HashMap, iter::Peekable, slice::Iter, str::FromStr};

use crate::lex::{Literal, Token};

#[derive(Debug, Clone)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    // Mod,
    // Exp,
    // Eq,
    // NotEq,
    // GreaterThan,
    // LessThan,
    // GreaterThanOrEq,
    // LessThanOrEq,
    // LogicalAnd,
    // LogicalOr,
    // BitwiseAnd,
    // BitwiseOr,
    // BitwiseXor,
    // BitwiseLeftShift,
    // BitwiseRightShift,
}
#[derive(Debug, Clone)]
pub enum OutputKind {
    Bool,
    List,
    Object,
    String,
    Number,
    Custom(String),
    Nothing,
}

impl FromStr for OutputKind {
    type Err = ();

    fn from_str(input: &str) -> Result<OutputKind, Self::Err> {
        match input {
            "string" => Ok(OutputKind::String),
            "number" => Ok(OutputKind::Number),
            kind => Ok(OutputKind::Custom(kind.to_string())),
        }
    }
}

// add {x,y}
// add params
// add name string

// The difference is that when passing an argument into a function we dont want to allow types
// so we cant
//
// add { x: 3 number, y: 4 number }
//
// should always be
//
// add { x: 3, y: 4 }
//
#[derive(Debug, Clone)]
pub enum Def {
    // These are actually statements, all definitions
    Fn {
        name: String,
        input: Option<Parameter>,
        output_kind: OutputKind,
        body: Vec<ExprOrDef>,
    },

    Struct(Struct),

    List {
        name: String,
        items: Vec<Expr>,
    },

    Union {
        name: String,
        items: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub struct Struct {
    name: String,
    // <key, Type/Value>
    kv: HashMap<String, Expr>,
}

// Only used in function calls
#[derive(Debug, Clone)]
pub enum Argument {
    Struct(Struct),
    Symbol(Symbol),
    Literal(Literal),
}

#[derive(Debug, Clone)]
struct Symbol {
    name: String,
    kind: Option<OutputKind>,
}

// Only used in function defs
#[derive(Debug, Clone)]
pub enum Parameter {
    Object(Vec<Symbol>),
    Symbol(Symbol),
}

// Used within definitions
#[derive(Debug, Clone)]
pub enum Expr {
    Binary {
        op: Op,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },

    Call {
        name: String,
        arg: Option<Argument>,
    },

    Literal(Literal),

    Conditional {
        cond: Box<Expr>, // assume it needs to resolve to true
                         // consequence: Box:Expr
                         // alternative Box:Expr
    },
}

#[derive(Debug, Clone)]
pub enum ExprOrDef {
    Expr(Expr),
    Def(Def),
}

pub struct Parser {
    lines: Vec<Vec<Token>>,
    line_index: usize,
    // content: Vec<ExprOrDef>,
}

impl Parser {
    pub fn new() -> Self {
        Self {
            line_index: 0,
            lines: Vec::new(),
            // content: Vec::new(),
        }
    }

    fn parse_output_kind(&self, val: &str) -> OutputKind {
        match val {
            "" => OutputKind::Nothing,
            " " => OutputKind::Nothing,
            "string" => OutputKind::String,
            "number" => OutputKind::Number,
            custom => OutputKind::Custom(custom.to_string()),
        }
    }

    fn parse_function_symbol(&self, tokens: &[Token]) -> Option<Symbol> {
        if let Some(Token::Id(name)) = tokens.get(0) {
            let mut kind = Some(OutputKind::Nothing);
            if let Some(Token::Id(k)) = tokens.get(1) {
                kind = Some(self.parse_output_kind(k))
            }

            Some(Symbol {
                name: name.to_owned(),
                kind,
            })
        } else {
            // panic!(
            //     "ERROR: Unknown parameter count expected 1-2 found: {}",
            //     tokens_count
            // );
            None
        }
    }

    fn tab_count(&self, tokens: &[Token]) -> usize {
        tokens
            .iter()
            .take_while(|&token| token.to_owned() == Token::Tab)
            .count()
    }

    fn parse_fn_signature(
        &self,
        name: String,
        params: &[Token],
        scope_level: usize,
        output_kind: OutputKind,
    ) -> Option<(ExprOrDef, usize)> {
        let params: Vec<Symbol> = params
            .split(|t| t.to_owned() == Token::Comma)
            .map(|c| self.parse_function_symbol(c))
            .flatten()
            .collect();

        let result = self.lines.iter().enumerate().find(|&(idx, future_line)| {
            let tabs = self.tab_count(future_line);
            if tabs <= scope_level && self.line_index < idx {
                true
            } else {
                false
            }
        });

        // find the next line > self.line_index that does not start with tab
        if let Some((future_idx, _)) = result {
            let mut body: Vec<ExprOrDef> = Vec::new();

            let body_lines = &self.lines[self.line_index+1..future_idx];
            for body_line in body_lines {
                // println!("bodyLine: {:#?}", body_line);
                if let Some((expr, i)) = self.parse_line(body_line.to_vec()) {
                    body.push(expr);
                }
            }

            let func = Def::Fn {
                name,
                input: Some(Parameter::Object(params)),
                output_kind,
                body,
            };

            // return the index for the line before the one without the tab
            return Some((ExprOrDef::Def(func), future_idx - 1));
        }

        // self.buffer.clear(); // remove current fn def line
        None
    }

    // usize here allows us to skip lines that we already pulled in
    fn parse_line(&self, line: Vec<Token>) -> Option<(ExprOrDef, usize)> {
        let scope_level = self.tab_count(&line);
        // line.retain(|t| t.to_owned() != Token::Tab);
        // let line = line.iter().filter(|t| t.to_owned() != Token::Tab);
        // when we encounter a fn def we need to get everyline with a tab_count >= count
        // this will be the body for the fn
        let line: Vec<Token> = line
            .into_iter()
            .filter(|t| {
                let tok = t.to_owned();
                if tok != Token::Space && tok != Token::Tab {
                    true
                } else {
                    false
                }
            })
            .collect();

        if line.len() == 3 {
            if let Some(Token::Plus) = line.get(1) {
                let l_token = line.get(0).expect("Failed to get lhs token");
                let r_token = line.get(2).expect("Failed to get rhs token");

                let (lhs, _) = self
                    .parse_line(vec![l_token.clone()])
                    .expect("Failed to parse lhs token");
                let (rhs, _) = self
                    .parse_line(vec![r_token.clone()])
                    .expect("Failed to parse rhs token");

                if let (ExprOrDef::Expr(e1), ExprOrDef::Expr(e2)) = (lhs, rhs) {
                    let bin_add = Expr::Binary {
                        op: Op::Add,
                        lhs: Box::new(e1),
                        rhs: Box::new(e2),
                    };

                    return Some((ExprOrDef::Expr(bin_add), self.line_index));
                }
            }
        }

        match &line[..] {
            [Token::Id(tok_one), Token::Plus, Token::Id(tok_two)] => {
                let expr = Expr::Binary {
                    op: Op::Add,
                    lhs: Box::new(Expr::Call {
                        name: tok_one.to_string(),
                        arg: None,
                    }),
                    rhs: Box::new(Expr::Call {
                        name: tok_two.to_string(),
                        arg: None,
                    }),
                };
                // self.buffer.clear();
                Some((ExprOrDef::Expr(expr), self.line_index))
            }
            [Token::Literal(tok_one), Token::Plus, Token::Literal(tok_two)] => {
                let expr = Expr::Binary {
                    op: Op::Add,
                    lhs: Box::new(Expr::Literal(tok_one.clone())),
                    rhs: Box::new(Expr::Literal(tok_two.clone())),
                };
                // self.buffer.clear();
                Some((ExprOrDef::Expr(expr), self.line_index))
            }
            [Token::Id(fn_name), Token::Colon, rest @ ..] => {
                let mut body = Vec::new();

                if let Some((expr, _)) = self.parse_line(rest.to_vec()) {
                    body.push(expr);
                }

                let func = Def::Fn {
                    name: fn_name.to_string(),
                    input: None,
                    output_kind: OutputKind::Nothing,
                    body,
                };

                // self.buffer.clear();
                Some((ExprOrDef::Def(func), self.line_index))
            }
            [Token::Id(fn_name), Token::Id(param_name), Token::Id(param_type), Token::Colon] => {
                // TODO: needs fixed:
                let param = Parameter::Symbol(Symbol {
                    name: param_name.to_owned(),
                    kind: Some(self.parse_output_kind(param_type)),
                });

                let func = Def::Fn {
                    name: fn_name.to_string(),
                    input: Some(param),
                    output_kind: OutputKind::Nothing,
                    body: Vec::new(),
                };

                // self.buffer.clear();
                Some((ExprOrDef::Def(func), self.line_index))
            }

            [Token::Id(fn_name), Token::Id(param_name), Token::Colon] => {
                // TODO: needs fixed:
                // let param = Parameter::Symbol(Symbol {
                //     name: param_name.to_owned(),
                //     kind: Some(self.parse_output_kind(param_name)),
                // });
                //
                let params = vec![Token::Id(param_name.to_string())];
                // let func = Def::Fn {
                //     name: fn_name.to_string(),
                //     input: Some(param),
                //     output_kind: OutputKind::Nothing,
                //     body: Vec::new(),
                // };
                let output_kind = OutputKind::Nothing;
                self.parse_fn_signature(fn_name.to_string(), &params, scope_level, output_kind)

                // self.buffer.clear();
                // Some((ExprOrDef::Def(func), self.line_index))
            }
            [Token::Id(fn_name), Token::CurlyOpen, params @ .., Token::CurlyClose, Token::Colon] => {
                // no return type - anon object function def
                // we also know its a root level function, not a fn within a fn
                let output_kind = OutputKind::Nothing;
                self.parse_fn_signature(fn_name.to_string(), params, scope_level, output_kind)
            }
            [Token::Id(fn_name), Token::CurlyOpen, params @ .., Token::CurlyClose, Token::RightArrow, Token::Id(output), Token::Colon] =>
            {
                // no return type - anon object function def
                // we also know its a root level function, not a fn within a fn
                let output_kind = self.parse_output_kind(output);
                self.parse_fn_signature(fn_name.to_string(), params, scope_level, output_kind)
            }
            [Token::Id(fn_call), Token::Id(symbol)] => {
                let fn_call = Expr::Call {
                    name: fn_call.to_string(),
                    arg: Some(Argument::Symbol(Symbol {
                        name: symbol.to_string(),
                        kind: None,
                    })),
                };
                // self.buffer.clear();
                Some((ExprOrDef::Expr(fn_call), self.line_index))
            }
            [Token::Id(fn_call), Token::Literal(lit)] => {
                let fn_call = Expr::Call {
                    name: fn_call.to_string(),
                    arg: Some(Argument::Literal(lit.clone())),
                };
                // self.buffer.clear();
                Some((ExprOrDef::Expr(fn_call), self.line_index))
            }
            [Token::Id(fn_call)] => {
                let fn_call = Expr::Call {
                    name: fn_call.to_string(),
                    arg: None,
                };
                // self.buffer.clear();
                Some((ExprOrDef::Expr(fn_call), self.line_index))
            }
            [Token::Comment(_)] => None,
            [Token::Literal(lit)] => {
                let ex = Expr::Literal(lit.clone());

                Some((ExprOrDef::Expr(ex), self.line_index))
            }
            [] => None,
            line => {
                println!("line: {:#?}", line);
                std::process::abort();
            }
        }
    }

    // it might not be for a module, so we just return the module body.
    // they caller can create the module if that is their intent
    pub fn parse(&mut self, tokens: &[Token]) -> Vec<ExprOrDef> {
        let mut val: Vec<Vec<Token>> = tokens
            .clone()
            .into_iter()
            .filter(|&t| match t.to_owned() {
                Token::Comment(_) => false,
                _ => true,
            })
            .map(|a| a.to_owned())
            .collect::<Vec<Token>>()
            .split(|t| t.to_owned() == Token::Newline)
            .map(|g| g.to_vec())
            .collect::<Vec<Vec<Token>>>();

        val.retain(|v| !v.is_empty());

        self.lines = val;

        let mut ast: Vec<ExprOrDef> = Vec::new();

        while self.line_index < self.lines.len() {
            if let Some(line) = self.lines.get(self.line_index) {
                if let Some((expr, index)) = self.parse_line(line.to_vec()) {
                    ast.push(expr);
                    self.line_index = index;
                }
            }
            self.line_index += 1;
        }

        // for line in &self.lines {}

        // self.content.clone()
        ast
    }
}


