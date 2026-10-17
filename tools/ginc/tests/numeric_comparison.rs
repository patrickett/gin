mod common;
use common::compile_and_run_exe;
use ginc::cli::Profile;
use test_fixtures::TempPackage;

#[test]
fn nominal_comparison_preserves_both_result_alternatives() {
    let package = TempPackage::new("numeric_comparison");
    package.write(
        "main.gin",
        r#"
Word is in -9223372036854775808 to 9223372036854775807
Word has
    finish(self):
        return
    less(lhs Word, rhs Word) True or False:
        left is in -9223372036854775808 to 9223372036854775807: lhs
        right is in -9223372036854775808 to 9223372036854775807: rhs
        return when left < right then True else False
    copy(value ref Word) Word:
        bits is in -9223372036854775808 to 9223372036854775807: value
        return bits as Word
    replace(previous Word, next Word) Word:
        discarded is in -9223372036854775808 to 9223372036854775807: previous
        return next
check(lhs Word, rhs Word) is in 0 to 1: return when lhs < rhs then 1 else 0
#entry
main:
    value Word: 1
    yes is in 0 to 255: check(Word.copy(value), 2)
    value :: Word.replace(value, 2)
    no is in 0 to 255: check(value, 1)
    return when yes = 1 and no = 0 then 0 else 1
"#,
    );
    let output = compile_and_run_exe(
        package.join("main.gin").parent().unwrap().to_path_buf(),
        &package.join("comparison"),
        Profile::Debug,
        "",
        "run numeric comparison",
    );
    assert!(output.status.success());
}
