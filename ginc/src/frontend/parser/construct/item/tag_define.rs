use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub enum TagValue {
    // Tag2 ::= Tag1
    // PossibleTags ::= Tag1 | Tag2
    Alias(Tag),
    // Person ::= (name String, age Number)
    Record(Vec<Parameter>),
    // Record(std::iter::Map<&'src str, Box<TagValue<'src>>>),
    // PersonSet ::= { p : Person }
    Set(/* TODO */),
}

#[derive(Debug, Clone)]
pub struct DefineTag {
    pub doc_comment: Option<DocComment>,
    pub tag: Tag,
    pub value: TagValue,
}

// TODO: can we use Map<ParameterName, ParameterValue> instead Vec<Parameter>
pub fn define_tag<'t, 's: 't, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t, 's>> + Clone + 't,
) -> impl Parser<'t, I, DefineTag, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    // LHS-only parser: nominal or generic, but no union
    let lhs = {
        let tag_name = select! { Token::Tag(name) => name };
        let params = parameter(expr.clone(), tag(expr.clone()))
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
            .or_not();

        tag_name.then(params).map(|(name, parameters)| {
            let name = name.to_string();

            match parameters {
                None => Tag::Nominal { name },
                Some(parameters) if parameters.is_empty() => Tag::Nominal { name },
                Some(parameters) => Tag::Generic { name, parameters },
            }
        })
    };

    // RHS: either a union of tags or a record
    let record = parameter(expr.clone(), tag(expr.clone()))
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::ParenOpen), just(Token::ParenClose));

    let rhs_value = choice((
        tag(expr.clone()).map(TagValue::Alias),
        record.map(TagValue::Record),
    ));

    doc_comment()
        .or_not()
        .then(lhs.then_ignore(just(Token::Is)).then(rhs_value))
        .map(|(doc_comment, (tag, value))| DefineTag {
            doc_comment,
            tag,
            value,
        })
}
