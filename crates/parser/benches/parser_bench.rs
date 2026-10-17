use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use lexer::Lexer;
use parser::cursor::TokenCursor;
use test_fixtures::bench_sources::{COPY_GIN, INT_GIN, MAYBE_GIN, SIZED_GIN};

/// IO pipeline fixture (~62 lines, ~1.6KB).
const IO_GIN: &str = "\
Int is in 0 to 4294967295

write_spec := 'sink'

write(fd Int, buf @Int, len Int) Int:
    result := process_output(write_spec, fd, buf, len)
    return result

print(s String):
    _result := write(1, s.pointer, s.len)
    return

println(s String):
    newline := '\\n'
    out := print(s)
    print(newline)
    return out
";

/// Combined bundle of import-free files for a larger parse workload (~193 lines, ~8KB).
/// Only files without `use` statements are concatenated so all top-level
/// elements parse correctly regardless of position.
fn bundle_source() -> String {
    let mut s = String::with_capacity(16 * 1024);
    s.push_str(MAYBE_GIN);
    s.push('\n');
    s.push_str(INT_GIN);
    s.push('\n');
    s.push_str(COPY_GIN);
    s.push('\n');
    s.push_str(SIZED_GIN);
    s
}

fn large_int_source() -> String {
    let mut s = String::with_capacity(64 * 1024);
    s.push_str("Int is in 0 to 18446744073709551615\n");
    for i in 0..400 {
        s.push_str("IntType");
        s.push_str(&i.to_string());
        s.push_str(" is in ");
        s.push_str(&i.to_string());
        s.push_str(" to 0x");
        s.push_str(&format!("{:x}", 1024 + i * 17));
        s.push('\n');
    }
    s
}

/// Validate that every benchmark source produces a non-empty AST.
/// Panics with a descriptive message on failure.
pub(crate) fn validate_sources() {
    let bundle = bundle_source();

    let sources: &[(&str, &str)] = &[
        ("maybe", MAYBE_GIN),
        ("int", INT_GIN),
        ("copy", COPY_GIN),
        ("sized", SIZED_GIN),
        ("io", IO_GIN),
        ("large_int", &large_int_source()),
        ("bundle", &bundle),
    ];

    for (label, source) in sources {
        let ast = TokenCursor::parse_source(source);
        let empty = ast.defs.is_empty()
            && ast.tags.is_empty()
            && ast.exprs.is_empty()
            && ast.uses.is_empty();
        assert!(
            !empty,
            "'{}' source produced empty AST — parse errors",
            label
        );
        eprintln!(
            "[validate] {:<10} bytes={:<6} defs={:<3} tags={:<3} exprs={:<3} uses={}",
            format!("{}:", label),
            source.len(),
            ast.defs.len(),
            ast.tags.len(),
            ast.exprs.len(),
            ast.uses.len(),
        );
    }
}

/// Benchmark full pipeline (lex + parse).
fn bench_lex_and_parse(c: &mut Criterion) {
    validate_sources();

    let mut group = c.benchmark_group("lex_and_parse");
    let bundle = bundle_source();

    let inputs: &[(&str, &str)] = &[
        ("maybe", MAYBE_GIN),
        ("int", INT_GIN),
        ("copy", COPY_GIN),
        ("sized", SIZED_GIN),
        ("io", IO_GIN),
        ("large_int", &large_int_source()),
        ("bundle", &bundle),
    ];

    for (label, source) in inputs {
        group.throughput(criterion::Throughput::Bytes(source.len() as u64));
        group.bench_with_input(BenchmarkId::new("handwritten", label), source, |b, src| {
            b.iter(|| black_box(TokenCursor::parse_source(src)));
        });
    }

    group.finish();
}

/// Benchmark parser speed only (pre-tokenized input).
fn bench_parse_only(c: &mut Criterion) {
    validate_sources();

    let mut group = c.benchmark_group("parse_only");
    let bundle = bundle_source();

    let inputs: &[(&str, &str)] = &[
        ("maybe", MAYBE_GIN),
        ("int", INT_GIN),
        ("copy", COPY_GIN),
        ("sized", SIZED_GIN),
        ("io", IO_GIN),
        ("large_int", &large_int_source()),
        ("bundle", &bundle),
    ];

    for (label, source) in inputs {
        let mut lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.by_ref().collect();
        let span_table = lexer.span_table().clone();

        group.throughput(criterion::Throughput::Bytes(source.len() as u64));
        group.bench_function(BenchmarkId::new("handwritten", label), |b| {
            b.iter(|| {
                let mut st = span_table.clone();
                let ast = TokenCursor::parse_tokens_with_errors(&tokens, &mut st).0;
                black_box(ast);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_lex_and_parse, bench_parse_only);
criterion_main!(benches);

#[cfg(test)]
#[path = "parser_bench_tests.rs"]
mod tests;
