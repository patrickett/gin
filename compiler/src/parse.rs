use std::{
    collections::HashMap,
    iter::{Map, Peekable},
    slice::Iter,
    str::FromStr,
};

use crate::lex::{Literal, Token, TokenKind};

const TAB_SIZE: usize = 4;

#[derive(Debug, Clone)]
pub enum GinType {
    Bool,
    List,
    Object(HashMap<String, GinType>),
    String,
    Number,
    // TODO: If a literal is unchanged in a function we should be able to return the actual value
    // we will refer to this as a constant since it is constant and unchanged
    // ConstantString(String),
    // ConstantNumber(usize),
    // ConstantObject(Map<String,>)
    Custom(String),
    Nothing,
}

impl FromStr for GinType {
    type Err = ();

    fn from_str(input: &str) -> Result<GinType, Self::Err> {
        match input {
            "number" => Ok(GinType::Number),
            "string" => Ok(GinType::String),
            "bool" => Ok(GinType::Bool),
            custom => Ok(GinType::Custom(custom.into())),
        }
    }
}

#[derive(Debug, Clone)]
pub enum FnArg {
    String(String),
    Number(usize),
    Id(String),
}

#[derive(Debug, Clone)]
pub enum Expr {
    Literal(Literal),

    /// Object Name, Object Type Defintions
    ObjectDefinition(String, HashMap<String, GinType>),

    ObjectLiteral(HashMap<String, Expr>),

    /// Name, Body, ReturnType
    FunctionDefinition(String, Vec<Expr>, GinType),
    /// FunctionName, Argument
    FunctionCall(String, Option<FnArg>),
}

impl Expr {
    fn gin_type(&self) -> GinType {
        match self {
            Expr::Literal(lit) => match lit {
                Literal::String(_) => GinType::String,
                Literal::Number(_) => GinType::Number,
            },
            Expr::ObjectLiteral(obj) => {
                let mut obj_def: HashMap<String, GinType> = HashMap::new();

                for (key, expr) in obj.iter() {
                    obj_def.insert(key.clone(), expr.gin_type());
                }

                GinType::Object(obj_def)
            }
            Expr::ObjectDefinition(_, _) => GinType::Nothing,
            Expr::FunctionDefinition(_, _, _) => GinType::Nothing,
            Expr::FunctionCall(_, _) => GinType::Nothing,
        }
    }
}

pub struct Parser {
    tokens: Vec<Token>,
    index: usize,
    line_number: usize,
    scope: usize,
}

impl Parser {
    pub fn new() -> Self {
        Self {
            tokens: vec![],
            index: 0,
            scope: 0,
            line_number: 0,
        }
    }

    fn current(&self) -> Option<Token> {
        if self.index < self.tokens.len() {
            let item = Some(self.tokens[self.index - 1].clone());
            item
        } else {
            None
        }
    }

    fn next_kind(&mut self) -> Option<TokenKind> {
        if self.index < self.tokens.len() {
            let item = self.tokens[self.index].clone();
            self.index += 1;
            Some(item.kind())
        } else {
            None
        }
    }

    fn panic_position(&self) {
        if let Some(token) = self.current() {
            panic!(
                "Unexpected ({:?}) at positon {} line {}",
                token.kind(),
                token.pos(),
                self.line_number
            )
        }
    }

    fn peek(&self) -> Option<Token> {
        if self.index < self.tokens.len() {
            let item = Some(self.tokens[self.index].clone());
            item
        } else {
            None
        }
    }

    fn peek_kind(&self) -> Option<TokenKind> {
        if self.index < self.tokens.len() {
            let item = self.tokens[self.index].clone();
            Some(item.kind())
        } else {
            None
        }
    }

    fn saw_newline(&mut self) {
        // println!("self.saw_newline");
        self.line_number += 1;
        self.scope = 0;
    }

    /// handle tabs and indentation
    fn handle_indentation(&mut self) {
        // println!("self.handle_indentation");
        loop {
            if let Some(tok) = self.peek_kind() {
                // self.next();
                match tok {
                    TokenKind::Tab => self.scope += 1,
                    TokenKind::Space => {
                        let mut space_count = 1;

                        loop {
                            // println!("space loop");
                            if let Some(TokenKind::Space) = self.peek_kind() {
                                self.next(); // eat the space
                                space_count += 1;
                            } else {
                                break;
                            }
                        }
                        self.scope = space_count / TAB_SIZE
                    }
                    _ => break,
                }
            } else {
                break;
            }
        }
        // println!("scope_level: {}", self.scope);
    }

