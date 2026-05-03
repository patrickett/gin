use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use lexer::Lexer;

fn small_source() -> &'static str {
    r#"
Maybe[x] is Some(x) or None

main:
    val Maybe(3): Some(3)
    if val is Some(v)
        val
        four: v + 1
    return four
"#
}

fn medium_source() -> &'static str {
    r#"
--- The 8-bit signed integer type.
SignedTinyInt is -128...127

--- The 16-bit signed integer type.
SignedSmallInt is -32768...32767

--- The 32-bit signed integer type.
SignedInt is -2147483648...2147483647

--- The 64-bit signed integer type.
SignedBigInt is -9223372036854775808...9223372036854775807

--- The 8-bit unsigned integer type.
TinyInt is 0...255

--- The 16-bit unsigned integer type.
SmallInt is 0...65535

--- The 32-bit unsigned integer type.
Int is 0...4294967295

--- The 64-bit unsigned integer type.
BigInt is 0...18446744073709551615

is_even(n Int) Bool: n % 2 == 0
is_odd(n Int) Bool: n % 2 /= 0

clamp(x, lo, hi Int) Int:
    if x < lo return lo
    if x > hi return hi
return x


abs(x Int) Int: when x < 0
                then -x
                else x

max(a, b Int) Int: when a >= b
                   then a
                   else b

min(a, b Int) Int: when a <= b
                   then a
                   else b

divide(a, b Int) Int: when b == 0
                      then 0
                      else a / b

fibonacci(n Int) Int: when n <= 1
                      then n
                      else fibonacci(n - 1) + fibonacci(n - 2)

factorial(n Int) Int: when n <= 1
                      then 1
                      else n * factorial(n - 1)

sum_to(n Int) Int: when n <= 0
                   then 0
                   else n + sum_to(n - 1)
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
                s.push_str(&format!("{:08X}", i as u32 * 0xdead_beef_u32));
            }
            _ => unreachable!(),
        }
    }
    s
}

fn large_mixed_source() -> String {
    let mut s = String::with_capacity(16 * 1024);
    // module header
    s.push_str("--- Core utilities for the example package.\n");
    s.push_str("--- Provides common data structures and helpers.\n\n");

    // type definitions
    for i in 0..20 {
        s.push_str(&format!(
            "--- Variant {i} of the result type.\nResult_{i}[x] is Some(x) or None\n\n"
        ));
    }

    // function definitions with varied bodies
    for i in 0..40 {
        s.push_str(&format!(
            "--- Compute value {i}.\ncompute_{i}(a, b Int) Int:\n"
        ));
        s.push_str("    intermediate: a + b\n");
        if i % 3 == 0 {
            s.push_str("    if intermediate > 100\n");
            s.push_str("        return intermediate - 100\n");
            s.push_str("    return intermediate * 2\n");
        } else if i % 3 == 1 {
            s.push_str("    for x in 0...intermediate\n");
            s.push_str("        if x % 2 == 0\n");
            s.push_str("            continue\n");
            s.push_str("        intermediate: intermediate + x\n");
            s.push_str("    return intermediate\n");
        } else {
            s.push_str("    val: a * b + a / b\n");
            s.push_str("    result: 'computed'\n");
            s.push_str("    return val\n");
        }
        s.push('\n');
    }

    // string/format string section
    for i in 0..15 {
        s.push_str(&format!("msg_{i}: \"value is (x) and name is 'test'\"\n"));
    }
    s.push('\n');

    // hex and underscore numbers
    for i in 0..30 {
        s.push_str(&format!(
            "const_{i}: {}\n",
            match i % 4 {
                0 => format!("0x{:08X}", i * 0xcafe),
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
        s.push_str(&format!("func_{func}(a, b Int) Int:\n"));
        s.push_str("    val: a + b\n");
        s.push_str("    return val\n\n");
    }
    s
}

fn lex_all(source: &str) -> usize {
    let mut lexer = Lexer::new(source);
    let mut count = 0;
    for (tok, _span) in &mut lexer {
        std::hint::black_box(tok);
        count += 1;
    }
    std::hint::black_box(&lexer.errors);
    count
}

fn bench_lexer(c: &mut Criterion) {
    let mut group = c.benchmark_group("lexer");

    let inputs: &[(&str, &str)] = &[
        ("small", small_source()),
        ("medium", medium_source()),
        ("number_heavy", &number_heavy_source()),
        ("large_mixed", &large_mixed_source()),
        ("doc_heavy", &doc_heavy_source()),
    ];

    for (label, source) in inputs {
        let size = source.len();
        group.throughput(criterion::Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("handwritten", label), source, |b, src| {
            b.iter(|| lex_all(src));
        });
    }

    group.finish();
}

fn bench_numbers(c: &mut Criterion) {
    let mut group = c.benchmark_group("numbers");

    let int_line = "42 0 999 1_000_000 0xFF 0xDEAD_BEEF 18446744073709551615";
    let float_line = "3.14 0.0 99.999 1_000.5_5 0.001 3.14159265358979";
    let mixed_line = "42 3.14 0xFF 0.0 1_000 99.999 0xDEAD 1_000.5_5";

    group.throughput(criterion::Throughput::Elements(1));
    group.bench_function("handwritten_int_line", |b| b.iter(|| lex_all(int_line)));
    group.bench_function("handwritten_float_line", |b| b.iter(|| lex_all(float_line)));
    group.bench_function("handwritten_mixed_line", |b| b.iter(|| lex_all(mixed_line)));

    group.finish();
}

criterion_group!(benches, bench_lexer, bench_numbers);
criterion_main!(benches);
