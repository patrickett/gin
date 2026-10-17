use diagnostic::Category;
use flask::CompileTarget;
use ginfmt::help;
use test_fixtures::TempPackage;

const TYPES: &str = "Word is in 0 to 255\nWord has\n    release(self):\n        return\nBox has\n    value Word\n    new(value Word) Self: return Self(value)\n    release(self):\n        return\nOuter has\n    value Box\n    release(self): return self.value.release\nEmpty has\n    new() Self: return Self()\n    release(self):\n        return\nconsume(value Box): return value.release\ntake_empty(value Empty): return value.release\n";

#[test]
fn nested_constructors_and_factories_have_advisory_help() {
    for expression in [
        "consume(Box(1))",
        "consume(Box.new(1))",
        "take_empty(Empty.new)",
        "Outer(value: Box(1))",
        "Outer(value: Box(1) as Box)",
    ] {
        let package = TempPackage::new("nested_construction");
        package.write_flask("nested_construction");
        let source = format!("{TYPES}run: {expression}\n");
        let path = package.write("main.gin", &source);
        let helps = help::suggestions(&path, CompileTarget::Library).unwrap();
        let hint = helps
            .iter()
            .find(|help| help.diagnostic.code.slug() == "fmt-nested-construction")
            .unwrap();
        assert_eq!(hint.diagnostic.category, Category::Help);
        assert!(hint.edits.is_empty());
        assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 0);
        assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    }
}

#[test]
fn advisory_help_does_not_prevent_verified_fixes_from_converging() {
    let package = TempPackage::new("advisory_and_fixable_help");
    package.write_flask("advisory_and_fixable_help");
    let path = package.write("main.gin", &format!("{TYPES}run:\n    consume(Box(1))\n"));
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 1);
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .ends_with("run: consume(Box(1))\n")
    );
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 0);
    assert!(
        help::suggestions(&path, CompileTarget::Library)
            .unwrap()
            .iter()
            .any(|help| help.diagnostic.code.slug() == "fmt-nested-construction")
    );
}

#[test]
fn named_values_and_constructor_receivers_do_not_have_nested_construction_help() {
    for body in [
        "\n    value: Box.new(1)\n    consume(value)\n    return",
        " Box.new(1).release",
    ] {
        let package = TempPackage::new("named_construction");
        package.write_flask("named_construction");
        let path = package.write("main.gin", &format!("{TYPES}run:{body}\n"));
        let helps = help::suggestions(&path, CompileTarget::Library).unwrap();
        assert!(
            helps
                .iter()
                .all(|help| help.diagnostic.code.slug() != "fmt-nested-construction"),
            "{helps:?}"
        );
    }
}

#[test]
fn an_unqualified_function_named_new_is_not_assumed_to_be_a_constructor() {
    let package = TempPackage::new("ordinary_new_function");
    package.write_flask("ordinary_new_function");
    let path = package.write(
        "main.gin",
        "new: 1\nidentity(value): return value\nrun: identity(new)\n",
    );
    assert!(
        help::suggestions(&path, CompileTarget::Library)
            .unwrap()
            .iter()
            .all(|help| help.diagnostic.code.slug() != "fmt-nested-construction")
    );
}
