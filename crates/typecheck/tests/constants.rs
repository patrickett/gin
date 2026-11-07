//! Zero-argument module constants (`name := expr`) are evaluated at check time and are
//! usable in domain bounds, array lengths, number-parameter arguments and claims.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, Target, check_module};

fn compile_with(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
    bits: u32,
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    let limits = Limits {
        target: Target {
            triple: "test-triple",
            register_bits: bits,
        },
        ..Limits::default()
    };
    check_module(name, &source, dependencies, limits)
}

fn compile(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    compile_with("case.constants", text, &[], 64)
}

#[test]
fn a_shift_and_add_chain_bounds_a_domain() {
    let source = "\
bits := 4
top := (1 << bits) - 1
#unrestricted
Small is in 0 to top
ok(x Small) Small: x
";
    compile(source).expect("checks");
}

#[test]
fn a_constant_bound_is_exact() {
    // 15 is the top; a function promising 0 to 14 for a Small is refused.
    let source = "\
bits := 4
top := (1 << bits) - 1
#unrestricted
Small is in 0 to top
narrow(x in 0 to top) in 0 to 14: x
";
    assert!(compile(source).is_err());
}

#[test]
fn arithmetic_forms_are_exact() {
    let source = "\
a := 3
b := a * a + 2
c := (b << 2) >> 1
d := -c + 100
#unrestricted
Exact is in d to d
ok(x in d to d) in 56 to 56: x
";
    // a=3, b=11, c=22, d=78
    assert!(compile(source).is_err(), "d is 78, not 56");
    let source = source.replace("56 to 56", "78 to 78");
    compile(&source).expect("checks");
}

#[test]
fn a_constant_may_reference_a_constant_declared_later() {
    let source = "\
#unrestricted
Small is in 0 to top
top := base + 1
base := 6
ok(x in 0 to top) in 0 to 7: x
";
    compile(source).expect("checks");
}

#[test]
fn a_cycle_is_an_error_not_a_hang() {
    let source = "\
a := b + 1
b := a + 1
#unrestricted
Bad is in 0 to a
";
    assert!(compile(source).is_err());
}

#[test]
fn a_negative_range_is_refused() {
    let source = "\
low := 10
high := 3
#unrestricted
Backwards is in low to high
";
    assert!(compile(source).is_err());
}

#[test]
fn a_bound_larger_than_the_checker_can_hold_is_refused() {
    let source = "\
huge := 1 << 300
#unrestricted
Big is in 0 to huge
";
    assert!(compile(source).is_err());
}

#[test]
fn a_constant_is_an_array_length() {
    let source = "\
size := 2 + 2
four(a [Item; size]) [Item; 4]: a
#unrestricted
Item is in 0 to 9
";
    compile(source).expect("checks");
    let wrong = source.replace("[Item; 4]", "[Item; 5]");
    assert!(compile(&wrong).is_err());
}

#[test]
fn a_constant_is_a_number_argument() {
    let source = "\
#unrestricted
Byte is in 0 to 255
#unrestricted
Length is in 0 to 1000
size := 8 * 2

Buf(element, capacity Length) has
    len in 0 to 1000

named(b Buf(Byte, size)) Buf(Byte, 16): b
";
    compile(source).expect("checks");
    let wrong = source.replace("Buf(Byte, 16): b", "Buf(Byte, 17): b");
    assert!(compile(&wrong).is_err());
}

#[test]
fn a_constant_is_usable_in_a_claim() {
    let source = "\
limit := 1 << 5
#unrestricted
Wide is in 0 to 1000
clamp(n in 0 to 1000 and <= limit) in 0 to 32: n
";
    compile(source).expect("checks");
    let wrong = source.replace("in 0 to 32", "in 0 to 31");
    assert!(compile(&wrong).is_err());
}

const TARGET_LIBRARY: &str = "\
index_bits := #target.register_bits
max_length := (1 << index_bits) - 1
#unrestricted
Length is in 0 to max_length
";

#[test]
fn the_target_register_width_reaches_the_bounds() {
    let source = format!("{TARGET_LIBRARY}ok(x in 0 to max_length) in 0 to 4294967295: x\n");
    // On 32 bits the top is 2^32 - 1, so the claim holds; on 64 bits it does not.
    compile_with("case.t32", &source, &[], 32).expect("checks on 32 bits");
    assert!(compile_with("case.t64", &source, &[], 64).is_err());
    let wide = format!("{TARGET_LIBRARY}ok(x in 0 to max_length) in 0 to 18446744073709551615: x\n");
    compile_with("case.t64", &wide, &[], 64).expect("checks on 64 bits");
}

#[test]
fn an_imported_constant_bounds_a_domain() {
    let library = compile_with(
        "core.slice",
        "bits := #target.register_bits\nmax_length := (1 << bits) - 1\n",
        &[],
        32,
    )
    .expect("library checks");
    let client = "\
use core.slice max_length
#unrestricted
Length is in 0 to max_length
ok(x in 0 to max_length) in 0 to 4294967295: x
";
    compile_with("app.client", client, &[&library], 32).expect("client checks");
    let wrong = client.replace("4294967295", "4294967294");
    assert!(compile_with("app.client", &wrong, &[&library], 32).is_err());
}

#[test]
fn an_unknown_constant_is_still_unknown() {
    let source = "#unrestricted\nBad is in 0 to nowhere\n";
    assert!(matches!(compile(source), Err(SourceCheckError::UnknownName)));
}

#[test]
fn a_constant_read_as_a_value_keeps_its_exact_range() {
    let source = "answer := 6 * 7\nf() in 42 to 42: answer\n";
    compile(source).expect("checks");
    // Out of range for the claimed domain: refused.
    assert!(compile("answer := 6 * 7\nf() in 0 to 41: answer\n").is_err());
}

#[test]
fn a_duplicate_constant_is_refused() {
    assert!(matches!(
        compile("a := 1\na := 2\n"),
        Err(SourceCheckError::Duplicate)
    ));
}

#[test]
fn a_parameter_hides_a_constant() {
    let source = "n := 100\nf(n in 0 to 3) in 0 to 3: n\n";
    compile(source).expect("checks");
}

#[test]
fn an_integer_constant_may_be_chosen_by_the_target() {
    // The target here is none of these named triples, so the `else` is taken: 5.
    let ok = "size := when #target.triple is 'no-such-target' then 1 else 5\n#unrestricted\nLimited is in 0 to size\nf() Limited: return 5\n";
    let file = parser::parse_strict(ok).unwrap();
    assert!(typecheck::check_file(&file, typecheck::Limits::default()).is_ok());
    // Negative controls: the other branch is not the one taken, and 6 is out of range.
    let other = ok.replace("then 1 else 5", "then 9 else 5").replace("return 5", "return 6");
    let file = parser::parse_strict(&other).unwrap();
    assert!(typecheck::check_file(&file, typecheck::Limits::default()).is_err());
}
