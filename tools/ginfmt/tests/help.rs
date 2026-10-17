use flask::CompileTarget;
use ginfmt::help;
use test_fixtures::TempPackage;

fn fixture(source: &str) -> (TempPackage, std::path::PathBuf) {
    let package = TempPackage::new("format_help");
    package.write_flask("format_help");
    let file = package.write("main.gin", source);
    (package, file)
}

#[test]
fn root_expression_uses_the_same_receiver_help_as_a_function_body() {
    let source = concat!(
        "Word is in 0 to 255\nBox has\n    value Word\n",
        "    read(self ref) Word: return self.value\n",
        "(Box.read(Box(value: 7)))\n",
    );
    let (_package, path) = fixture(source);
    let hints = help::suggestions(&path, CompileTarget::Library).unwrap();
    let hint = hints
        .iter()
        .find(|hint| hint.diagnostic.code.slug() == "fmt-receiver-call")
        .unwrap();
    assert_eq!(
        &source[hint.edits[0].span.to_range()],
        "(Box.read(Box(value: 7)))"
    );
    assert_eq!(hint.edits[0].replacement, "(Box(value: 7).read)");
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 1);
    assert!(
        std::fs::read_to_string(path)
            .unwrap()
            .ends_with("(Box(value: 7).read)\n")
    );
}

#[test]
fn receiver_help_preserves_mutation_borrows_and_constructor_shorthand() {
    let source = r#"Word is in 0 to 255
Box has
    value Word
    read(self ref) Word: return self.value
    replace(previous Word, next Word) Word: return next
    write(self mut, next Word):
        self.value:: Box.replace(self.value, next)
        return
run:
    value Word: 7
    box: Box(value: value)
    Box.write(box, Box.read(box))
    result: Box.read(box)
    return
"#;
    let (_package, path) = fixture(source);
    let hints = help::suggestions(&path, CompileTarget::Library).unwrap();
    assert!(
        hints
            .iter()
            .any(|hint| hint.edits.iter().any(|edit| edit.replacement == "box.read"))
    );
    assert!(
        hints
            .iter()
            .any(|hint| hint.diagnostic.code.slug() == "fmt-field-shorthand")
    );
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 3);
    let fixed = std::fs::read_to_string(&path).unwrap();
    assert!(fixed.contains("box: Box(value)"), "{fixed}");
    assert!(fixed.contains("box.write(box.read)"), "{fixed}");
    assert!(fixed.contains("Box.replace(self.value, next)"), "{fixed}");
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 0);
}

#[test]
fn comments_and_unknown_methods_are_not_rewritten() {
    let source = r#"Word is in 0 to 255
Box has
    value Word
    read(self ref) Word: return self.value
run(receiver ref Box):
    first: Box.read(receiver -- keep this comment
    )
    unknown: Missing.read(receiver)
    return
"#;
    let (_package, path) = fixture(source);
    assert!(
        help::suggestions(&path, CompileTarget::Library)
            .unwrap()
            .is_empty()
    );
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 0);
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn same_named_fields_and_non_receiver_functions_keep_explicit_calls() {
    let source = r#"Word is in 0 to 255
Box has
    read Word
    read(self ref) Word: return self.read
    helper(value ref Self) Word: return value.read
run(receiver ref Box):
    first: Box.read(receiver)
    second: Box.helper(receiver)
    return
"#;
    let (_package, path) = fixture(source);
    assert!(
        help::suggestions(&path, CompileTarget::Library)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn imported_generic_methods_use_the_resolved_signature() {
    let (package, path) = fixture(
        r#"use format_help.storage Buffer
Word is in 0 to 255
run:
    value Buffer(Word): Buffer(value: 7 as Word)
    read: Buffer.read(value)
    return
"#,
    );
    package.write(
        "storage/storage.gin",
        "Buffer(element) has\n    value element\n    read(self ref) element: return self.value\n",
    );
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 1);
    assert!(
        std::fs::read_to_string(path)
            .unwrap()
            .contains("read: value.read")
    );
}

#[test]
fn uninstantiated_generic_bodies_prefer_receiver_calls() {
    let (_package, path) = fixture(
        r#"Buffer(element) has
    value element
    read(self ref) element: return self.value
    copy(self ref) Self: return Buffer(value: Buffer.read(self))
"#,
    );
    let hints = help::suggestions(&path, CompileTarget::Library).unwrap();
    assert!(
        hints.iter().any(|hint| hint
            .edits
            .iter()
            .any(|edit| edit.replacement == "self.read")),
        "{hints:?}"
    );
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 1);
    assert!(
        std::fs::read_to_string(path)
            .unwrap()
            .contains("Buffer(value: self.read)")
    );
}

