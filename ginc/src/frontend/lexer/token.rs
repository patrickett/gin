use crate::frontend::lexer::LexContext;
use crate::frontend::lexer::handle_newline;
use logos::Logos;

// TODO: can we get away with removing logos and skipping lexing and incremental parsing
// or go from source -> ast maybe with some incomplete nodes
#[derive(Logos, Debug, PartialEq, Clone)]
#[logos(extras = LexContext)]
pub enum Token<'src> {
    #[token("optional")]
    Optional,
    #[token("required")]
    Required,
    #[token("continue")]
    Continue,
    #[token("derives")]
    Derives,
    #[token("private")]
    Private,
    #[token("public")]
    Public,
    #[token("define")]
    Define,
    #[token("return")]
    Return,
    #[token("break")]
    Break,
    #[token("Nothing")]
    Nothing,
    #[token("alias")]
    Alias,
    #[token("macro")]
    Macro,
    #[token("needs")]
    Needs,
    #[token("loop")]
    Loop,
    #[token("then")]
    Then,
    #[token("when")]
    When,
    #[token("else")]
    Else,
    #[token("does")]
    Does,
    #[token("from")]
    From,
    #[token("self")]
    SelfInstance,
    #[token("Self")]
    SelfTag,
    #[token("for")]
    For,
    #[token("use")]
    Use,
    #[token("has")]
    Has,
    #[token("and")]
    And,
    #[token("def")]
    Def,
    #[token("where")]
    Where,
    #[token("as")]
    As,
    #[token("do")]
    Do,
    #[token("if")]
    If,
    #[token("in")]
    In,
    #[token("is")]
    Is,
    #[token("of")]
    Of,
    #[token("or")]
    Or,
    #[regex(r"\p{Lu}[\p{L}]*")]
    Tag(&'src str),
    #[regex(r"_?\p{Ll}+(?:_\p{Ll}+)*")]
    Id(&'src str),
    #[regex(r"[0-9]+\.[0-9]+", |lex| lex.slice().parse::<f64>().unwrap())]
    Float(f64),
    #[regex(r"[0-9]+", |lex| lex.slice().parse::<i64>().unwrap())]
    Int(i64),
    #[regex(r"'[^'\n]*'", |lex| {
        let s = lex.slice();
        &s[1..s.len()-1] // strip the quotes
    })]
    String(&'src str),
    #[regex(r"'[^'\n]*", |lex| {
        let s = lex.slice();
        &s[1..] // strip opening quote
    })]
    UnterminatedString(&'src str),
    // Format strings - double quoted strings with (var) interpolation
    #[regex(r#""[^"\n]*""#, |lex| {
        let s = lex.slice();
        &s[1..s.len()-1] // strip the quotes
    })]
    FormatString(&'src str),
    #[regex(r#""[^"\n]*"#, |lex| {
        let s = lex.slice();
        &s[1..] // strip opening quote
    })]
    UnterminatedFormatString(&'src str),
    #[regex(r"---[^\n]*", callback = |lex| { lex.slice() })]
    DocComment(&'src str),
    #[regex(r"--[^\n]*", callback = |lex| { lex.slice() })]
    Comment(&'src str),
    #[token("...")]
    Ellipsis,
    #[token("::=")]
    IsReplacedBy,
    // #[token(":=")]
    // Assignment,
    #[token("|-")]
    /// https://en.wikipedia.org/wiki/Turnstile_(symbol)
    Turnstile,
    // Operators (longest first)
    #[token("==")]
    EqEq,
    #[token("!=")]
    NotEqual,
    #[token("<=")]
    LessEq,
    #[token("<-")]
    ArrowLeft,
    #[token("->")]
    ArrowRight,
    #[token(">=")]
    GreaterEq,
    #[token("=")]
    Equals,
    #[token("<")]
    Less,
    #[token(">")]
    Greater,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("\\")]
    SlashOr,
    #[token("/")]
    Slash,
    #[token("|")]
    Bar,
    #[token("^")]
    Caret,
    #[token("~")]
    Tilde,
    // Punctuation
    #[token("..")]
    DotDot,
    #[token(".")]
    Dot,
    #[token("@")]
    At,
    #[token("#")]
    Pound,
    #[token(":")]
    Colon,
    #[token(";")]
    ColonSemi,
    #[token("(")]
    ParenOpen,
    #[token(")")]
    ParenClose,
    #[token("[")]
    BracketOpen,
    #[token("]")]
    BracketClose,
    #[token("{")]
    CurlyOpen,
    #[token("}")]
    CurlyClose,
    #[token("&")]
    Ampersand,
    #[token(",")]
    Comma,

    // Newline triggers indentation logic
    #[regex(r"\n", handle_newline)]
    Newline,

    // Indentation tokens
    Indent,
    Dedent,

    // Inline whitespace (skip, but only non-leading)
    #[regex(r"[ \t]+", logos::skip)]
    Whitespace,
}
