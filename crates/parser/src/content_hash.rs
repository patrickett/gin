//! Content-based hashing of source text.

use lexer::Lexer;
use sha2::{Digest, Sha256};

/// Compute a SHA-256 hex digest of raw source text.
///
/// This performs semantic hashing by lexing the source and hashing
/// only the non-comment tokens. This means that adding/removing comments does not
/// invalidate the cache, significantly improving incremental compilation performance.
pub fn compute_content_hash(source: &str) -> String {
    let mut hasher = Sha256::new();

    // Lex the source and hash only non-comment tokens
    let lexer = Lexer::new(source);
    for (token, _span) in lexer {
        let token_str = format!("{:?}", token);
        hasher.update(token_str.as_bytes());
    }

    format!("{:x}", hasher.finalize())
}
