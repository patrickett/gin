mod common;
use common::compile_exe;
use ginc::cli::Profile;
use std::process::Command;
use test_fixtures::TempPackage;

#[test]
fn receiver_parameter_conventions_preserve_record_fields() {
    for profile in [Profile::Debug, Profile::Release] {
        for receiver in ["value", "self"] {
            for convention in ["ref", "mut"] {
                let parameter = if receiver == "self" {
                    format!("self {convention}")
                } else {
                    format!("value {convention} Self")
                };
                let argument = "record";
                let source = format!(
                    r#"Word is in 0 to 255
Word has finish(self): return
Record has
    len Word
    finish(self): return

    length({parameter}) Word:
        result: {receiver}.len
        return result

#entry
main:
    record: Record(len: 7)
    result: Record.length({argument})
    record.finish
    return result
"#
                );
                let package = TempPackage::new("receiver_fields");
                let input = package.write("main.gin", &source);
                let output = package.join("program");
                let result = compile_exe(input, &output.clone(), profile);
                assert!(result.is_success(), "{parameter}: {result:?}");
                assert!(
                    !result.has_typecheck_diagnostics(),
                    "{parameter}: {result:?}"
                );
                let status = Command::new(output).status().expect("run receiver method");
                assert_eq!(status.code(), Some(7), "{profile:?}: {parameter}");
            }
        }
    }
}