    fn find_implicit_return_type(&self, body: &Vec<Expr>) -> GinType {
        if let Some(t) = body.last() {
            match t {
                // if we get a fncall we need find its decl
                // then we return its return type
                Expr::FunctionCall(f_name_call, _) => {
                    let e = body.iter().find(|e| match e {
                        Expr::ObjectDefinition(_, _) => false,
                        Expr::Literal(_) => false,
                        Expr::FunctionDefinition(f_name_def, _, _) => f_name_def == f_name_call,
                        Expr::FunctionCall(_, _) => false,
                        Expr::ObjectLiteral(_) => false,
                    });

                    if let Some(expr) = e {
                        if let Expr::FunctionDefinition(_, _, r_type) = expr {
                            return r_type.to_owned();
                        }
                    }
                    GinType::Nothing
                }
                expr => expr.gin_type(),
            }
        } else {
            GinType::Nothing
        }
    }

    fn eat(&mut self, token_kind: TokenKind) {
        if let Some(kind) = self.peek_kind() {
            if kind == token_kind {
                self.next(); // eat token
                             // println!("ate {:#?}", token_kind);
            }
        }
    }

    fn handle_id(&mut self, name: String) -> Option<Expr> {
        // println!("self.handle_id({})", &name);
        if let Some(kind) = self.next_kind() {
            match kind {
                TokenKind::Colon => {
                    self.eat(TokenKind::Space);

                    // after a :
                    if let Some(tok) = self.next() {
                        let mut body = Vec::new();

                        match tok.kind() {
                            TokenKind::Newline => {
                                // multiline fn
                                self.saw_newline();
                                self.handle_indentation();

                                if let Some(kind) = self.next_kind() {
                                    match kind {
                                        TokenKind::Id(n) => {
                                            if let Some(xpr) = self.handle_id(n) {
                                                body.push(xpr)
                                            }
                                            self.handle_indentation();
                                            loop {
                                                if self.scope > 0 {
                                                    if let Some(xpr) = self.expr() {
                                                        body.push(xpr);
                                                    }
                                                } else {
                                                    break;
                                                }
                                            }

                                            let return_type = self.find_implicit_return_type(&body);

                                            // implicit return type
                                            // return the type of the last expression in the function body

                                            let xpr =
                                                Expr::FunctionDefinition(name, body, return_type);

                                            // we finished the line
                                            Some(xpr)
                                        }
                                        TokenKind::Literal(lit) => {
                                            if let Some(TokenKind::Newline) = self.next_kind() {
                                                self.saw_newline();
                                                body.push(Expr::Literal(lit.clone()));

                                                let return_type =
                                                    self.find_implicit_return_type(&body);

                                                let xpr = Expr::FunctionDefinition(
                                                    name,
                                                    body,
                                                    return_type,
                                                );

                                                // we finished the line
                                                Some(xpr)
                                            } else {
                                                self.panic_position();
                                                None
                                            }
                                        }

                                        _ => panic!(
                                            "Unexpected ({:?}) at positon {} line {}",
                                            tok.kind(),
                                            tok.pos(),
                                            self.line_number
                                        ),
                                    }
                                } else {
                                    None
                                }
                            }
                            TokenKind::Literal(lit) => {
                                if let Some(kind) = self.peek_kind() {
                                    match kind {
                                        TokenKind::Newline => {
                                            self.saw_newline();
                                            self.eat(TokenKind::Newline);

                                            let expr = Expr::Literal(lit.clone());
                                            body.push(expr.clone());
                                            // we finished the line

                                            Some(Expr::FunctionDefinition(
                                                name,
                                                body,
                                                expr.gin_type(),
                                            ))
                                        }
                                        TokenKind::Comma => {
                                            self.eat(TokenKind::Comma);
                                            self.eat(TokenKind::Space);
                                            Some(Expr::Literal(lit))
                                        }
                                        TokenKind::CurlyClose => {
                                            self.eat(TokenKind::Space);
                                            Some(Expr::Literal(lit))
                                        }
                                        TokenKind::Space => {
                                            self.eat(TokenKind::Space);
                                            if let Some(TokenKind::CurlyClose) = self.peek_kind() {
                                                Some(Expr::Literal(lit))
                                            } else {
                                                None
                                            }
                                        }
                                        u => {
                                            println!("found u: {:#?}", u);
                                            None
                                        }
                                    }
                                } else {
                                    None
                                }
                            }
                            TokenKind::CurlyOpen => {
                                println!("got curly open");

                                let mut object_contents: HashMap<String, Expr> = HashMap::new();
                                self.eat(TokenKind::Space);

                                loop {
                                    let kind = self.next_kind();
                                    if let Some(TokenKind::Id(o_name)) = kind {
                                        if let Some(expr) = self.handle_id(o_name.clone()) {
                                            // println!("expr: {:#?}", &expr);
                                            object_contents.insert(o_name, expr);
                                        } else {
                                            println!("got nothing from handle_id")
                                        }
                                        self.eat(TokenKind::Space);
                                    } else if Some(TokenKind::CurlyClose) == kind {
                                        self.eat(TokenKind::Space);

                                        println!("got curly close");
                                        break;
                                    }
                                }

                                self.eat(TokenKind::Newline);
                                body.push(Expr::ObjectLiteral(object_contents));
                                let ret_type = self.find_implicit_return_type(&body);

                                Some(Expr::FunctionDefinition(name, body, ret_type))
                            }
                            _ => panic!(
                                "Unexpected ({:?}) at positon {} line {}",
                                tok.kind(),
                                tok.pos(),
                                self.line_number
                            ),
                        }
                    } else {
                        self.panic_position();
                        None
                    }
                }
                TokenKind::Space => {
                    if let Some(tok) = self.next() {
                        match tok.kind() {
                            TokenKind::Literal(lit) => {
                                let arg = match lit {
                                    Literal::String(s) => FnArg::String(s),
                                    Literal::Number(n) => FnArg::Number(n),
                                };

                                Some(Expr::FunctionCall(name, Some(arg)))
                            }
                            TokenKind::Id(id) => {
                                Some(Expr::FunctionCall(name, Some(FnArg::Id(id))))
                            }
                            TokenKind::CurlyOpen => {
                                // this is the start to a object definition
                                if let Some(TokenKind::Newline) = self.peek_kind() {
                                    self.next(); // eat newline
                                }

                                // this is a bit eager, but we are going to assume we only
                                // have one pair of curlies and there is no nesting yet.
                                //

                                let mut object_contents: HashMap<String, GinType> = HashMap::new();
                                // loop until end of object
                                loop {
                                    self.handle_indentation();
                                    let maybe_next_kind = self.next_kind();
                                    if let Some(next_kind) = maybe_next_kind {
                                        match next_kind {
                                            TokenKind::CurlyClose => break,
                                            TokenKind::Id(id) => {
                                                self.eat(TokenKind::Space);
                                                if let Some(TokenKind::Id(obj_t)) = self.next_kind()
                                                {
                                                    let obj_type =
                                                        GinType::from_str(&obj_t.as_str())
                                                            .expect("Failed to parse object type");
                                                    object_contents.insert(id, obj_type);
                                                }
                                            }
                                            TokenKind::Newline => continue,
                                            // NOTE: commas are not needed when defining
                                            // only in runtime as it could be mistaken for a fnCall arugment
                                            kind => panic!(
                                                "Unexpected ({:?}) line {}",
                                                kind, self.line_number
                                            ),
                                        }
                                    } else {
                                        // got nothing
                                        break;
                                    }
                                }

                                Some(Expr::ObjectDefinition(name, object_contents))
                            }
                            _ => {
                                panic!(
                                    "Unexpected ({:?}) at positon {} line {} ",
                                    tok.kind(),
                                    tok.pos(),
                                    self.line_number
                                )
                            }
                        }
                    } else {
                        self.panic_position();
                        None
                    }
                }
                TokenKind::Newline => {
                    self.saw_newline();
                    Some(Expr::FunctionCall(name, None))
                }
                unexpected => {
                    if let Some(token) = self.current() {
                        panic!(
                            "Unexpected ({:?}) at positon {} line {}",
                            token.kind(),
                            token.pos(),
                            self.line_number
                        )
                    } else {
                        panic!("Unexpected: {:#?}", unexpected)
                    }
                }
            }
        } else {
            println!("WHY ARE WE HERE? JUST TO SUFFER?");
            None
        }
    }

