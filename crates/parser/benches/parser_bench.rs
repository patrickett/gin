use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use lexer::Lexer;
use parser::cursor::TokenCursor;

// Synthetic Gin sources kept inline so benchmarks don't depend on external files.
// Each exercises the parser with different constructs.

/// Simple Tag `is` declare (~8 lines, ~100 bytes).
const MAYBE_GIN: &str = "\
Maybe(x) is
    Some(x) or
    None
";

/// Range declares with doc comments (~24 lines, ~750 bytes).
const INT_GIN: &str = "\
--- The 8-bit signed integer type.
SignedTinyInt is in -128...127
--- The 16-bit signed integer type.
SignedSmallInt is in -32768...32767
--- The 32-bit signed integer type.
SignedInt is in -2147483648...2147483647
--- The 64-bit signed integer type.
SignedBigInt is in -9223372036854775808...9223372036854775807
--- The 8-bit unsigned integer type.
TinyInt is in 0...255
--- The 16-bit unsigned integer type.
SmallInt is in 0...65535
--- The 32-bit unsigned integer type.
Int is in 0...4294967295
--- The 64-bit unsigned integer type.
BigInt is in 0...18446744073709551615
--- Alias for the 8-bit unsigned integer type.
Byte is TinyInt
";

/// Auto trait + when-is (~32 lines, ~820 bytes).
const COPY_GIN: &str = "\
Type is Primitive(width BigInt, signed Bool)
     or Record(name String, fields List(NamedTy))
     or Union(name String, variants List(VariantShape))
     or Ptr(inner Type)
     or Opaque(name String)

NamedTy has name String, ty Type
VariantShape has name String, fields List(NamedTy)
Bool is True or False
BigInt is in 0...18446744073709551615
List(x) has pointer Pointer(x), length BigInt
String has bytes List(BigInt)

Copy has can_copy Bool: is_copy(Self)

is_copy(x Type) Bool := when x is
    Primitive(_, _)     then True
    Ptr(_)              then False
    Opaque(_)           then False
    Record(_, fields)   then all_named_copy(fields)
    Union(_, variants)  then all_variants_copy(variants)

all_named_copy(fields List(NamedTy)) Bool := when fields is
    []                  then True
    [f, ...rest]        then is_copy(f.ty) and all_named_copy(rest)

all_variants_copy(variants List(VariantShape)) Bool := when variants is
    []                  then True
    [v, ...rest]        then all_named_copy(v.fields) and all_variants_copy(rest)
";

/// When-is recursion + add/mul helpers (~53 lines, ~1.3KB).
const SIZED_GIN: &str = "\
Type is Primitive(width BigInt, signed Bool)
     or Record(name String, fields List(NamedTy))
     or Union(name String, variants List(VariantShape))
     or Ptr(inner Type)
     or Opaque(name String)

NamedTy has name String, ty Type
VariantShape has name String, fields List(NamedTy)
Bool is True or False
BigInt is in 0...18446744073709551615
List(x) has pointer Pointer(x), length BigInt
String has bytes List(BigInt)

Size is Const(BigInt) or Dynamic
Sized has size Size: compute_size(Self)

compute_size(x Type) Size := when x is
    Primitive(w, _)     then Const(w / 8)
    Ptr(_)              then Const(8)
    Opaque(_)           then Dynamic
    Record(_, fields)   then sum_named(fields)
    Union(_, variants)  then union_size(variants)

sum_named(fields List(NamedTy)) Size := when fields is
    []                  then Const(0)
    [f, ...rest]        then add(compute_size(f.ty), sum_named(rest))

add(a Size, b Size) Size := when (a, b) is
    (Const(x), Const(y)) then Const(x + y)
                         else Dynamic

union_size(variants List(VariantShape)) Size := add(union_disc(variants), union_max_payload(variants))

union_disc(variants List(VariantShape)) Size := when variants is
    []           then Const(0)
    [_]          then Const(1)
                 else Const(2)

union_max_payload(variants List(VariantShape)) Size := when variants is
    []              then Const(0)
    [v, ...rest]    then max_size(sum_named(v.fields), union_max_payload(rest))

max_size(a Size, b Size) Size := when (a, b) is
    (Const(x), Const(y)) then Const(when x > y then x else y)
                         else Dynamic
";

/// IO pipeline fixture (~62 lines, ~1.6KB).
const IO_GIN: &str = "\
Int is in 0...4294967295
Pointer(x) is @x

write_spec := 'sink'

write(fd Int, buf Pointer(Int), len Int) Int:
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
    s.push_str("Int is in 0...18446744073709551615\n");
    for i in 0..400 {
        s.push_str("IntType");
        s.push_str(&i.to_string());
        s.push_str(" is in ");
        s.push_str(&i.to_string());
        s.push_str("...0x");
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
