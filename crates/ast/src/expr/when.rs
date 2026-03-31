use crate::prelude::*;
use chumsky::span::SimpleSpan;
use lexer::Token;

/// Exhaustive conditional expression.
///
/// Boolean condition form:
/// ```gin
/// when n % 15 = 0 then print("FizzBuzz")
///      n % 05 = 0 then print("Fizz")
///      n % 03 = 0 then print("Buzz")
///      else print(n)
/// ```
///
/// Pattern matching form:
/// ```gin
/// when value
///     is Some(x) then x
///     is None    then 0
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WhenExpr {
    /// Subject expression for pattern matching (e.g., `when self`)
    /// None for condition-based when
    pub subject: Option<Box<Spanned<Expr>>>,
    pub arms: Vec<WhenArm>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WhenArm {
    /// Boolean condition: `<condition> then <body>`
    Cond {
        condition: Box<Spanned<Expr>>,
        body: Box<Spanned<Expr>>,
    },
    /// Pattern match: `is <tag> then <body>`
    Is {
        pattern: Tag,
        body: Box<Spanned<Expr>>,
    },
    /// Fallthrough: `else <body>`
    Else(Box<Spanned<Expr>>),
}

/// Internal enum for disambiguating the two when forms during parsing.
#[allow(clippy::large_enum_variant)]
enum WhenTail {
    /// Boolean: the initial expr was a condition, here's the result + more arms
    Boolean {
        first_result: Spanned<Expr>,
        rest: Vec<WhenArm>,
    },
    /// Pattern: the initial expr was the subject, here are the is/else arms
    Pattern(Vec<WhenArm>),
}

pub fn when_expr<'t, I>(
    expr: impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, WhenExpr, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use Token::*;

    let tag_parser = tag(expr.clone());

    // is <Tag> then <expr> (inline, then on same line)
    let is_arm = just(Is)
        .ignore_then(tag_parser.clone())
        .then_ignore(just(Then))
        .then(expr.clone())
        .map(|(pattern, body)| WhenArm::Is {
            pattern,
            body: Box::new(body),
        });

    // <expr> then <expr>
    let cond_arm =
        expr.clone()
            .then_ignore(just(Then))
            .then(expr.clone())
            .map(|(condition, body)| WhenArm::Cond {
                condition: Box::new(condition),
                body: Box::new(body),
            });

    // else <expr>
    let else_arm = just(Else)
        .ignore_then(expr.clone())
        .map(|body| WhenArm::Else(Box::new(body)));

    // Pattern form starting with `Is <tag>`, handling both:
    //   - Inline: `is Some(x) then result [else ...]`
    //   - Indented then/else: `is Some(x)\n    then result\n    else ...`
    let pattern_form = just(Is)
        .ignore_then(tag_parser)
        .then(choice((
            // Inline: Then immediately follows the tag
            just(Then)
                .ignore_then(expr.clone())
                .then(
                    choice((is_arm.clone(), else_arm.clone()))
                        .repeated()
                        .collect::<Vec<_>>(),
                )
                .boxed(),
            // Indented: Then/else on next line(s) in an indented block
            just(Newline)
                .repeated()
                .at_least(1)
                .ignore_then(just(Indent))
                .ignore_then(
                    just(Then).ignore_then(expr.clone()).then(
                        choice((is_arm.clone(), else_arm.clone()))
                            .repeated()
                            .collect::<Vec<_>>(),
                    ),
                )
                .then_ignore(just(Dedent).or_not())
                .boxed(),
        )))
        .map(|(pattern, (first_result, rest))| {
            let mut arms = vec![WhenArm::Is {
                pattern,
                body: Box::new(first_result),
            }];
            arms.extend(rest);
            WhenTail::Pattern(arms)
        })
        .boxed();

    // After `when <expr>`, the next token disambiguates:
    //   Then   → boolean form (the expr was a condition)
    //   Is     → pattern form (the expr was the subject)
    //   Indent → block pattern form (the expr was the subject, arms indented)
    just(When)
        .ignore_then(expr.clone())
        .then(choice((
            // Boolean form: Then <result>, optionally followed by more arms
            just(Then)
                .ignore_then(expr.clone())
                .then(
                    choice((
                        // Inline else
                        else_arm.clone().map(|arm| vec![arm]),
                        // Indented block of additional arms
                        just(Indent)
                            .ignore_then(
                                choice((cond_arm.clone(), else_arm.clone()))
                                    .repeated()
                                    .collect::<Vec<_>>(),
                            )
                            .then_ignore(just(Dedent).or_not()),
                    ))
                    .or_not(),
                )
                .map(|(first_result, rest)| WhenTail::Boolean {
                    first_result,
                    rest: rest.unwrap_or_default(),
                })
                .boxed(),
            // Pattern form: Is <tag> ...
            pattern_form,
            // Block pattern form: Indent (is|else arms)+ Dedent
            just(Indent)
                .ignore_then(
                    choice((is_arm, else_arm))
                        .repeated()
                        .at_least(1)
                        .collect::<Vec<_>>(),
                )
                .then_ignore(just(Dedent).or_not())
                .map(WhenTail::Pattern)
                .boxed(),
        )))
        .map(|(initial_expr, tail)| match tail {
            WhenTail::Boolean { first_result, rest } => {
                let mut arms = vec![WhenArm::Cond {
                    condition: Box::new(initial_expr),
                    body: Box::new(first_result),
                }];
                arms.extend(rest);
                WhenExpr {
                    subject: None,
                    arms,
                }
            }
            WhenTail::Pattern(arms) => WhenExpr {
                subject: Some(Box::new(initial_expr)),
                arms,
            },
        })
}
