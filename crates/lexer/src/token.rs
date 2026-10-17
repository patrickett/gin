/// Maximum indentation depth supported by the lexer.
pub const MAX_INDENT_DEPTH: usize = 16;

/// Indent and Dedent are mutually exclusive branches per newline.
/// We can exploit this by using `pending_indent` and `pending_dedents`
#[derive(Debug)]
pub struct LexContext {
    pub indent_stack: [u16; MAX_INDENT_DEPTH],
    pub indent_depth: u8,
    /// Dedent tokens still to be emitted, decremented one per call.
    pub pending_dedents: u8,
    /// Whether a single Indent token is waiting to be emitted.
    pub pending_indent: bool,
    /// Flag set when indentation exceeds MAX_INDENT_DEPTH.
    pub indent_overflow: bool,
}

impl Default for LexContext {
    fn default() -> Self {
        Self {
            indent_stack: [0u16; MAX_INDENT_DEPTH],
            indent_depth: 1,
            pending_dedents: 0,
            pending_indent: false,
            indent_overflow: false,
        }
    }
}

macro_rules! define_tokens {
    (
        keywords { $( $keyword:ident => $spelling:literal, )* }
        other { $( $variant:ident $( ( $field:ident : $payload:ty ) )? => $format:literal, )* }
    ) => {
        #[derive(Debug, PartialEq, Clone, Copy)]
        pub enum Token<'src> {
            $( $keyword, )*
            $( $variant $( ( $payload ) )?, )*
        }

        impl std::fmt::Display for Token<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self {
                    $( Self::$keyword => write!(f, "[{}]", $spelling), )*
                    $( Self::$variant $( ( $field ) )? => write!(f, $format), )*
                }
            }
        }

        impl<'src> Token<'src> {
            pub const KEYWORDS: &'static [Token<'static>] = &[$(Token::$keyword,)*];

            pub fn from_keyword(word: &str) -> Option<Self> {
                match word {
                    $( $spelling => Some(Self::$keyword), )*
                    _ => None,
                }
            }

            pub fn keyword(self) -> Option<&'static str> {
                match self {
                    $( Self::$keyword => Some($spelling), )*
                    _ => None,
                }
            }
        }
    };
}

define_tokens! {
    keywords {
        Extern => "extern",
        Continue => "continue",
        Private => "private",
        Return => "return",
        Break => "break",
        Loop => "loop",
        Ref => "ref",
        Mut => "mut",
        Own => "own",
        Deref => "deref",
        Domain => "domain",
        Then => "then",
        Do => "do",
        To => "to",
        Thus => "thus",
        When => "when",
        Else => "else",
        SelfInstance => "self",
        For => "for",
        While => "while",
        Use => "use",
        Has => "has",
        And => "and",
        As => "as",
        If => "if",
        In => "in",
        Is => "is",
        Of => "of",
        Not => "not",
        Or => "or",
        SelfTag => "Self",
    }
    other {
        Ampersand => "[&]",
        ArrowLeft => "[<-]",
        At => "[@]",
        BracketClose => "[]]",
        BracketOpen => "[[]",
        Caret => "[^]",
        Colon => "[:]",
        ColonColon => "[::]",
        ColonEq => "[:=]",
        ColonSemi => "[;]",
        Comma => "[,]",
        Comment(s: &'src str) => "[comment: \"{s}\"]",
        CurlyClose => "[}}]",
        CurlyOpen => "[{{]",
        Dedent => "[dedent]",
        DocComment(s: &'src str) => "[doc-comment: \"{s}\"]",
        ModuleDocComment(s: &'src str) => "[module-doc-comment: \"{s}\"]",
        Dot => "[.]",
        Eq => "[=]",
        EqEq => "[==]",
        Float(n: f64) => "[float: {n}]",
        FormatInterpEnd => "[interp-end]",
        FormatInterpStart => "[interp-start]",
        FormatStringDelim => "[format-delim]",
        FormatStringText(s: &'src str) => "[format-text: \"{s}\"]",
        Greater => "[>]",
        GreaterEq => "[>=]",
        Id(s: &'src str) => "[id: \"{s}\"]",
        Indent => "[indent]",
        Infer => "[...]",
        Int(n: i256::I256) => "[int: {n}]",
        Less => "[<]",
        LessEq => "[<=]",
        Minus => "[-]",
        Newline => "[\\n]",
        NotEq => "[/=]",
        ParenClose => "[)]",
        ParenOpen => "[(]",
        Percent => "[%]",
        Pipe => "[|]",
        Plus => "[+]",
        Pound => "[#]",
        Question => "[?]",
        ShiftLeft => "[<<]",
        ShiftRight => "[>>]",
        Slash => "[/]",
        SlashOr => "[\\]",
        Star => "[*]",
        String(s: &'src str) => "[string: \"{s}\"]",
        Tag(s: &'src str) => "[tag: \"{s}\"]",
        UnterminatedFormatString => "[unterminated-format]",
        UnterminatedString(s: &'src str) => "[unterminated-string: \"{s}\"]",
    }
}

#[cfg(test)]
mod tests {
    use super::Token;
    use crate::Lexer;

    #[test]
    fn every_keyword_lexes_and_displays_its_declared_spelling() {
        let mut spellings = std::collections::HashSet::new();
        for token in Token::KEYWORDS {
            let word = token.keyword().unwrap();
            assert!(spellings.insert(word), "duplicate keyword: {word}");
            let mut lexer = Lexer::new(word);
            assert_eq!(lexer.next_raw().unwrap().0, *token, "{word}");
            assert_eq!(token.to_string(), format!("[{word}]"));
            assert_eq!(Token::from_keyword(word), Some(*token));
        }
    }

    #[test]
    fn contextual_words_and_keyword_prefixes_are_not_reserved() {
        for word in [
            "asm",
            "target",
            "register",
            "return_value",
            "owned",
            "reference",
        ] {
            assert_eq!(Token::from_keyword(word), None);
            assert_eq!(Lexer::new(word).next_raw().unwrap().0, Token::Id(word));
        }
        for token in Token::KEYWORDS {
            let word = format!("{}value", token.keyword().unwrap());
            assert_eq!(Token::from_keyword(&word), None);
            assert!(Lexer::new(&word).next_raw().unwrap().0.keyword().is_none());
        }
    }
}
