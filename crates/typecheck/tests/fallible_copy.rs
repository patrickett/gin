mod support;

use support::{check, flaw_codes};

#[test]
fn generic_fallible_copy_preserves_the_success_value() {
    let source = r#"Error is OutOfMemory or Fragmented
Error has release(self): return
CopyResult(element) is Copied(element) or Error
CopyResult(element) has
    release(self):
        when self is
            Copied(value) then element.release(value)
            Error(error) then error.release
        return
Copy has
    copy(self ref) CopyResult(Self): return Copied((bits is in Self: self) as Self)
Byte is in 0 to 255
Byte has Copy
    release(self): return
copy_value(element, source ref element) CopyResult(element):
    attempt: element.copy(source)
    return when attempt is
        Copied(value) then Copied(value)
        Error(error) then error
run:
    value Byte: 7
    attempt: copy_value(Byte, value)
    when attempt is
        Copied(copied) then copied.release
        Error(error) then error.release
    value.release
    return
"#;
    let typed = check(source);
    assert!(flaw_codes(&typed).is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn qualified_error_variants_inject_into_nested_results() {
    let typed = check(
        "Error is Empty or Full\nError has release(self): return\nWord is in 0 to 255\nWord has release(self): return\nResult is Word or Error\nResult has release(self):\n    when self is\n        Word(value) then value.release\n        Error(error) then error.release\n    return\nproduce() Result: return Error.Empty\n",
    );
    assert!(flaw_codes(&typed).is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn named_success_wrappers_preserve_the_owned_payload_state() {
    let typed = check(
        r#"Count is in 0 to 255
Count has release(self): return
Resource has
    count Count
    read(self ref and self.count = 1): return
    release(self): return
Copied(element) has
    value element
    take(self) element: return self.value
    release(self): return element.release(self.value)
Error has release(self): return
CopyResult(element) is Copied(element) or Error
CopyResult(element) has
    release(self):
        when self is
            Copied(value) then value.release
            Error(error) then error.release
        return
produce() copied Copied(Resource) thus copied.value.count = 1 or Error:
    return Copied(value: Resource(count: 1))
run:
    attempt: produce()
    when attempt is
        Copied(wrapper) then
            resource: wrapper.take
            resource.read
            resource.release
        Error(error) then error.release
    return
"#,
    );
    assert!(flaw_codes(&typed).is_empty(), "{:?}", typed.all_flaws());
}
