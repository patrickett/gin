use crate::frontend::prelude::*;

// PERF: replace Vec<T> where possible with sized arrays

pub mod construct;

pub type Spanned<T> = (T, SimpleSpan);
pub type ParserError<'tokens, 'source_code> = extra::Err<Rich<'tokens, Token<'source_code>>>;

// DEV NOTE:
//     't = 'tokens
//     's = 'source_code

/// Parses a stream of tokens
pub fn token_parser<'t, 's: 't, I>() -> impl Parser<'t, I, ParsedFile, ParserError<'t, 's>>
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    import()
        .repeated()
        .collect::<Vec<_>>()
        .or_not()
        .then(item().repeated().collect::<(TagMap, DefMap)>())
        .map(|(imports, (tags, defs))| ParsedFile {
            imports,
            tags,
            defs,
        })
}

impl FromIterator<Item> for (TagMap, DefMap) {
    fn from_iter<I: IntoIterator<Item = Item>>(iter: I) -> Self {
        let mut tags = TagMap::new();
        let mut defs = DefMap::new();

        for item in iter {
            match item.value {
                ItemValue::TagValue(name, bind) => {
                    tags.insert(name, bind);
                }
                ItemValue::DefValue(name, bind) => {
                    defs.insert(name, bind);
                }
            }
        }

        (tags, defs)
    }
}

// pub fn parse_program_and_report<'t, 's: 't, I>(
//     tokens: I,
//     src_txt: &'s str,
//     filename: String,
// ) -> Option<GinAST<'s>>
// where
//     I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
// {
//     let parse_result = token_parser().parse(tokens);
//     let (maybe_output, errors) = parse_result.into_output_errors();

//     if errors.is_empty() {
//         return Some(maybe_output.unwrap_or_default());
//     }

//     let mut cache = ariadne::sources([(filename.clone(), src_txt)]);

//     for err in errors.into_iter() {
//         let span = err.span();
//         let (start, end) = (span.start(), span.end());

//         let ariadne_span = (filename.clone(), Range { start, end });
//         let msg = format!("{err:?}");

//         let report = Report::build(
//             ReportKind::Custom("error", ariadne::Color::Red),
//             ariadne_span.clone(),
//         )
//         .with_message(msg.clone())
//         // TODO: better error msgs
//         .with_label(Label::new(ariadne_span).with_message("here"))
//         .finish();

//         report.eprint(&mut cache).unwrap();
//     }

//     None
// }
