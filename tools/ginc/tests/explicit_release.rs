mod common;
use common::compile_exe;
use ginc::cli::Profile;
use std::process::Command;
use test_fixtures::TempPackage;

#[test]
fn release_runs_only_when_explicitly_called() {
    for profile in [Profile::Debug, Profile::Release] {
        for (body, expected) in [
            ("self.leaf.forget", 0),
            ("self.leaf.release", 1),
            ("take(self.leaf)", 1),
            (
                "decision: when observed = 0 then release_leaf(self.leaf) else forget_leaf(self.leaf)\n        ping(0)",
                1,
            ),
            (
                "decision: when observed = 1 then release_leaf(self.leaf) else forget_leaf(self.leaf)\n        ping(0)",
                0,
            ),
            (
                "self.leaf.forget\n        when observed = 0 then\n            return",
                0,
            ),
            (
                "when observed = 0 then\n            self.leaf.release\n            return\n        self.leaf.forget",
                1,
            ),
        ] {
            let package = TempPackage::new("explicit_release");
            package.write_flask("explicit_release");
            let input = package.write(
                "main.gin",
                &format!(
                    "\n{}    replace(previous Self, next Self) Self: return next\nLeaf has\n    counter @Word\n    forget(self):\n        return\n    release(self):\n        pointer: self.counter\n        count is in Word: pointer.[0]\n        pointer.[0]:: Word.replace(pointer.[0], (count + 1) as Word)\n        return\nrelease_leaf(value Leaf) Word:\n    value.release\n    return 0 as Word\nforget_leaf(value Leaf) Word:\n    value.forget\n    return 0 as Word\ntake(value own Leaf):\n    value.release\n    return\nping(value Word):\n    return\nBox has\n    leaf Leaf\n    release(self):\n        observed is in Word: self.leaf.counter.[0]\n        {body}\n        return\n#entry\nmain:\n    count: 0 as Word\n    value: Box(leaf: Leaf(counter: @count))\n    value.release\n    return count\n", test_fixtures::preludes::WORD_PRELUDE
                ),
            );
            let output = package.join("program");
            let result = compile_exe(input, &output.clone(), profile);
            assert!(
                result.is_success() && !result.has_typecheck_diagnostics(),
                "{profile:?}, {body}: {result:?}"
            );
            let run = Command::new(output).status().expect("run explicit release");
            assert_eq!(run.code(), Some(expected), "{profile:?}, {body}");
        }
    }
}

#[test]
fn imported_generic_methods_release_fields_explicitly() {
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new("explicit_generic");
        package.write_flask("explicit_generic");
        package.write(
            "storage/storage.gin",
            r#"
Release has
    release(self)
Box(element) has Release
    value element
    release(self):
        self.value.release
        return
"#,
        );
        let input = package.write(
            "main.gin",
            &format!("\nuse explicit_generic.storage Box, Release\n{}    replace(previous Self, next Self) Self: return next\nLeaf has\n    counter @Word\n    release(self):\n        pointer: self.counter\n        count is in Word: pointer.[0]\n        pointer.[0]:: Word.replace(pointer.[0], (count + 1) as Word)\n        return\nBorrower has Release\n    value ref Leaf\n    release(self):\n        return\nconsume(value Box(Leaf)):\n    value.release\n    return\ninspect(value ref Box(Leaf)) Word: return value.value.counter.[0]\n#entry\nmain:\n    count: 0 as Word\n    leaf: Leaf(counter: @count)\n    borrowed: Borrower(value: leaf)\n    borrowed.release\n    value Box(Leaf): Box(value: Leaf(counter: @count))\n    consume(value)\n    next Box(Leaf): Box(value: Leaf(counter: @count))\n    observed is in Word: inspect(next)\n    next.release\n    leaf.release\n    total is in Word: count\n    return when observed = 1 and total = 3 then 1 else 99\n", test_fixtures::preludes::WORD_PRELUDE),
        );
        let output = package.join("program");
        let result = compile_exe(input, &output.clone(), profile);
        assert!(
            result.is_success() && !result.has_typecheck_diagnostics(),
            "{profile:?}: {result:?}"
        );
        assert_eq!(
            Command::new(output).status().unwrap().code(),
            Some(1),
            "{profile:?}"
        );
    }
}

#[test]
fn explicit_release_handles_arrays_and_active_union_payloads() {
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new("explicit_shapes");
        package.write_flask("explicit_shapes");
        let input = package.write(
            "main.gin",
            &format!("\nRelease has\n    release(self)\n{}    replace(previous Self, next Self) Self: return next\nLeaf has\n    counter @Word\n    release(self):\n        pointer: self.counter\n        count is in Word: pointer.[0]\n        pointer.[0]:: Word.replace(pointer.[0], (count + 1) as Word)\n        return\nBundle has Release\n    items [Leaf; 2]\n    release(self):\n        self.items.[0].release\n        self.items.[1].release\n        return\nidle(value Word):\n    return\nMaybe is Some(Leaf) or None\nMaybe has Release\n    release(self): return when self is Some(value) then value.release else idle(0)\n#entry\nmain:\n    count: 0 as Word\n    first: Leaf(counter: @count)\n    second: Leaf(counter: @count)\n    bundle: Bundle(items: [first, second])\n    bundle.release\n    third: Leaf(counter: @count)\n    present Maybe: Some(third)\n    present.release\n    absent Maybe: None\n    absent.release\n    return count\n", test_fixtures::preludes::WORD_PRELUDE),
        );
        let output = package.join("program");
        let result = compile_exe(input, &output.clone(), profile);
        assert!(
            result.is_success() && !result.has_typecheck_diagnostics(),
            "{profile:?}: {result:?}"
        );
        assert_eq!(
            Command::new(output).status().unwrap().code(),
            Some(3),
            "{profile:?}"
        );
    }
}
