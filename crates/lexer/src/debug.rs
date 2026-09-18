use std::fmt;

use span::Span;

use crate::{Lexer, Token};

/// A token paired with its source span, with a human-readable [`Display`](fmt::Display) format.
///
/// Renders as `[id: "print"] (0..4)` — the token display followed by the byte span.
pub struct TokenSpanned<'src>(pub Token<'src>, pub Span);

impl<'src> fmt::Display for TokenSpanned<'src> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}..{})", self.0, self.1.start(), self.1.end())
    }
}

/// Extension trait providing debug token formatting on `str`.
pub trait DebugTokensExt {
    /// Lex `self` and return a formatted listing of every token and any errors.
    ///
    /// Useful for debugging lexer output in tests or from a REPL:
    ///
    /// ```ignore
    /// println!("{}", "print(x + 1)".debug_tokens());
    /// // [id: "print"] (0..5)
    /// // [(] (5..6)
    /// // [id: "x"] (6..7)
    /// // [+] (8..9)
    /// // [int: 1] (10..11)
    /// // [)] (11..12)
    /// ```
    fn debug_tokens(&self) -> String;
}

impl DebugTokensExt for str {
    fn debug_tokens(&self) -> String {
        let mut lexer = Lexer::new(self);
        let tokens: Vec<_> = lexer.by_ref().collect();
        let mut out = String::new();
        for (tok, span_id) in &tokens {
            let span = lexer.get_span(*span_id);
            out.push_str(&format!("{}\n", TokenSpanned(*tok, span)));
        }
        if !lexer.errors.is_empty() {
            out.push_str("\nerrors:\n");
            for err in &lexer.errors {
                out.push_str(&format!(
                    "  {} ({}..{})\n",
                    err.message,
                    err.span.start(),
                    err.span.end()
                ));
            }
        }
        out
    }
}
