mod support;

#[test]
fn sum_and_array_temporaries_in_loops_use_entry_storage() {
    let source = format!(
        "{}\nMaybe(value) is Some(value) or None\nmake(value Word) Maybe(Word): return Maybe.Some(value)\narray(value Word) [Word; 2]: return [value, value]\n\nexercise(count is in Word) Word:\n    result Word: 0\n    for index in 0 to count\n        choice: make(7)\n        selected Word: when choice is Some(value) then value else 0\n        temporary: array(selected).[0]\n        result:: temporary\n    return result\n",
        test_fixtures::preludes::WORD_PRELUDE
    );
    let (llvm, diagnostics) = support::codegen_to_llvm_ir(&source, "stack_temporaries.gin");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(!llvm.is_empty());
    let mut entry = false;
    let mut allocations = 0;
    for line in llvm.lines() {
        if line.starts_with("define ") {
            entry = true;
        } else if line.starts_with('}')
            || (!line.starts_with(' ') && line.contains(':') && !line.starts_with(';'))
        {
            entry = false;
        }
        if line.contains(" = alloca ") {
            assert!(
                entry,
                "stack allocation outside entry block: {line}\n{llvm}"
            );
            allocations += 1;
        }
    }
    assert!(allocations >= 3, "expected sum and array storage\n{llvm}");
}
