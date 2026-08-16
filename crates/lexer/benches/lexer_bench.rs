use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use lexer::Lexer;

// Synthetic Gin sources kept small and inline so benchmarks don't depend on
// external files. Each exercises the lexer with different constructs.

/// Simple Tag `is` declare (~8 lines).
const MAYBE_GIN: &str = "\
Maybe(x) is
    Some(x) or
    None
";

/// Range declares with doc comments (~24 lines).
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

/// Auto trait + when-is (~32 lines).
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

/// When-is recursion + add/mul helpers (~53 lines).
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

/// Value pipeline + format strings (~60 lines).
const IO_GIN: &str = "\
Int is in 0...4294967295

Counter has value Int, step Int

Counter.new(start Int) Counter:
    return Counter(start, 1)

Counter.bump(self Counter) Counter:
    if self.step < 0:
        return self
    return Counter(self.value + self.step, self.step)

Counter.scale(self Counter, factor Int) Counter:
    return Counter(self.value * factor, self.step * factor)

Counter.log(self Counter) String:
    return self.value .. '-' .. self.step

emit(label String, count Int) Int:
    if count <= 0:
        return count
    return emit(label, count - 1)

run(value Int):
    state := Counter.new(value)
    next := state.bump()
    final := next.scale(2)
    return final.log()

pipe(value Int) Int:
    text := '\n'
    total := emit('items', value)
    if total > 0:
        return total
    return value + 1
";

/// A larger module with declarations + method-like pipeline (~100 lines).
const PIPE_GIN: &str = "\
ModuleState has
    name Str,
    values List(BigInt),

ModuleState.empty(name Str) ModuleState:
    return ModuleState(name, [])

ModuleState.push(self ModuleState, value BigInt) ModuleState:
    values := self.values.push(value)
    return ModuleState(self.name, values)

ModuleState.pop(self ModuleState) (ModuleState, BigInt):
    return self, 0

ModuleState.total(self ModuleState) BigInt:
    return reduce(self.values)

reduce(values List(BigInt)) BigInt := when values is
    []                  then 0
    [x, ...rest]        then x + reduce(rest)

reduce_two(a BigInt, b BigInt) BigInt := when (a, b) is
    (_, _) then a + b

State has
    current BigInt,

State.new() State:
    return State(0)

State.next(self State, delta BigInt) State:
    return State(self.current + delta)

State.merge(a State, b State) State:
    return State(a.current + b.current)

combine(a BigInt, b BigInt) BigInt := when (a, b) is
    (0, y) then y
    (x, 0) then x
    (x, y) then x + y

register(value BigInt):
    state := State.new()
    updated := state.next(value)
    backup := combine(updated.current, updated.current)
    return backup
";

/// Combined bundle of import-free files for a larger lex workload (~193 lines, ~8KB).
fn bundle_source() -> String {
    let mut s = String::with_capacity(16 * 1024);
    s.push_str(MAYBE_GIN);
    s.push('\n');
    s.push_str(INT_GIN);
    s.push('\n');
    s.push_str(PIPE_GIN);
    s
}

fn large_int_source() -> String {
    let mut src = String::with_capacity(64 * 1024);
    src.push_str("Int is in 0...18446744073709551615\n");
    for i in 0..400 {
        src.push_str("let a");
        src.push_str(&i.to_string());
        src.push_str(" := ");
        src.push_str("1_");
        src.push_str(&(i * 7).to_string());
        src.push_str(" + 0x");
        src.push_str(&format!("{:x}", i * 31));
        src.push_str(" + ");
        src.push_str("3.14");
        src.push('\n');
    }
    src
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

    let bundle = bundle_source();

    let inputs: &[(&str, &str)] = &[
        ("maybe", MAYBE_GIN),
        ("int", INT_GIN),
        ("copy", COPY_GIN),
        ("sized", SIZED_GIN),
        ("io", IO_GIN),
        ("pipe", PIPE_GIN),
        ("bundle", &bundle),
        ("large_int", &large_int_source()),
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
