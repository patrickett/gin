use crate::expr::r#return::r#return;
use crate::prelude::*;
use chumsky::span::SimpleSpan;
use lexer::Token;

use crate::expr::r#return::Return;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IfCondition {
    Bool(Box<Spanned<Expr>>),
    Pattern {
        subject: Box<Spanned<Expr>>,
        tag: Tag,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IfExpr {
    pub condition: IfCondition,
    pub body: Vec<Spanned<Expr>>,
    pub ret: Return,
}

pub fn if_expr<'t, I>(
    expr: impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, IfExpr, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use Token::*;

    // Parse: If <expr> [is <tag>]
    let condition = expr
        .clone()
        .then(just(Is).ignore_then(tag(expr.clone())).or_not())
        .map(|(subject, maybe_tag)| match maybe_tag {
            None => IfCondition::Bool(Box::new(subject)),
            Some(t) => IfCondition::Pattern {
                subject: Box::new(subject),
                tag: t,
            },
        });

    // Custom parser for if blocks:
    // - If <condition>
    // - Newline (repeated, to handle blank lines)
    // - Two forms:
    //   1. Indented form: Indent + body + Dedent (required) + return
    //   2. Non-indented form: body + (optional Dedent) + return
    // This ensures the if parser only consumes a dedent if it opened one.
    let indented_form = just(Indent)
        .ignore_then(expr.clone().repeated().collect::<Vec<_>>())
        .then_ignore(just(Dedent)) // REQUIRED when we saw Indent
        .then(r#return(expr.clone()))
        .map(|(body, ret)| (body, ret));

    let non_indented_form = expr
        .clone()
        .repeated()
        .collect::<Vec<_>>()
        .then_ignore(just(Dedent).or_not()) // Skip dedent if present (belongs to parent)
        .then(r#return(expr.clone()))
        .map(|(body, ret)| (body, ret));

    just(If)
        .ignore_then(condition)
        .then_ignore(just(Newline).repeated()) // Consume all newlines after condition
        .then(choice((indented_form, non_indented_form)))
        .map(|(condition, (body, ret))| IfExpr {
            condition,
            body,
            ret,
        })
}
