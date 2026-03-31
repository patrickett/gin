//! Sub-lexer for format string internals.
//! Used via `morph()` from the main Token lexer when a `"` is encountered.

use super::token::Token;
use diagnostic::lex::LexSymptom;
use chumsky::span::SimpleSpan;
use logos::Lexer;
use std::collections::VecDeque;

mod format_string_token;
pub use format_string_token::*;

pub struct FormatStringResult<'src> {
    pub tokens: VecDeque<(Token<'src>, SimpleSpan)>,
    pub errors: Vec<(LexSymptom, SimpleSpan)>,
    pub main_lexer: Lexer<'src, Token<'src>>,
}

pub struct FormatStringLexer;

impl FormatStringLexer {
    /// Lex the interior of a format string after the opening `"`.
    /// Takes ownership of the main lexer (via morph), returns all produced tokens,
    /// errors, and the restored main lexer.
    pub fn lex<'src>(
        main_lexer: Lexer<'src, Token<'src>>,
        open_span: SimpleSpan,
    ) -> FormatStringResult<'src> {
        let mut tokens = VecDeque::new();
        let mut errors = Vec::new();

        tokens.push_back((Token::FormatStringDelim, open_span));

        let mut fmt_lexer: Lexer<'src, FormatStringToken<'src>> = main_lexer.morph();

        loop {
            let start = fmt_lexer.span().end;
            let next = fmt_lexer.next();
            let end = fmt_lexer.span().end;
            let span: SimpleSpan = (start..end).into();

            match next {
                Some(Ok(FormatStringToken::Text(s))) | Some(Ok(FormatStringToken::Escape(s))) => {
                    tokens.push_back((Token::FormatStringText(s), span));
                }
                Some(Ok(FormatStringToken::End)) => {
                    tokens.push_back((Token::FormatStringDelim, span));
                    return FormatStringResult {
                        tokens,
                        errors,
                        main_lexer: fmt_lexer.morph(),
                    };
                }
                Some(Ok(FormatStringToken::InterpOpen)) => {
                    tokens.push_back((Token::FormatInterpStart, span));
                    let mut main_lexer: Lexer<'src, Token<'src>> = fmt_lexer.morph();
                    let mut paren_depth: u32 = 0;

                    loop {
                        let istart = main_lexer.span().end;
                        let inext = main_lexer.next();
                        let iend = main_lexer.span().end;
                        let ispan: SimpleSpan = (istart..iend).into();

                        match inext {
                            Some(Ok(Token::ParenOpen)) => {
                                paren_depth += 1;
                                tokens.push_back((Token::ParenOpen, ispan));
                            }
                            Some(Ok(Token::ParenClose)) => {
                                if paren_depth == 0 {
                                    tokens.push_back((Token::FormatInterpEnd, ispan));
                                    fmt_lexer = main_lexer.morph();
                                    break;
                                }
                                paren_depth -= 1;
                                tokens.push_back((Token::ParenClose, ispan));
                            }
                            Some(Ok(Token::FormatStringDelim)) => {
                                tokens.push_back((Token::UnterminatedFormatString, ispan));
                                errors.push((LexSymptom::UnclosedString, ispan));
                                return FormatStringResult {
                                    tokens,
                                    errors,
                                    main_lexer,
                                };
                            }
                            Some(Ok(Token::Newline | Token::Indent | Token::Dedent)) => {
                                // Ignore indent tracking inside interpolation
                            }
                            Some(Ok(tok)) => {
                                tokens.push_back((tok, ispan));
                            }
                            Some(Err(err)) => {
                                errors.push((err, ispan));
                            }
                            None => {
                                let eof_span: SimpleSpan =
                                    (main_lexer.span().end..main_lexer.span().end).into();
                                tokens.push_back((Token::UnterminatedFormatString, eof_span));
                                errors.push((LexSymptom::UnclosedString, open_span));
                                return FormatStringResult {
                                    tokens,
                                    errors,
                                    main_lexer,
                                };
                            }
                        }
                    }
                }
                Some(Ok(FormatStringToken::Newline)) | None => {
                    tokens.push_back((Token::UnterminatedFormatString, span));
                    errors.push((LexSymptom::UnclosedString, open_span));
                    return FormatStringResult {
                        tokens,
                        errors,
                        main_lexer: fmt_lexer.morph(),
                    };
                }
                Some(Err(())) => {
                    errors.push((LexSymptom::UnexpectedCharacter, span));
                }
            }
        }
    }
}
