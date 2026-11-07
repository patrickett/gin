//! Domain conversions are recorded, so programs that use domains evaluate.
use i256::I256;
use typecheck::{
    BodyResolver, CheckedSourceBody, CheckedSourceCallee, CheckedSourceFile, Evaluated, Limits,
    SourceCheckError, check_file, evaluate_value,
};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict domain fixture");
    check_file(&file, Limits::default())
}

const NUMERIC: &str = "\
Numeric has
    #infix('+')
    add(self, rhs Self):
        a in Self: self
        b in Self: rhs
        return a + b
";

struct Bodies<'a>(&'a [Option<CheckedSourceBody>]);

impl BodyResolver for Bodies<'_> {
    fn body(&self, callee: CheckedSourceCallee) -> Option<&CheckedSourceBody> {
        match callee {
            CheckedSourceCallee::Local { function } => self.0[function as usize].as_ref(),
            CheckedSourceCallee::Dependency { .. } => None,
        }
    }
}

fn run(checked: &CheckedSourceFile, function: &str, arguments: &[Evaluated]) -> Option<Evaluated> {
    let index = checked.functions.iter().position(|f| f.name == function)?;
    let body = checked.bodies[index].as_ref()?;
    evaluate_value(body, arguments, &Bodies(&checked.bodies), &mut 10_000)
}

fn int(value: u128) -> Evaluated {
    Evaluated::Integer(I256::from(value as i128))
}

/// A domain value is a record of schema `schema` holding its integer.
fn domain(schema: u32, value: u128) -> Evaluated {
    Evaluated::Record {
        schema,
        fields: vec![int(value)],
    }
}

fn byte(wrap: &str) -> String {
    format!("{NUMERIC}#unrestricted{wrap}\nByte is in 0 to 255\nByte has Numeric\n")
}

#[test]
fn an_integer_built_into_a_domain_is_recorded_and_evaluates_to_a_domain_value() {
    let source = format!("{}make() Byte: return 7\n", byte(""));
    let checked = check(&source).unwrap();
    assert_eq!(run(&checked, "make", &[]), Some(domain(0, 7)));
}

#[test]
fn a_domains_own_method_reads_self_as_an_integer_when_recorded() {
    let source = format!(
        "{NUMERIC}#unrestricted\nByte is in 0 to 255\nByte has Numeric\n    value(self) in 0 to 255: self\n"
    );
    let checked = check(&source).unwrap();
    assert_eq!(run(&checked, "Byte.value", &[domain(0, 42)]), Some(int(42)));
}

#[test]
fn the_provided_operator_evaluates_through_the_trait_default() {
    let source = format!("{}double(x Byte) Byte: return x + x\n", byte(", #wrap"));
    let checked = check(&source).unwrap();
    assert_eq!(
        run(&checked, "double", &[domain(0, 21)]),
        Some(domain(0, 42))
    );
}

#[test]
fn a_wrapping_domain_evaluates_to_the_residue_and_a_plain_one_is_refused() {
    let wrapping = format!("{}inc(x Byte) Byte: return x + 1\n", byte(", #wrap"));
    let checked = check(&wrapping).unwrap();
    assert_eq!(
        run(&checked, "inc", &[domain(0, 254)]),
        Some(domain(0, 255))
    );
    assert_eq!(run(&checked, "inc", &[domain(0, 255)]), Some(domain(0, 0)));
    let plain = format!("{}inc(x Byte) Byte: return x + 1\n", byte(""));
    assert_eq!(check(&plain), Err(SourceCheckError::Unproved));
}

#[test]
fn a_known_value_outside_a_wrapping_domain_folds_to_its_residue() {
    for (literal, residue) in [(256, 0), (1000, 232), (255, 255)] {
        let source = format!("{}f() Byte: return {literal}\n", byte(", #wrap"));
        let checked = check(&source).unwrap();
        assert_eq!(
            run(&checked, "f", &[]),
            Some(domain(0, residue)),
            "{literal}"
        );
    }
}

#[test]
fn a_wrapped_register_addition_evaluates_modulo_the_word() {
    let source = format!(
        "{NUMERIC}#unrestricted, #wrap\nWord is in 0 to 18446744073709551615\nWord has Numeric\nRegister has contents Word\nadd(left Register, right Register) Register:\n    Register(a) := left\n    Register(b) := right\n    return Register(contents: a + b)\n"
    );
    let checked = check(&source).unwrap();
    let register = |n: u128| Evaluated::Record {
        schema: 1,
        fields: vec![domain(0, n)],
    };
    let max = u128::from(u64::MAX);
    assert_eq!(
        run(&checked, "add", &[register(1), register(2)]),
        Some(register(3))
    );
    assert_eq!(
        run(&checked, "add", &[register(max), register(1)]),
        Some(register(0))
    );
    assert_eq!(
        run(&checked, "add", &[register(max), register(max)]),
        Some(register(max - 1))
    );
}

#[test]
fn an_annotated_local_converts_when_recorded() {
    let source = format!("{}f() Byte:\n    b Byte: 9\n    return b\n", byte(""));
    let checked = check(&source).unwrap();
    assert_eq!(run(&checked, "f", &[]), Some(domain(0, 9)));
}

#[test]
fn a_body_that_converts_nothing_still_records() {
    let source = format!(
        "{}Pair has\n    first Byte\n    second Byte\nswap(p Pair) Pair:\n    Pair(first, second) := p\n    return Pair(first: second, second: first)\n",
        byte("")
    );
    let checked = check(&source).unwrap();
    let index = checked
        .functions
        .iter()
        .position(|f| f.name == "swap")
        .unwrap();
    assert!(checked.bodies[index].is_some());
}
