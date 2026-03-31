use crate::ast::ModPath;
use crate::parse::delimited_list;
use crate::prelude::*;
use chumsky::span::SimpleSpan;

/// A capitalized variant constructor call, e.g. `Some(5)` or `Maybe.Some(3)`.
///
/// Distinct from [`FnCall`] (which uses lowercase identifiers) — this constructs
/// a tagged union value with the given variant name and arguments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TagCall {
    /// Simple variant name (e.g., "Some") - used for variant lookup
    pub name: Intern::<::std::string::String>,
    /// Optional qualified path (e.g., ModPath { root: "Maybe", segments: ["Some"] })
    pub qual_path: Option<ModPath>,
    pub args: Vec<Spanned<Expr>>,
    pub span: SimpleSpan,
}

pub fn tag_call<'t, I>(
    expr: impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, TagCall, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let args = delimited_list(Token::ParenOpen, expr, Token::Comma, Token::ParenClose);

    // Qualified form: Maybe.Some(x), Result.Ok(x) — uses Tag.Tag pattern
    let qualified =
        crate::ast::tag_variant_path()
            .then(args.clone())
            .map_with(|(path, args), e| {
                let variant_name = *path.segments.last().unwrap_or(&path.root);
                TagCall {
                    name: variant_name,
                    qual_path: Some(path),
                    args,
                    span: e.span(),
                }
            });

    // Simple form: Some(x), None()
    let simple = select! { Token::Tag(name) => Intern::<::std::string::String>::new(name.to_string()) }
        .then(args)
        .map_with(|(name, args), e| TagCall {
            name,
            qual_path: None,
            args,
            span: e.span(),
        });

    // Prefer qualified to avoid ambiguity
    choice((qualified, simple))
}