    fn expr(&mut self) -> Option<Expr> {
        // skip newline spam
        loop {
            if let Some(TokenKind::Newline) = self.peek_kind() {
                self.saw_newline();
                self.next(); // eats the newline
            } else {
                break;
            }
        }

        if let Some(tok) = self.next() {
            let kind = tok.kind();
            match kind {
                TokenKind::Id(name) => {
                    // println!("{} | self.expr -> handle_id({})", self.scope, name);
                    self.handle_id(name)
                }
                TokenKind::Space => {
                    panic!(
                        "Cannot start a expression with ({:?}) at positon {} line {}",
                        tok.kind(),
                        tok.pos(),
                        self.line_number
                    )
                }
                _ => self.expr(),
            }
        } else {
            // println!("-- End of tokens --");
            None
        }
    }

    pub fn parse(&mut self, mut tokens: Vec<Token>) -> Vec<Expr> {
        // will get replaced with a single ignore check on next token
        // when we stream tokens one at a time
        tokens.retain(|token| match token.kind() {
            TokenKind::Comment(_) => false,
            _ => true,
        });

        self.tokens = tokens;

        // We are assuming we are getting the result of tokens from a file.
        // so by default we have a root module

        let mut body: Vec<Expr> = Vec::new();

        loop {
            if let Some(expr) = self.expr() {
                body.push(expr)
            } else {
                break;
            }
        }

        body
    }
}

impl Iterator for Parser {
    type Item = Token;

    fn next(&mut self) -> Option<Self::Item> {
        // println!("index: {}", self.index);
        if self.index < self.tokens.len() {
            let item = Some(self.tokens[self.index].clone());
            self.index += 1;
            item
        } else {
            None
        }
    }
}
