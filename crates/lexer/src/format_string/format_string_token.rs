use crate::LexContext;
use logos::Logos;

#[derive(Logos, Debug, PartialEq, Clone)]
#[logos(extras = LexContext)]
#[logos(error = ())]
pub enum FormatStringToken<'src> {
    /// Closing double-quote
    #[token("\"")]
    End,

    /// Start of interpolation
    #[token("(")]
    InterpOpen,

    /// Literal text segment (everything except `"`, `(`, newline, and backslash)
    #[regex(r#"[^"(\n\\]+"#)]
    Text(&'src str),

    /// Escape sequence (e.g. `\n`, `\(`, `\\`) — kept as raw source for parser unescape
    #[regex(r"\\.", |lex| lex.slice())]
    Escape(&'src str),

    /// Newline inside format string = unterminated
    #[token("\n")]
    Newline,
}
