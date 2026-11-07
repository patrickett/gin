//! `#target` is a record of the machine's primitive facts, known while compiling: `triple` as
//! text and `register_bits` as a number. Everything else is meant to be derived in Gin.
use typecheck::{
    CheckedSourceBodyKind, CheckedSourceModule, Limits, SourceCheckError, Target, check_module,
};

fn check_for(
    text: &str,
    target: Target,
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    let limits = Limits {
        target,
        ..Limits::default()
    };
    check_module("case.target", &source, &[], limits)
}

/// Code is chosen by the target, at compile time, with a case over its triple.
const PICK: &str = "\
syscall_number() in 0 to 999: return when #target.triple is
    'aarch64-apple-darwin' then 4
    'x86_64-unknown-linux-gnu' then 1
    else 0
";

fn number(module: &CheckedSourceModule, function: &str) -> Option<i32> {
    let file = module.checked();
    let index = file.functions.iter().position(|f| f.name == function)?;
    let body = file.bodies[index].as_ref()?;
    body.expressions.iter().find_map(|e| match &e.kind {
        CheckedSourceBodyKind::Integer(value) => (0..1000).find(|n| *value == (*n).into()),
        _ => None,
    })
}

#[test]
fn a_case_over_the_triple_picks_the_code_for_the_target() {
    for (target, expected) in [
        (Target::AARCH64_DARWIN, 4),
        (Target::X86_64_LINUX, 1),
        (Target::AARCH64_LINUX, 0),
    ] {
        let module = check_for(PICK, target).expect("checks");
        assert_eq!(number(&module, "syscall_number"), Some(expected), "{}", target.triple);
    }
}

#[test]
fn register_bits_is_a_number() {
    let source = "bits() in 0 to 128: return #target.register_bits\n";
    for target in [Target::AARCH64_DARWIN, Target::X86_64_LINUX] {
        let module = check_for(source, target).expect("checks");
        assert_eq!(number(&module, "bits"), Some(64));
    }
}

/// It takes part in arithmetic and refinements like any number the compiler knows.
#[test]
fn register_bits_can_be_computed_with() {
    let source = "bytes() in 8 to 8: return #target.register_bits / 8\n".replace("/ 8", "- 56");
    let module = check_for(&source, Target::AARCH64_DARWIN);
    assert!(module.is_ok(), "{:?}", module.err());
}

#[test]
fn only_triple_and_register_bits_exist() {
    let source = "f() in 0 to 9: return #target.nonsense\n";
    assert!(check_for(source, Target::AARCH64_DARWIN).is_err());
}

#[test]
fn another_query_is_still_not_defined() {
    let source = "f() in 0 to 9: return #machine.triple\n";
    assert_eq!(
        check_for(source, Target::AARCH64_DARWIN).unwrap_err(),
        SourceCheckError::Unsupported
    );
}

/// The default target is the machine running the compiler.
#[test]
fn the_default_target_is_the_host() {
    assert_eq!(Limits::default().target, Target::host());
}
