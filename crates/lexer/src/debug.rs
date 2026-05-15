use std::fmt;

use span::Span;

use crate::{Lexer, Token};

impl<'src> fmt::Display for Token<'src> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Id(s) => write!(f, "[id: \"{s}\"]"),
            Token::Tag(s) => write!(f, "[tag: \"{s}\"]"),
            Token::Int(n) => write!(f, "[int: {n}]"),
            Token::Float(n) => write!(f, "[float: {n}]"),
            Token::String(s) => write!(f, "[string: \"{s}\"]"),
            Token::UnterminatedString(s) => write!(f, "[unterminated-string: \"{s}\"]"),
            Token::FormatStringText(s) => write!(f, "[format-text: \"{s}\"]"),
            Token::Comment(s) => write!(f, "[comment: \"{s}\"]"),
            Token::DocComment(s) => write!(f, "[doc-comment: \"{s}\"]"),
            Token::ModuleDocComment(s) => write!(f, "[module-doc-comment: \"{s}\"]"),

            Token::FormatStringDelim => write!(f, "[format-delim]"),
            Token::FormatInterpStart => write!(f, "[interp-start]"),
            Token::FormatInterpEnd => write!(f, "[interp-end]"),
            Token::UnterminatedFormatString => write!(f, "[unterminated-format]"),

            Token::Extern => write!(f, "[extern]"),
            Token::Continue => write!(f, "[continue]"),
            Token::Private => write!(f, "[private]"),
            Token::Return => write!(f, "[return]"),
            Token::Break => write!(f, "[break]"),
            Token::Loop => write!(f, "[loop]"),
            Token::Mut => write!(f, "[mut]"),
            Token::Own => write!(f, "[own]"),
            Token::Then => write!(f, "[then]"),
            Token::When => write!(f, "[when]"),
            Token::Else => write!(f, "[else]"),
            Token::SelfInstance => write!(f, "[self]"),
            Token::SelfTag => write!(f, "[Self]"),
            Token::For => write!(f, "[for]"),
            Token::While => write!(f, "[while]"),
            Token::Use => write!(f, "[use]"),
            Token::Has => write!(f, "[has]"),
            Token::And => write!(f, "[and]"),
            Token::Infer => write!(f, "[...]"),
            Token::Asm => write!(f, "[asm]"),
            Token::As => write!(f, "[as]"),
            Token::If => write!(f, "[if]"),
            Token::In => write!(f, "[in]"),
            Token::Is => write!(f, "[is]"),
            Token::Of => write!(f, "[of]"),
            Token::Or => write!(f, "[or]"),

            Token::EqEq => write!(f, "[==]"),
            Token::NotEq => write!(f, "[/=]"),
            Token::LessEq => write!(f, "[<=]"),
            Token::ArrowLeft => write!(f, "[<-]"),
            Token::ArrowRight => write!(f, "[->]"),
            Token::GreaterEq => write!(f, "[>=]"),
            Token::Eq => write!(f, "[=]"),
            Token::Less => write!(f, "[<]"),
            Token::Greater => write!(f, "[>]"),
            Token::Plus => write!(f, "[+]"),
            Token::Minus => write!(f, "[-]"),
            Token::Star => write!(f, "[*]"),
            Token::Percent => write!(f, "[%]"),
            Token::SlashOr => write!(f, "[\\]"),
            Token::Slash => write!(f, "[/]"),
            Token::Caret => write!(f, "[^]"),
            Token::Pipe => write!(f, "[|]"),
            Token::ShiftLeft => write!(f, "[<<]"),
            Token::ShiftRight => write!(f, "[>>]"),
            Token::Tilde => write!(f, "[~]"),
            Token::Dot => write!(f, "[.]"),
            Token::At => write!(f, "[@]"),
            Token::Pound => write!(f, "[#]"),
            Token::ColonEq => write!(f, "[:=]"),
            Token::Colon => write!(f, "[:]"),
            Token::ColonSemi => write!(f, "[;]"),
            Token::ParenOpen => write!(f, "[(]"),
            Token::ParenClose => write!(f, "[)]"),
            Token::BracketOpen => write!(f, "[[]"),
            Token::BracketClose => write!(f, "[]]"),
            Token::CurlyOpen => write!(f, "[{{]"),
            Token::CurlyClose => write!(f, "[}}]"),
            Token::Ampersand => write!(f, "[&]"),
            Token::Comma => write!(f, "[,]"),

            Token::Newline => write!(f, "[\\n]"),
            Token::Indent => write!(f, "[indent]"),
            Token::Dedent => write!(f, "[dedent]"),
            Token::Whitespace => write!(f, "[whitespace]"),
        }
    }
}

/// A token paired with its source span, with a human-readable [`Display`](fmt::Display) format.
///
/// Renders as `[id: "print"] (0..4)` — the token display followed by the byte span.
pub struct TokenSpanned<'src>(pub Token<'src>, pub Span);

impl<'src> fmt::Display for TokenSpanned<'src> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}..{})", self.0, self.1.start, self.1.end)
    }
}

/// Lex `source` and return a formatted listing of every token and any errors.
///
/// Useful for debugging lexer output in tests or from a REPL:
///
/// ```ignore
/// println!("{}", debug_tokens("print(x + 1)"));
/// // [id: "print"] (0..5)
/// // [(] (5..6)
/// // [id: "x"] (6..7)
/// // [+] (8..9)
/// // [int: 1] (10..11)
/// // [)] (11..12)
/// ```
pub fn debug_tokens(source: &str) -> String {
    let mut lexer = Lexer::new(source);
    let tokens: Vec<_> = lexer.by_ref().collect();
    let mut out = String::new();
    for (tok, span_id) in &tokens {
        let span = lexer.get_span(*span_id);
        out.push_str(&format!("{}\n", TokenSpanned(*tok, span)));
    }
    if !lexer.errors.is_empty() {
        out.push_str("\nerrors:\n");
        for (err, span_id) in &lexer.errors {
            let span = lexer.get_span(*span_id);
            out.push_str(&format!("  {err:?} ({}..{})\n", span.start, span.end));
        }
    }
    out
}
