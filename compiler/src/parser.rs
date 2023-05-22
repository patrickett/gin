use crate::lex::Token;

#[derive(Debug, Clone)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Exp,
    Eq,
    NotEq,
    GreaterThan,
    LessThan,
    GreaterThanOrEq,
    LessThanOrEq,
    LogicalAnd,
    LogicalOr,
    BitwiseAnd,
    BitwiseOr,
    BitwiseXor,
    BitwiseLeftShift,
    BitwiseRightShift,
}

#[derive(Debug, Clone)]
pub enum Literal {
    String(String),
    Number(usize),
}

#[derive(Debug, Clone)]
pub enum Expr {
    Assignment {
        lhs: String,
        rhs: Box<Expr>,
    },
    // Parenthesized(BoxExpr>),
    Literal(Literal),

    // List {},
    // Tuple,
    BinExpr {
        lhs: Box<Expr>,
        op: Op,
        rhs: Box<Expr>,
    },
}

pub fn parse(tokens: Vec<Token>) -> Vec<Expr> {
    let mut ast: Vec<Expr> = Vec::new();
    let mut current_expr: Vec<Token> = Vec::new();

    for token in tokens {
        match token {
            Token::Newline => {
                if !current_expr.is_empty() {
                    if let Some(expr) = build_expr(&current_expr) {
                        ast.push(expr);
                    }
                    current_expr.clear();
                }
            }
            _ => current_expr.push(token),
        }
    }

    if !current_expr.is_empty() {
        if let Some(expr) = build_expr(&current_expr) {
            ast.push(expr);
        }
    }

    ast
}

fn build_expr(tokens: &[Token]) -> Option<Expr> {
    let mut index = 0;
    parse_expr(tokens, &mut index)
}

// Recursive descent parser
fn parse_expr(tokens: &[Token], index: &mut usize) -> Option<Expr> {
    let mut left = parse_primary(tokens, index)?;

    while let Some(token) = tokens.get(*index) {
        let op = match token {
            Token::Plus => Op::Add,
            Token::Dash => Op::Sub,
            Token::Star => Op::Mul,
            Token::SlashBack => Op::Div,
            _ => break,
        };

        *index += 1;
        let right = parse_primary(tokens, index)?;
        // left = Expr::BinOp(Box::new(left), operator, Box::new(right));
        left = Expr::BinExpr {
            lhs: Box::new(left),
            op,
            rhs: Box::new(right),
        };
    }

    Some(left)
}

fn parse_primary(tokens: &[Token], index: &mut usize) -> Option<Expr> {
    if let Some(token) = tokens.get(*index) {
        match token {
            Token::Pound => {
                *index += 1;
                let expr = parse_expr(tokens, index)?;
                if let Some(Token::Newline) = tokens.get(*index) {
                    *index += 1;
                    Some(expr)
                } else {
                    panic!("Missing end to comment")
                }
            }
            Token::String(s) => {
                *index += 1;
                Some(Expr::Literal(Literal::String(s.to_string())))
            }
            Token::Number(num) => {
                *index += 1;
                Some(Expr::Literal(Literal::Number(*num)))
            }
            Token::Id(name) => {
                *index += 1;
                if let Some(Token::Colon) = tokens.get(*index) {
                    *index += 1;
                    let expr = parse_expr(tokens, index)?;
                    Some(Expr::Assignment {
                        lhs: name.to_string(),
                        rhs: Box::new(expr),
                    })
                } else {
                    panic!("Missing colon after Token::Id");
                }
            }
            Token::ParenLeft => {
                *index += 1;
                let expr = parse_expr(tokens, index)?;

                if let Some(Token::ParenRight) = tokens.get(*index) {
                    *index += 1;
                    Some(expr)
                } else {
                    panic!("Missing closing parenthesis")
                }
            }
            Token::Colon => None,
            Token::EOF => None,
            tok => panic!("Unexpected token: {:#?}", tok),
        }
    } else {
        None
    }
}
