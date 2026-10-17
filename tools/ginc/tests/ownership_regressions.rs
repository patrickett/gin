mod common;

use common::compile_and_run_exe;
use ginc::cli::Profile;
use std::process::Command;
use test_fixtures::{TempPackage, host_target_triple};

const RESOURCE: &str = r#"Byte is in 0 to 255
Byte.finish(self): return
Resource has
    value @Byte
    release(self): return
"#;

const REJECTED_PROGRAMS: [(&str, &str, bool, &str); 6] = [
    (
        "captured_resource",
        r#"run(value Resource):
    finish(ignored Byte): value.release
    finish(0)
    finish(0)
    value.release
    return
"#,
        false,
        "type-use-of-moved-value",
    ),
    (
        "repeated_array",
        r#"run(value Resource):
    values: [value; 2]
    values.[0].release
    values.[1].release
    value.release
    return
"#,
        false,
        "type-sequence-repeat-seed-not-copyable",
    ),
    (
        "indexed_moves",
        r#"run(values [Resource; 2], i is in 0 to 1):
    values.[i].release
    values.[0].release
    values.[1].release
    return
"#,
        false,
        "type-use-of-maybe-moved-value",
    ),
    (
        "binding_transfer",
        r#"run(value Resource):
    alias: value
    value.release
    alias.release
    return
"#,
        true,
        "type-use-of-moved-value",
    ),
    (
        "local_terminal",
        "run(value Resource):\n    forget(item Resource): return\n    forget(value)\n    return\n",
        false,
        "type-terminal-value-not-consumed",
    ),
    (
        "early_return",
        r#"run(value own Resource, flag ref Byte):
    when flag is 0 then return
    value.release
    return
"#,
        true,
        "type-owned-param-not-consumed",
    ),
];

#[test]
fn ownership_regressions_prevent_object_emission() {
    for (name, source, defining_module, code) in REJECTED_PROGRAMS {
        let package = TempPackage::new(name);
        package.write_flask("ownership");
        let resource = format!("{RESOURCE}{}", if defining_module { source } else { "" });
        package.write("resource/resource.gin", &resource);
        let client = if defining_module { "" } else { source };
        let input = package.write(
            "main.gin",
            &format!("use ownership.resource Byte, Resource\n{client}#entry\nmain: 0\n"),
        );
        for profile in [Profile::Debug, Profile::Release] {
            let output = package.join(format!("rejected-{}.o", profile.as_str()));
            let result = Command::new(env!("CARGO_BIN_EXE_ginc"))
                .arg(&input)
                .args([
                    "--target",
                    &host_target_triple(),
                    "--emit",
                    "obj",
                    "--no-cache",
                ])
                .args(["--profile", profile.as_str()])
                .arg("--output")
                .arg(&output)
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(!result.status.success(), "{name}, {profile:?}: {stderr}");
            assert!(
                stderr.contains(&format!("[{code}]")),
                "{name}, {profile:?}: {stderr}"
            );
            assert!(!output.exists(), "{name}, {profile:?}");
        }
    }
}

#[test]
fn valid_local_capture_binding_and_array_transfers_release_once() {
    let resource = r#"Byte is in 0 to 255
Byte.finish(self): return
Byte.replace(previous Self, next Self) Self: return next
Resource has
    count @Byte
    release(self):
        pointer: self.count
        previous is in Byte: pointer.[0]
        pointer.[0]:: Byte.replace(pointer.[0], ((previous + 1) & 255) as Byte)
        return
transfer(value Resource):
    alias: value
    alias.release
    return
"#;
    let source = r#"use ownership.resource Byte, Resource, transfer
finish(value Resource):
    local(ignored Byte): value.release
    local(0)
    return
#entry
main:
    count Byte: 0
    finish(Resource(count: @count))
    transfer(Resource(count: @count))
    value: Resource(count: @count)
    array: [value; 1]
    array.[0].release
    return count
"#;
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new("ownership_transfers");
        package.write_flask("ownership");
        package.write("resource/resource.gin", resource);
        let input = package.write("main.gin", source);
        let output = compile_and_run_exe(
            input,
            &package.join("program"),
            profile,
            "",
            "run ownership transfers",
        );
        assert_eq!(output.status.code(), Some(3));
    }
}
