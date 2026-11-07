//! `#load` and `#store` mark body-less functions as typed reads and writes of memory.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict fixture");
    check_file(&file, Limits::default()).map(|_| ())
}

const PRELUDE: &str = "#unrestricted\nByte is in 0 to 255\n\n";
const LOAD: &str = "#load\nload(element Type, pointer ref @element, index in 0 to 15) element\n\n";
const STORE: &str = "#store\nstore(element Type, pointer ref @element, index in 0 to 15, value element) ()\n\n";

fn with(tail: &str) -> String {
    format!("{PRELUDE}{LOAD}{STORE}{tail}")
}

#[test]
fn a_load_and_a_store_are_called_like_functions() {
    let ok = with("copy(pointer ref @Byte, from in 0 to 15, into in 0 to 15):\n    value: load(Byte, pointer, from)\n    store(Byte, pointer, into, value)\n    return\n");
    assert_eq!(check(&ok), Ok(()));
}

#[test]
fn the_value_stored_has_the_element_type() {
    let bad = format!("{PRELUDE}Other is in 0 to 9\n{LOAD}{STORE}put(pointer ref @Byte, value in 0 to 9):\n    store(Byte, pointer, 0, value)\n    return\n");
    assert!(check(&bad).is_err());
}

#[test]
fn a_marker_on_the_wrong_shape_is_an_instruction_target_error() {
    for text in [
        // No element type.
        format!("{PRELUDE}#load\nload(pointer ref @Byte, index in 0 to 15) Byte\n"),
        // The pointer is not a pointer.
        format!("{PRELUDE}#load\nload(element Type, pointer ref element, index in 0 to 15) element\n"),
        // A store that answers something.
        format!("{PRELUDE}#store\nstore(element Type, pointer ref @element, index in 0 to 15, value element) Byte\n"),
        // A load with a value to write.
        format!("{PRELUDE}#load\nload(element Type, pointer ref @element, index in 0 to 15, value element) element\n"),
    ] {
        assert_eq!(check(&text), Err(SourceCheckError::InstructionTarget), "{text}");
    }
}

#[test]
fn a_marker_must_precede_a_body_less_function() {
    let with_body = format!("{PRELUDE}#load\nload(element Type, pointer ref @element, index in 0 to 15) element: return 0\n");
    assert_eq!(check(&with_body), Err(SourceCheckError::InstructionTarget));
    let nothing = format!("{PRELUDE}#store\n");
    assert_eq!(check(&nothing), Err(SourceCheckError::InstructionTarget));
    let doubled = format!("{PRELUDE}#load\n#store\nstore(element Type, pointer ref @element, index in 0 to 15, value element) ()\n");
    assert_eq!(check(&doubled), Err(SourceCheckError::Duplicate));
}

#[test]
fn an_unmarked_body_less_function_is_still_refused() {
    let text = format!("{PRELUDE}load(element Type, pointer ref @element, index in 0 to 15) element\n");
    assert!(check(&text).is_err());
}

#[test]
fn a_load_of_a_type_parameter_is_a_value_of_that_parameter() {
    // Regression (G-D): the result of a `#load` at a type parameter used to be refused as
    // `source-check-type`, since no summary may invent a value of an unknown type.
    let get = "get(item Type, pointer ref @item, index in 0 to 15) item: return load(item, pointer, index)\n";
    assert_eq!(check(&with(get)), Ok(()));
    // And it passes on through a caller, which returns what `get` read.
    let forward = format!("{get}again(item Type, pointer ref @item) item: return get(item, pointer, 0)\n");
    assert_eq!(check(&with(&forward)), Ok(()));
}

#[test]
fn a_load_into_another_type_is_still_a_type_error() {
    let wrong = "get(item Type, pointer ref @item) Byte: return load(item, pointer, 0)\n";
    assert_eq!(check(&with(wrong)), Err(SourceCheckError::Type));
}

#[test]
fn a_load_of_another_modules_private_type_is_refused() {
    use typecheck::{CheckedSourceModule, check_module};
    let module = |name: &str, text: &str, deps: &[&CheckedSourceModule]| {
        let source = parser::parse_strict(text).expect("strict fixture");
        check_module(name, &source, deps, Limits::default())
    };
    let library = module("lib.secret", "Secret has\n    #private\n    n in 0 to 9\n    release(self):\n        Secret(n) := self\n        return\n", &[]).unwrap();
    let forge = format!("use lib.secret Secret\n{PRELUDE}{LOAD}forge(pointer ref @Secret) Secret: return load(Secret, pointer, 0)\n");
    assert_eq!(module("app", &forge, &[&library]).map(|_| ()), Err(SourceCheckError::Privacy));
}

mod recording_casts {
    use typecheck::{Limits, check_file};

    fn recorded(text: &str, name: &str) -> bool {
        let file = parser::parse_strict(text).expect("strict fixture");
        let checked = check_file(&file, Limits::default()).expect("the fixture checks");
        let index = checked.functions.iter().position(|function| function.name == name).unwrap();
        checked.bodies[index].is_some()
    }

    const BASE: &str = "#unrestricted\nAddress is in 0 to 18446744073709551615\n#unrestricted\nByte is in 0 to 255\n";

    #[test]
    fn an_address_cast_keeps_its_register_and_is_recorded() {
        let ok = format!("{BASE}to_pointer(a Address) @Byte: return #unchecked(a as @Byte)\nto_number(p @Byte) in 0 to 18446744073709551615: return #unchecked(p as Address)\nmain(): return\n");
        let _ = ok;
        let text = format!("{BASE}to_pointer(a Address) @Byte: return #unchecked(a as @Byte)\nmain(): return\n");
        assert!(recorded(&text, "to_pointer"));
    }

    #[test]
    fn a_pointer_read_as_a_domain_stays_unrecorded() {
        // Not a register-to-register cast the recorder reproduces: the body is left unrecorded
        // (the checker still accepts it), never guessed.
        let text = format!("{BASE}to_domain(p @Byte) Address: return #unchecked(p as Address)\nmain(): return\n");
        assert!(!recorded(&text, "to_domain"));
    }
}
