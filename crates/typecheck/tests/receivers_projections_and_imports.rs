//! `call(...).field`, a receiver with claims (`self and ...`), a generic type's function that
//! writes its own type parameters, field claims, and the files of one module importing the
//! same name each.
use ast::SourceFile;
use typecheck::{
    CheckedSourceModule, Limits, SourceCheckError, check_file, check_module, check_module_files,
};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

const NUMBERS: &str = "#unrestricted\nByte is in 0 to 255\n\n#unrestricted\nPair has\n    first Byte\n    second Byte\n\nmake() Pair: return Pair(first: 1, second: 2)\n";

#[test]
fn a_field_of_a_call_result_is_taken() {
    check(&format!("{NUMBERS}pick() Byte: return make().second\n")).unwrap();
    // A call of known arguments is worked out at check time, so the field is exactly 2.
    check(&format!("{NUMBERS}pick() in 2 to 2:\n    return make().second\n")).unwrap();
    assert!(check(&format!("{NUMBERS}pick() in 1 to 1:\n    return make().second\n")).is_err());
    assert!(check(&format!("{NUMBERS}pick() Byte: return make().third\n")).is_err());
}

#[test]
fn the_other_fields_of_a_linear_result_must_not_owe_an_ending() {
    let linear = "#unrestricted\nByte is in 0 to 255\n\nToken has\n    number in 0 to 3\n\n    release(self):\n        Token(number) := self\n        return\n\nBoth has\n    byte Byte\n    token Token\n\nmake() Both: return Both(byte: 1, token: Token(number: 1))\n";
    // Taking the byte would drop the token.
    assert_eq!(
        check(&format!("{linear}pick() Byte: return make().byte\n")),
        Err(SourceCheckError::Unconsumed)
    );
}

const COUNTER: &str = "cap := 1024\n#unrestricted\nOff is in 0 to cap\n\nBuf has\n    length Off\n\n";

#[test]
fn a_receiver_may_carry_claims() {
    let release = "    release(self):\n        Buf(length) := self\n        return\n\n";
    check(&format!(
        "{COUNTER}{release}    grow(self mut and self.length < cap) where self.length :: self.length + 1:\n        self.length :: self.length + 1\n        return\n"
    ))
    .unwrap();
    // The claim is shown at the call.
    check(&format!(
        "{COUNTER}{release}    grow(self mut and self.length < cap) where self.length :: self.length + 1:\n        self.length :: self.length + 1\n        return\n\nstep(b mut Buf and b.length < cap):\n    b.grow\n    return\n"
    ))
    .unwrap();
}

#[test]
fn a_generic_types_function_may_write_its_own_type_parameters() {
    check(
        "Release has\n    release(self)\n\nHolder(element has Release) has\n    first element\n\n    release(self):\n        Holder(first) := self\n        element.release(first)\n        return\n\n    dispose(element has Release, held Holder(element)):\n        held.release\n        return\n",
    )
    .unwrap();
}

#[test]
fn a_field_may_carry_claims_on_its_siblings() {
    check("#unrestricted\nN is in 0 to 100\n\nPair has\n    cap N\n    used N and <= cap\n\n    release(self):\n        Pair(cap, used) := self\n        return\n").unwrap();
}

fn files(texts: &[&str]) -> Vec<SourceFile> {
    texts
        .iter()
        .enumerate()
        .map(|(index, text)| parser::parse_strict_file(text, index as u32).expect("fixture"))
        .collect()
}

#[test]
fn two_files_of_a_module_may_each_import_the_same_name() {
    let library: CheckedSourceModule = check_module(
        "lib",
        &parser::parse_strict("#unrestricted\nByte is in 0 to 255\n").unwrap(),
        &[],
        Limits::default(),
    )
    .unwrap();
    let texts = [
        "use lib Byte\nfirst(b Byte) Byte: return b\n",
        "use lib Byte\nsecond(b Byte) Byte: return b\n",
    ];
    check_module_files("app", &files(&texts), &[&library], Limits::default()).unwrap();
}
