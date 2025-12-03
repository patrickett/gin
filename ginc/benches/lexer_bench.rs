//! Benchmarks for the lexer
//
// These benchmarks use `criterion` to measure the performance of the
// `GinLexer`.  They are intentionally simple – we just feed a large
// synthetic source string that contains a mix of identifiers, keywords
// and literals.  The goal is to get a sense of how fast the lexer can
// tokenize a typical program.

use criterion::{Criterion, criterion_group, criterion_main};
use ginc::frontend::lexer::GinLexer;

/// Generate a synthetic source string.  It repeats a set of tokens many
/// times so that the lexer has to process a lot of input.  The string is
/// constructed once per benchmark run.
fn synthetic_source(repeats: usize) -> String {
    let base = r#"use http.web as h, crypto.hash as c

    literal: 4.0

    Result is Ok(t) | Err(e)


    ServerState is StateIdle       |
                    StateConnected |
                    StateError     |
                    StateRetrying




    f(x): x

    complex_function (params):
        s: f (params)


    return s
"#;
    base.repeat(repeats)
}

/// Benchmark the lexer by iterating over all tokens until EOF.
fn bench_lexer(c: &mut Criterion) {
    let src = synthetic_source(10_000); // ~ 160k chars
    c.bench_function("gin lexer", |b| {
        b.iter(|| {
            let lex = GinLexer::new(&src);
            for (_tok, _span) in lex {
                // Consume all tokens
            }
        });
    });
}

criterion_group!(benches, bench_lexer);
criterion_main!(benches);
