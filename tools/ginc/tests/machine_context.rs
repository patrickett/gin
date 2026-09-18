use ginc::cli::{Args, Profile};
use ginc::compile::GinCompiler;
use std::io::Write;
use std::process::{Command, Stdio};
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn machine_register_helpers_execute_with_runtime_inputs() {
    let target = host_target_triple();
    let source = include_str!("fixtures/machine_main.gin");
    let source = if target.starts_with("x86_64") {
        source.replace("x9", "rax").replace("x10", "rcx")
    } else {
        source.to_string()
    };
    let backend = test_fixtures::machine_backend_source(&target);
    let dependency = TempPackage::new("machine_backend_library");
    dependency.write_flask("micro");
    dependency.write("micro.gin", backend);
    let source = format!("use micro architecture\n\n{source}");
    let source = source.replace(
        "runtime_print_int(result)",
        "runtime_print_int(result)\n    c: runtime_read_int\n    d: runtime_read_int\n    add(mut c, d)\n    runtime_print_int(c)\n    e: runtime_read_int\n    f: runtime_read_int\n    mov(mut e, f)\n    runtime_print_int(e)",
    );
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new("machine_register_helpers");
        let cache = TempPackage::new("machine_register_cache");
        let input = package.write("main.gin", &test_fixtures::with_runtime(&source));
        let executable = package.join("program");
        let mut args = Args {
            input,
            output: Some(executable.clone()),
            target: Some(target.clone()),
            profile,
            no_cache: true,
            dependencies: [("micro".to_string(), dependency.path().to_path_buf())].into(),
            gin_home: Some(flask::GinHome::new(cache.path().to_path_buf())),
            ..Default::default()
        };
        let result = GinCompiler::compile(&mut args);
        assert!(result.is_success(), "{result:?}");
        assert!(!result.has_typecheck_diagnostics(), "{result:?}");
        for (input, expected) in [
            ("20\n22\n", "42\n42\n22\n"),
            ("-13\n5\n", "-8\n-8\n5\n"),
            (
                "9223372036854775807\n1\n",
                "-9223372036854775808\n-9223372036854775808\n1\n",
            ),
        ] {
            let mut child = Command::new(&executable)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
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
}
