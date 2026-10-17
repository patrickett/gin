use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use lexer::Lexer;
use test_fixtures::bench_sources::{COPY_GIN, INT_GIN, MAYBE_GIN, SIZED_GIN};

// Sources shared with the parser benchmark live in `test-fixtures`; the
// lexer-only ones stay inline.

/// Value pipeline + format strings (~60 lines).
const IO_GIN: &str = "Int is in 0 to 4294967295\n\nCounter has value Int, step Int\n\nCounter.new(start Int) Counter:\n    return Counter(start, 1)\n\nCounter.bump(self Counter) Counter:\n    when self.step < 0: then\n        return self\n    return Counter(self.value + self.step, self.step)\n\nCounter.scale(self Counter, factor Int) Counter:\n    return Counter(self.value * factor, self.step * factor)\n\nCounter.log(self Counter) String:\n    return self.value .. '-' .. self.step\n\nemit(label String, count Int) Int:\n    when count <= 0: then\n        return count\n    return emit(label, count - 1)\n\nrun(value Int):\n    state := Counter.new(value)\n    next := state.bump()\n    final := next.scale(2)\n    return final.log()\n\npipe(value Int) Int:\n    text := '\n'\n    total := emit('items', value)\n    when total > 0: then\n        return total\n    return value + 1\n";

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
    src.push_str("Int is in 0 to 18446744073709551615\n");
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