#[test]
fn duplicate_parameter_names_are_formatter_help_and_rename_uses() {
    let source = "Word is in 0 to 255\nAllocation(element) has value element\nconsume(element, allocation Allocation(element)) Allocation(element): return allocation\n";
    let (_package, path) = fixture(source);
    let hints = help::suggestions(&path, CompileTarget::Library).unwrap();
    let hint = hints
        .iter()
        .find(|hint| hint.diagnostic.code.slug() == "fmt-duplicate-parameter-name")
        .unwrap();
    assert_eq!(hint.diagnostic.category, diagnostic::Category::Help);
    assert_eq!(hint.diagnostic.arg("suggestion"), Some("a"));
    assert_eq!(hint.edits.len(), 2);
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 1);
    let fixed = std::fs::read_to_string(&path).unwrap();
    assert!(
        fixed.contains("consume(element, a Allocation(element)) Allocation(element): return a"),
        "{fixed}"
    );
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 0);
}

#[test]
fn session_parameter_help_preserves_the_type_and_renames_body_uses() {
    let source = "Session is Open or Closed\nExitStatus is Success or Failure\nrun(session Session) ExitStatus:\n    terminal: session\n    return Failure\n";
    let (_package, path) = fixture(source);
    let hints = help::suggestions(&path, CompileTarget::Library).unwrap();
    let hint = hints
        .iter()
        .find(|hint| hint.diagnostic.code.slug() == "fmt-duplicate-parameter-name")
        .unwrap();
    assert_eq!(hint.diagnostic.arg("suggestion"), Some("s"));
    assert_eq!(hint.edits.len(), 2);
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 1);
    let fixed = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        fixed,
        "Session is Open or Closed\nExitStatus is Success or Failure\nrun(s Session) ExitStatus:\n    terminal: s\n    return Failure\n"
    );
}

#[test]
fn bodyless_method_parameters_get_help_but_fields_and_variables_do_not() {
    let source = "Allocation(element) has value element\nHolder(element) has allocation Allocation(element)\nPool has\n    reclaim(self mut, element, allocation Allocation(element))\nallocation Allocation(Word)\nWord is in 0 to 255\n";
    let (_package, path) = fixture(source);
    let hints = help::suggestions(&path, CompileTarget::Library).unwrap();
    assert_eq!(hints.len(), 1, "{hints:?}");
    assert_eq!(hints[0].diagnostic.arg("parameter"), Some("allocation"));
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 1);
    let fixed = std::fs::read_to_string(&path).unwrap();
    assert!(
        fixed.contains("reclaim(self mut, element, a Allocation(element))"),
        "{fixed}"
    );
    assert!(fixed.contains("Holder(element) has allocation Allocation(element)"));
    assert!(fixed.contains("\nallocation Allocation(Word)"));
}

#[test]
fn parameter_abbreviations_avoid_other_parameters_and_local_names() {
    let source = "Allocation is in 0 to 255\nkeep(a Allocation, allocation Allocation) Allocation: return allocation\nother(allocation Allocation) Allocation:\n    a: 0 as Allocation\n    return allocation\n";
    let (_package, path) = fixture(source);
    let hints = help::suggestions(&path, CompileTarget::Library).unwrap();
    assert_eq!(hints.len(), 2, "{hints:?}");
    assert!(
        hints
            .iter()
            .all(|hint| hint.diagnostic.arg("suggestion") == Some("al"))
    );
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 2);
    let fixed = std::fs::read_to_string(&path).unwrap();
    assert!(
        fixed.contains("keep(a Allocation, al Allocation) Allocation: return al"),
        "{fixed}"
    );
    assert!(fixed.contains("other(al Allocation)"));
    assert!(fixed.contains("    a: 0 as Allocation\n    return al"));
}

#[test]
fn parameter_rename_preserves_shadowed_names_and_other_functions() {
    let source = "Allocation is in 0 to 255\nkeep(allocation Allocation) Allocation:\n    before: allocation\n    allocation: 1 as Allocation\n    return allocation\nother(value Allocation) Allocation: return value\n";
    let (_package, path) = fixture(source);
    let hints = help::suggestions(&path, CompileTarget::Library).unwrap();
    assert_eq!(hints.len(), 1, "{hints:?}");
    assert_eq!(hints[0].edits.len(), 2, "{hints:?}");
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 1);
    let fixed = std::fs::read_to_string(&path).unwrap();
    assert!(fixed.contains("keep(a Allocation) Allocation:\n    before: a\n    allocation: 1 as Allocation\n    return allocation"), "{fixed}");
    assert!(fixed.contains("other(value Allocation) Allocation: return value"));
}

#[test]
fn reference_parameter_names_can_be_shortened() {
    let source = "Allocation has value Word\nWord is in 0 to 255\nread(allocation ref Allocation) Word: return allocation.value\n";
    let (_package, path) = fixture(source);
    let hints = help::suggestions(&path, CompileTarget::Library).unwrap();
    assert_eq!(hints.len(), 1, "{hints:?}");
    assert_eq!(hints[0].edits.len(), 2, "{hints:?}");
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 1);
    let fixed = std::fs::read_to_string(&path).unwrap();
    assert!(
        fixed.contains("read(a ref Allocation) Word: return a.value"),
        "{fixed}"
    );
}
