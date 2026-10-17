use flask::CompileTarget;
use ginfmt::help;
use test_fixtures::TempPackage;

fn fixture(source: &str) -> (TempPackage, std::path::PathBuf) {
    let package = TempPackage::new("direct_bind_help");
    package.write_flask("direct_bind_help");
    let path = package.write("main.gin", source);
    (package, path)
}

#[test]
fn single_expression_value_blocks_have_verified_fixes() {
    let source = "Word is in 0 to 255\nvalue Word:\n    1\n";
    let (_package, path) = fixture(source);
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 1);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "Word is in 0 to 255\nvalue Word: 1\n"
    );
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 0);
}

#[test]
fn explicit_function_returns_are_preserved() {
    for source in [
        "Word is in 0 to 255\nidentity(input Word) Word:\n    return input\n",
        "Word is in 0 to 255\nidentity(input Word) Word: return input\n",
        "Word is in 0 to 255\nStore has\n    value Word\n    read(self ref) Word:\n        return self.value\n",
        "Bool is True or False\nfinish:\n    return\nrun(flag Bool):\n    when flag is True then finish\n    return\n",
    ] {
        let (_package, path) = fixture(source);
        assert_eq!(
            help::fix(&path, CompileTarget::Library).unwrap(),
            0,
            "{source}"
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    }
}

#[test]
fn comments_and_multiple_statements_keep_their_value_blocks() {
    for source in [
        "Word is in 0 to 255\nvalue Word:\n    -- explanation\n    1\n",
        "Word is in 0 to 255\nvalue Word:\n    result: 1\n    result\n",
    ] {
        let (_package, path) = fixture(source);
        assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 0);
        assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    }
}
