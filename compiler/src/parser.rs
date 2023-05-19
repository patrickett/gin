use crate::lex::Token;

#[derive(Debug)]
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
    Assign,
}

#[derive(Debug)]
pub enum Expr {
    // Literals
    String(String),
    Number(usize),

    List {},
    Tuple,

    BinOp {
        lhs: Box<Expr>,
        op: Op,
        rhs: Box<Expr>,
    },
}

pub fn parse(tokens: Vec<Token>) {
    let lines: Vec<Vec<Token>> = tokens
        .split(|val| match val {
            Token::Newline => true,
            _ => false,
        })
        .map(|s| s.to_vec())
        .collect();

    // for line in lines {
    //     let mut toks = line.iter().peekable();
    //     while let Some(token) = toks.next() {
    //         match token {
    //             // Token::Id(val) => toks.peek(),
    //             _ => (),
    //         }
    //     }
    // }

    println!("{:#?}", lines);
}

// Parse function to convert Vec<Token> into AST
fn parse_1(tokens: &[Token]) -> Result<Expr, String> {
    let mut index = 0;

    // Helper function to get the current token
    fn current_token(tokens: &[Token], index: usize) -> Option<&Token> {
        tokens.get(index)
    }

    // Helper function to advance to the next token
    fn next_token(tokens: &[Token], index: &mut usize) -> Option<&Token> {
        *index += 1;
        current_token(tokens, *index)
    }

    // Recursive expression parsing function
    fn parse_expression(tokens: &[Token], index: &mut usize) -> Result<Expr, String> {
        let current = current_token(tokens, *index);

        match current {
            // Some(Token::Id())
            // Some(Token::Number(num)) => {
            //     *index += 1;
            //     Ok(Expr::Literal(*num))
            // }
            // Some(Token::Op(op)) => {
            //     *index += 1;
            //     let left = parse_expression(tokens, index)?;
            //     let right = parse_expression(tokens, index)?;
            //     Ok(Expr::BinOp(Box::new(left), op, Box::new(right)))
            // }
            _ => Err("Unexpected token".to_string()),
        }
    }

    // Start parsing from the first token
    let ast = parse_expression(tokens, &mut index)?;

    // Ensure all tokens have been consumed
    if index < tokens.len() {
        Err("Unexpected tokens after parsing".to_string())
    } else {
        Ok(ast)
    }
}
