//! Facts the compiler need not know: the width of a domain in bytes, and the width of an index,
//! written in Gin from `domain of` and `#target`, and folded while compiling.
//!
//! The compiler reports only primitive facts (`#target`, the declared ranges of types). A
//! library derives the rest, so each is written once in the language and not once more in the
//! compiler.
use typecheck::{CheckedSourceBodyKind, CheckedSourceModule, Limits, Target, check_module};

fn check(text: &str) -> CheckedSourceModule {
    let source = parser::parse_strict(text).expect("valid syntax");
    let limits = Limits {
        target: Target::AARCH64_DARWIN,
        ..Limits::default()
    };
    check_module("case.derived", &source, &[], limits).expect("checks")
}

/// The number a function returns, when its body folded to one.
fn number(module: &CheckedSourceModule, function: &str) -> Option<i32> {
    let file = module.checked();
    let index = file.functions.iter().position(|f| f.name == function)?;
    let body = file.bodies[index].as_ref()?;
    let value = body.expressions.iter().find_map(|e| match &e.kind {
        CheckedSourceBodyKind::Return { value } => Some(*value),
        _ => None,
    })?;
    match &body.expressions[value as usize].kind {
        CheckedSourceBodyKind::Integer(number) => (0..100).find(|n| *number == (*n).into()),
        _ => None,
    }
}

const LIBRARY: &str = "\
#unrestricted
Byte is in 0 to 255
#unrestricted
Short is in 0 to 65535
#unrestricted
Word is in 0 to 4294967295

bytes_for(max in 0 to 4294967295) in 1 to 4:
    return when max <= 255 then 1 else when max <= 65535 then 2 else when max <= 16777215 then 3 else 4

byte_width() in 1 to 4: return bytes_for(#upper(Byte))
short_width() in 1 to 4: return bytes_for(#upper(Short))
word_width() in 1 to 4: return bytes_for(#upper(Word))

index_width() in 1 to 8: return when #target.register_bits <= 32 then 4 else 8
index_in_use() in 1 to 8: return index_width()
";

#[test]
fn the_width_of_a_domain_is_derived_not_asked_of_the_compiler() {
    let module = check(LIBRARY);
    assert_eq!(number(&module, "byte_width"), Some(1));
    assert_eq!(number(&module, "short_width"), Some(2));
    assert_eq!(number(&module, "word_width"), Some(4));
}

/// An index is as wide as a register, whatever the target says.
#[test]
fn the_width_of_an_index_follows_the_target() {
    assert_eq!(number(&check(LIBRARY), "index_in_use"), Some(8));
    let narrow = LIBRARY.to_owned();
    let source = parser::parse_strict(&narrow).expect("valid syntax");
    let limits = Limits {
        target: Target {
            triple: "i686-unknown-linux-gnu",
            register_bits: 32,
        },
        ..Limits::default()
    };
    let module = check_module("case.narrow", &source, &[], limits).expect("checks");
    assert_eq!(number(&module, "index_in_use"), Some(4));
}
