use std::{
    collections::HashMap,
    fs,
    iter::{Map, Peekable},
    path::{Path, PathBuf},
    ptr::null,
    slice::Iter,
    str::FromStr,
};

use crate::{
    lex::Lexer,
    token::{Literal, Token, TokenKind},
};

const TAB_SIZE: usize = 4;

#[derive(Debug, Clone)]
pub struct Module {
    path: PathBuf,
    body: Vec<Expr>,
}

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
    token_index: usize,
    lexer: Lexer,
    line_number: usize,
    scope: usize,
}

impl Parser {
    pub fn new() -> Self {
        Self {
            tokens: Vec::new(),
            token_index: 0,
            lexer: Lexer::new(),
            scope: 0,
            line_number: 0,
        }
    }

    fn current(&self) -> Option<Token> {
        if self.token_index < self.tokens.len() {
            let item = Some(self.tokens[self.token_index - 1].clone());
            item
        } else {
            None
        }
    }

    fn next_token(&mut self) -> Option<Token> {
        self.token_index += 1;
        self.lexer.next()
    }

    fn saw_newline(&mut self) {
        self.line_number += 1;
        self.scope = 0;
    }

    /// handle tabs and indentation
    fn handle_indentation(&mut self) {
        // println!("self.handle_indentation");
        loop {
            // println!("handle indent loop");
            if let Some(token) = self.next_token() {
                match token.kind() {
                    TokenKind::Tab => self.scope += 1,
                    TokenKind::Space => {
                        let mut space_count = 1;

                        loop {
                            // println!("space loop");
                            match self.next_token() {
                                Some(tok) => match tok.kind() {
                                    TokenKind::Space => {
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
                }
            } else {
                break;
            }
        }
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
        match self.next_token() {
            Some(tk) => {
                if tk.kind() == token_kind {
                    // eat token
                } else {
                    self.lexer.return_to_queue(tk)
                }
            }
            _ => {}
        }
    }

    fn handle_id(&mut self, name: String) -> Option<Expr> {
        if let Some(token) = self.next_token() {
            match token.kind() {
                TokenKind::Colon => {
                    self.eat(TokenKind::Space);

                    // after a :
                    if let Some(tok) = self.next_token() {
                        let mut body = Vec::new();

                        match tok.kind() {
                            TokenKind::Newline => {
                                // multiline fn
                                self.saw_newline();
                                self.handle_indentation();

                                if let Some(tk) = self.next_token() {
                                    match tk.kind() {
                                        TokenKind::Id(n) => {
                                            if let Some(xpr) = self.handle_id(n) {
                                                body.push(xpr)
                                            }
                                            self.handle_indentation();
                                            loop {
                                                // println!("scope loop");
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
                                        TokenKind::Literal(lit) => match self.next_token() {
                                            Some(ltk) => match ltk.kind() {
                                                TokenKind::Newline => {
                                                    self.saw_newline();
                                                    body.push(Expr::Literal(lit.clone()));

                                                    let ret = self.find_implicit_return_type(&body);

                                                    // we finished the line
                                                    Some(Expr::FunctionDefinition(name, body, ret))
                                                }
                                                _ => None,
                                            },
                                            _ => None,
                                        },
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
                                if let Some(tok) = self.next_token() {
                                    match tok.kind() {
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

                                            if let Some(tok) = self.next_token() {
                                                match tok.kind() {
                                                    TokenKind::CurlyClose => {
                                                        Some(Expr::Literal(lit))
                                                    }
                                                    _ => {
                                                        self.lexer.return_to_queue(tok);
                                                        None
                                                    }
                                                }
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
                                // println!("got curly open");

                                let mut object_contents: HashMap<String, Expr> = HashMap::new();
                                self.eat(TokenKind::Space);

                                loop {
                                    println!("curly loop");
                                    if let Some(token) = self.next_token() {
                                        match token.kind() {
                                            TokenKind::Id(o_name) => {
                                                if let Some(expr) = self.handle_id(o_name.clone()) {
                                                    object_contents.insert(o_name, expr);
                                                } else {
                                                    println!("got nothing from handle_id")
                                                }
                                                self.eat(TokenKind::Space);
                                            }
                                            TokenKind::CurlyClose => {
                                                self.eat(TokenKind::Space);
                                                println!("got curly close");
                                                break;
                                            }
                                            _ => break,
                                        }
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
                        panic!(
                            "Unexpected ({:?}) at positon {} line {}",
                            token.kind(),
                            token.pos(),
                            self.line_number
                        );
                    }
                }
                TokenKind::Space => {
                    if let Some(tok) = self.next_token() {
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
                                if let Some(tk) = self.next_token() {
                                    match tk.kind() {
                                        TokenKind::Newline => {}
                                        _ => self.lexer.return_to_queue(tk),
                                    }
                                }

                                // this is a bit eager, but we are going to assume we only
                                // have one pair of curlies and there is no nesting yet.
                                //

                                let mut object_contents: HashMap<String, GinType> = HashMap::new();
                                // loop until end of object
                                loop {
                                    // println!("curly2 loop");
                                    self.handle_indentation();

                                    match self.next_token() {
                                        Some(ntk) => match ntk.kind() {
                                            TokenKind::CurlyClose => break,
                                            TokenKind::Id(id) => {
                                                self.eat(TokenKind::Space);

                                                let nntk = self.next_token();
                                                if let Some(tkn) = nntk {
                                                    match tkn.kind() {
                                                        TokenKind::Id(obj_t) => {
                                                            let obj_type =
                                                                GinType::from_str(&obj_t.as_str())
                                                                    .expect(
                                                                    "Failed to parse object type",
                                                                );
                                                            object_contents.insert(id, obj_type);
                                                        }
                                                        // TODO: panic?
                                                        _ => {}
                                                    }
                                                }
                                            }
                                            TokenKind::Newline => continue,
                                            // NOTE: commas are not needed when defining
                                            // only in runtime as it could be mistaken for a fnCall arugment
                                            kind => panic!(
                                                "Unexpected ({:?}) line {}",
                                                kind, self.line_number
                                            ),
                                        },
                                        _ => break,
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
                        panic!(
                            "Unexpected ({:?}) at positon {} line {}",
                            token.kind(),
                            token.pos(),
                            self.line_number
                        );
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
            // println!("expr newline loop");
            if let Some(t) = self.next_token() {
                match t.kind() {
                    TokenKind::Newline => self.saw_newline(),
                    _ => {
                        self.lexer.return_to_queue(t);
                        break;
                    }
                }
            } else {
                break;
            }
        }

        if let Some(tok) = self.next_token() {
            match tok.kind() {
                TokenKind::Id(name) => self.handle_id(name),
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

    pub fn parse_module(&mut self, path: &Path) -> Module {
        let mut module = Module {
            path: path.to_path_buf(),
            body: Vec::new(),
        };

        if let Ok(file_contents) = fs::read_to_string(path) {
            let file_name = path
                .file_stem()
                .expect("Failed to read file name")
                .to_str()
                .expect("Failed to convert file name to str");

            self.lexer.set_source_content(file_contents);

            loop {
                // println!("parse mod loop");
                if let Some(expr) = self.expr() {
                    module.body.push(expr)
                } else {
                    break;
                }
            }
        }
        module
    }
}
