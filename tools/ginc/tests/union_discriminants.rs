mod common;
use common::compile_exe;
use ginc::cli::Profile;
use std::io::Write;
use std::process::{Command, Stdio};
use test_fixtures::TempPackage;

#[test]
fn numbered_unions_match_convert_and_store_across_module_boundaries() {
    let package = TempPackage::new("union_discriminants");
    package.write_flask("union_discriminants");
    package.write(
        "codes/codes.gin",
        "#bits(16)\nCode is Foo := 300 or Bar := -2 or Baz := 1\n",
    );
    let source = r#"use union_discriminants.codes Code
Word is in -9223372036854775808 to 9223372036854775807
Word.finish(self):
    return
Holder has
    code Code
    take(self) Code: return self.code
runtime_read_int Word: return runtime_number as Word
runtime_print_int(value Word) Word: return runtime_print(value as RuntimeInt) as Word
decode(value Word) Code: return when value = 300 then value as Code else Code.Bar
encode(value Code) Word: return value as Word
round_trip(value Code) Code: return (value as Word) as Code
inspect(value Code) Word: return when value is
    Foo then 30
    Bar then 20
    Baz then 10
#entry
main Word:
    runtime_print_int(encode(decode(runtime_read_int)))
    runtime_print_int(inspect(decode(runtime_read_int)))
    holder := Holder(code: round_trip(decode(runtime_read_int)))
    runtime_print_int(encode(holder.take))
    return 0
"#;
    let input_file = package.write("main.gin", &test_fixtures::with_runtime(source));
    let executable = package.join("union_discriminants");
    let result = compile_exe(
        input_file.parent().unwrap().to_path_buf(),
        &executable.clone(),
        Profile::Debug,
    );
    assert!(result.is_success(), "compilation failed: {result:?}");
    assert!(!result.has_typecheck_diagnostics(), "{result:?}");
    for (input, expected) in [
        ("300\n", "300\n30\n300\n"),
        ("-2\n", "-2\n20\n-2\n"),
        ("99\n", "-2\n20\n-2\n"),
    ] {
        let mut child = Command::new(&executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("run compiled numbered union");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.repeat(3).as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    }
}
