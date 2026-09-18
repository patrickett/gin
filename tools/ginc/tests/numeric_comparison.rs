use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;
use std::process::Command;
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn nominal_comparison_preserves_both_result_alternatives() {
    let package = TempPackage::new("numeric_comparison");
    package.write(
        "main.gin",
        r#"
Word is in -9223372036854775808 to 9223372036854775807
Word has
    #infix('<')
    less(lhs Word, rhs Word) True or False:
        left is in -9223372036854775808 to 9223372036854775807: lhs
        right is in -9223372036854775808 to 9223372036854775807: rhs
    return when left < right is True then True else False
    copy(value ref Word) Word:
        bits is in -9223372036854775808 to 9223372036854775807: value
    return bits as Word
    replace(previous Word, next Word) Word:
        discarded is in -9223372036854775808 to 9223372036854775807: previous
    return next
check(lhs Word, rhs Word) Word: when lhs < rhs is True then 1 else 0
main:
    value Word: 1
    yes is in 0 to 255: check(Word.copy(ref value), 2)
    value :: Word.replace(value, 2)
    no is in 0 to 255: check(value, 1)
return when yes = 1 is True and no = 0 is True then 0 else 1
"#,
    );
    let output = package.join("comparison");
    let mut args = Args {
        input: package.join("main.gin").parent().unwrap().to_path_buf(),
        output: Some(output.clone()),
        emit: Emit::Exe,
        profile: Profile::Debug,
        target: Some(host_target_triple()),
        no_cache: true,
        ..Default::default()
    };
    let result = GinCompiler::compile(&mut args);
    assert!(result.is_success(), "{result:?}");
    assert!(!result.has_typecheck_diagnostics(), "{result:?}");
    assert!(Command::new(output).status().unwrap().success());
}
