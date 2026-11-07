//! Raw classifications without values or borrowed payloads.
use span::Span;

/// Classification and byte range only. Recover spelling from the original source
/// using `token.span.extract(source)`, including any literal delimiters.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

macro_rules! kinds {
    ($($keyword:ident => $word:literal,)* ; $($other:ident,)*) => {
        /// Raw classification. `Indent` and `Dedent` are parser-only virtual kinds.
        #[derive(Debug, PartialEq, Eq, Clone, Copy)]
        pub enum TokenKind { $($keyword,)* $($other,)* }

        impl TokenKind {
            pub const KEYWORDS: &'static [Self] = &[$(Self::$keyword,)*];
            pub fn from_keyword(word: &str) -> Option<Self> {
                match word { $($word => Some(Self::$keyword),)* _ => None }
            }
            pub fn keyword(self) -> Option<&'static str> {
                match self { $(Self::$keyword => Some($word),)* _ => None }
            }
        }
    };
}

kinds! {
    Extern => "extern", Continue => "continue", Private => "private",
    Return => "return", Break => "break", Loop => "loop", Ref => "ref",
    Mut => "mut", Own => "own", Deref => "deref", Domain => "domain",
    Then => "then", Do => "do", To => "to", Thus => "thus", When => "when",
    Where => "where",
    Else => "else", SelfInstance => "self", For => "for", While => "while",
    Use => "use", Has => "has", And => "and", As => "as", If => "if",
    In => "in", Is => "is", Of => "of", Not => "not", Or => "or",
    SelfTag => "Self",
    ;
    Ampersand, ArrowLeft, At, BracketClose, BracketOpen, Caret, Colon,
    ColonColon, ColonEq, ColonSemi, Comma, Comment, CurlyClose, CurlyOpen,
    Dedent, DocComment, ModuleDocComment, Dot, Eq, EqEq, Float,
    FormatStringDelim, Greater, GreaterEq, Id, Indent, Infer, Int, Less,
    LessEq, MalformedInt, MalformedFloat, Minus, Newline, NotEq,
    ParenClose, ParenOpen, Percent, Pipe, Plus, Pound, Question,
    ShiftLeft, ShiftRight, Slash, SlashOr, Star, String, Tag,
    UnterminatedString, Unknown,
}
