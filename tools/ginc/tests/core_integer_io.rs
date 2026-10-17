mod common;

use common::compile_exe_checked;
use ginc::cli::Profile;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use test_fixtures::TempPackage;

#[test]
fn integer_input_preserves_signed_limits_and_rejects_overflow() {
    let package = TempPackage::new("core_integer_io");
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/core");
    package.copy_tree(&core, "core");
    package.write_flask_with_deps("core_integer_io", "core: './core'\n");
    let input = package.write(
        "main.gin",
        "use core.io read_int, print_int\n#entry\nmain:\n\
         \x20   for index in 0 to 8\n\
         \x20       print_int(read_int)\n\
         \x20   return 0\n",
    );
    let values = "0\n+17\n-42\n9223372036854775807\n-9223372036854775808\n\
                  9223372036854775808\n-9223372036854775809\n999999999999999999999999\n 0012\n";
    let expected = "0\n17\n-42\n9223372036854775807\n-9223372036854775808\n0\n0\n0\n12\n";
    for profile in [Profile::Debug, Profile::Release] {
        let executable = package.join(format!("integer-io-{profile:?}"));
        compile_exe_checked(input.clone(), &executable, profile);
        let mut process = Command::new(executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        process
            .stdin
            .take()
            .unwrap()
            .write_all(values.as_bytes())
            .unwrap();
        let output = process.wait_with_output().unwrap();
        assert!(output.status.success(), "{profile:?}: {output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            expected,
            "{profile:?}"
        );
        assert!(output.stderr.is_empty(), "{profile:?}: {output:?}");
    }
}
