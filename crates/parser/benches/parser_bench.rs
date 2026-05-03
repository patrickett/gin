use parser::expr;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use lexer::Lexer;

/// Convert a 0-based index to a bijective alphabetic suffix (a, b, ..., z, aa, ab, ...).
/// Needed because the lexer's `lex_keyword_or_id` only continues with [a-z_],
/// NOT digits — so `compute_0` splits into `Id("compute_")` + `Int(0)`.
fn alpha_idx(i: usize) -> String {
    let mut s = String::new();
    let mut n = i + 1; // 1-indexed for bijective base-26
    while n > 0 {
        n -= 1;
        s.push((b'a' + (n % 26) as u8) as char);
        n /= 26;
    }
    s.chars().rev().collect()
}

fn small_source() -> &'static str {
    r#"Maybe[x] is Some(x) or None

main(a, b):
    val: a + b
    doubled: val * 2
return doubled
"#
}

fn medium_source() -> &'static str {
    r#"Color is Red or Green or Blue
Shape is Circle or Square or Triangle
Status is Active or Inactive or Pending
Direction is North or South or East or West
Season is Spring or Summer or Autumn or Winter
Result is Success or Error

add(a, b): a + b
sub(a, b): a - b
mul(a, b): a * b
div(a, b): a / b
double(x): x * 2
square(x): x * x
halve(x): x / 2
offset(a, b): a + b
scale(x, y): x * y
one: 1
two: 2
three: 3
hundred: 100
pi: 3.14
greeting: 'hello'
"#
}

fn number_heavy_source() -> String {
    let mut s = String::with_capacity(8192);
    for i in 0..200 {
        if i > 0 {
            s.push('\n');
        }
        match i % 6 {
            0 => {
                s.push_str("x: ");
                s.push_str(&format!("{}", i * 1000 + 42));
            }
            1 => {
                s.push_str("y: ");
                let v = i as f64 * std::f64::consts::PI + std::f64::consts::E;
                s.push_str(&format!("{v:.5}"));
            }
            2 => {
                s.push_str("z: 0x");
                s.push_str(&format!("{:X}", i * 255));
            }
            3 => {
                s.push_str("n: ");
                s.push_str(&format!("1_{:03}_{:03}", i, i * 7));
            }
            4 => {
                s.push_str("f: ");
                s.push_str(&format!("{}.{:06}", i, i * 12345));
            }
            5 => {
                s.push_str("h: 0x");
                s.push_str(&format!("{:08X}", (i as u32).wrapping_mul(0xdead_beef_u32)));
            }
            _ => unreachable!(),
        }
    }
    s
}

fn large_mixed_source() -> String {
    let mut s = String::with_capacity(16 * 1024);

    // 20 declare unions — Tag names can include digits (lex_tag reads [a-zA-Z0-9]).
    for i in 0..20 {
        s.push_str(&format!("Result{i} is Success{i} or Error{i}\n"));
    }

    s.push('\n');

    // 40 simple function binds — single-param only to keep the benchmark
    // source structure uniform.
    for i in 0..40 {
        let suffix = alpha_idx(i);
        if i % 3 == 0 {
            s.push_str(&format!("compute{suffix}(x): x + x\n"));
        } else if i % 3 == 1 {
            s.push_str(&format!("transform{suffix}(x): x * 2\n"));
        } else {
            s.push_str(&format!("scale{suffix}(x): x * x\n"));
        }
    }

    s.push('\n');

    // 30 simple value binds (alpha suffixes)
    for i in 0..30 {
        let suffix = alpha_idx(i);
        s.push_str(&format!(
            "const{suffix}: {}\n",
            match i % 4 {
                0 => format!("0x{:08X}", (i as u32).wrapping_mul(0x0000CAFE)),
                1 => format!("1_000_{}", i * 100),
                2 => format!("{}.{:06}", i, i * 999),
                _ => format!("{}", i * 1_000_000),
            }
        ));
    }

    s
}

fn doc_heavy_source() -> String {
    let mut s = String::with_capacity(16 * 1024);
    for func in 0..10 {
        for line in 0..20 {
            s.push_str(&format!(
                "--- Function {func} documentation line {line}: \
                 this is a longer comment to exercise scan-to-newline across many bytes of text.\n"
            ));
        }
        // Single-param only to keep the benchmark source structure uniform.
        s.push_str(&format!("func{}(x): x * 2\n", alpha_idx(func)));
    }
    s
}

/// Validate that all benchmark sources parse without errors.
/// Panics with a descriptive message if any source has parse errors,
/// so broken sources are caught before benchmarking.
fn validate_sources() {
    let number_heavy = number_heavy_source();
    let large_mixed = large_mixed_source();
    let doc_heavy = doc_heavy_source();

    let sources: &[(&str, &str)] = &[
        ("small", small_source()),
        ("medium", medium_source()),
        ("number_heavy", &number_heavy),
        ("large_mixed", &large_mixed),
        ("doc_heavy", &doc_heavy),
    ];

    for (label, source) in sources {
        // ── Validate with handwritten parser ──
        let hw_ast = expr::parse_source(source);
        let hw_empty = hw_ast.defs.is_empty()
            && hw_ast.tags.is_empty()
            && hw_ast.exprs.is_empty()
            && hw_ast.uses.is_empty();
        if hw_empty {
            panic!(
                "handwritten parser produced empty AST for '{}' source — likely parse errors",
                label
            );
        }

        eprintln!(
            "[validate] {:<15} defs={:<3} tags={:<3} exprs={:<3} uses={}",
            format!("{}:", label),
            hw_ast.defs.len(),
            hw_ast.tags.len(),
            hw_ast.exprs.len(),
            hw_ast.uses.len(),
        );
    }
}

/// Benchmark full pipeline (lex + parse) with the handwritten parser.
fn bench_lex_and_parse(c: &mut Criterion) {
    validate_sources();

    let mut group = c.benchmark_group("lex_and_parse");

    let number_heavy = number_heavy_source();
    let large_mixed = large_mixed_source();
    let doc_heavy = doc_heavy_source();

    let inputs: &[(&str, &str)] = &[
        ("small", small_source()),
        ("medium", medium_source()),
        ("number_heavy", &number_heavy),
        ("large_mixed", &large_mixed),
        ("doc_heavy", &doc_heavy),
    ];

    for (label, source) in inputs {
        let size = source.len();
        group.throughput(criterion::Throughput::Bytes(size as u64));

        group.bench_with_input(BenchmarkId::new("handwritten", label), source, |b, src| {
            b.iter(|| expr::parse_source(src));
        });
    }

    group.finish();
}

/// Benchmark parser speed only (pre-tokenized input):
/// - handwritten parser (via handwritten::parse_tokens)
fn bench_parse_only(c: &mut Criterion) {
    validate_sources();

    let mut group = c.benchmark_group("parse_only");

    let number_heavy = number_heavy_source();
    let large_mixed = large_mixed_source();
    let doc_heavy = doc_heavy_source();

    let inputs: &[(&str, &str)] = &[
        ("small", small_source()),
        ("medium", medium_source()),
        ("number_heavy", &number_heavy),
        ("large_mixed", &large_mixed),
        ("doc_heavy", &doc_heavy),
    ];

    for (label, source) in inputs {
        let mut lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.by_ref().collect();
        let span_table = lexer.span_table().clone();

        group.throughput(criterion::Throughput::Bytes(source.len() as u64));
        group.bench_function(BenchmarkId::new("handwritten", label), |b| {
            b.iter(|| {
                let mut st = span_table.clone();
                let ast = expr::parse_tokens(&tokens, &mut st);
                std::hint::black_box(ast);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_lex_and_parse, bench_parse_only);
criterion_main!(benches);

// ── correctness tests that run with `cargo test` ──────────────────────────────

#[cfg(test)]
mod tests {

    /// Each benchmark source must parse without errors and
    /// produce a non-empty handwritten AST.
    #[test]
    fn test_sources_parse_cleanly() {
        validate_sources();
    }

    #[test]
    fn test_small_source() {
        let ast = expr::parse_source(small_source());
        assert!(!ast.tags.is_empty(), "small source should have declares");
        assert!(!ast.defs.is_empty(), "small source should have binds");
    }

    #[test]
    fn test_medium_source() {
        let ast = expr::parse_source(medium_source());
        assert!(!ast.tags.is_empty(), "medium source should have declares");
        assert!(!ast.defs.is_empty(), "medium source should have binds");
    }

    #[test]
    fn test_number_heavy_source() {
        let ast = expr::parse_source(&number_heavy_source());
        assert!(!ast.defs.is_empty(), "number_heavy should have binds");
    }

    #[test]
    fn test_large_mixed_source() {
        let ast = expr::parse_source(&large_mixed_source());
        assert!(
            !ast.tags.is_empty() && !ast.defs.is_empty(),
            "large_mixed should have both declares and binds"
        );
    }

    #[test]
    fn test_alpha_idx() {
        assert_eq!(alpha_idx(0), "a");
        assert_eq!(alpha_idx(25), "z");
        assert_eq!(alpha_idx(26), "aa");
        assert_eq!(alpha_idx(27), "ab");
        assert_eq!(alpha_idx(51), "az");
        assert_eq!(alpha_idx(52), "ba");
    }

    #[test]
    fn test_doc_heavy_source() {
        let ast = expr::parse_source(&doc_heavy_source());
        assert!(!ast.defs.is_empty(), "doc_heavy should have binds");
    }
}
