mod common;
use common::{compile_exe, compile_exe_checked};
use ginc::cli::{Args, Profile};
use ginc::compile::GinCompiler;
use resolve::GinPackageExt;
use std::process::Command;
use test_fixtures::TempPackage;

#[test]
fn count_number_conversions_preserve_values_and_receiver_conventions() {
    let package = TempPackage::new("core");
    package.write_flask("core");
    package.write(
        "ops/numeric.gin",
        include_str!("../../../modules/core/ops/numeric.gin"),
    );
    package.write(
        "alloc/count.gin",
        include_str!("../../../modules/core/alloc/count.gin"),
    );
    package.write(
        "slice/slice.gin",
        "Byte is in 0 to 255\nByte.release(self):\n    return\n",
    );
    let input = package.write(
        "main.gin",
        r#"use core.alloc Count
use core.ops Numeric
use core.slice Byte

measure(value ref has Numeric): return value.as_num * 8
consume(value has Numeric):
    observed: value.as_num
    consumed: value.to_num
    return (observed * 4) + (consumed * 4)

#entry
main:
    count Count(Byte): 7
    first: count.as_num
    second: count.as_num
    bytes: measure(count)
    consumed: count.to_num
    empty Count(Byte): 0
    zero: empty.to_num
    other Count(Byte): 3
    other_bytes: consume(other)
    return when first = 7 and second = 7 and bytes = 56 and consumed = 7 and zero = 0 and other_bytes = 24 then 0 else 1
        "#,
    );
    for profile in [Profile::Debug, Profile::Release] {
        let executable = package.join(format!("program-{profile:?}"));
        compile_exe_checked(input.clone(), &executable, profile);
        let output = Command::new(executable).output().unwrap();
        assert!(output.status.success(), "{profile:?}: {output:?}");
    }
}

#[test]
fn allocator_and_list_number_conversions_typecheck() {
    for relative_path in ["alloc/global.gin", "collection/List.gin"] {
        let input = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../modules/core")
            .join(relative_path)
            .canonicalize()
            .unwrap();
        let sources = input
            .parent()
            .unwrap()
            .collect_gin_files()
            .into_iter()
            .map(|path| {
                let source = std::fs::read_to_string(&path).unwrap();
                ginc::driver::SourceFile::new(path, source)
            })
            .collect();
        let mut driver = ginc::driver::CompilerDriver::default();
        let parsed = driver
            .parse(ginc::driver::SourcePackage::new(&input, sources))
            .unwrap();
        let checked = driver.check(
            ginc::driver::CompilePackage::new(parsed, flask::CompileTarget::Library)
                .with_import_resolution(true),
        );
        let flaws: Vec<_> = checked
            .files()
            .iter()
            .flat_map(|file| {
                file.front_end_diagnostics()
                    .iter()
                    .chain(file.typecheck_diagnostics())
                    .map(|diagnostic| (file.path(), diagnostic))
            })
            .collect();
        assert!(flaws.is_empty(), "{relative_path}: {flaws:?}");
    }
}

#[test]
fn whole_core_interface_accepts_absolute_and_relative_entry_paths() {
    let core = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../modules/core")
        .canonicalize()
        .unwrap();
    let cwd = std::env::current_dir().unwrap();
    let common = cwd
        .ancestors()
        .find(|ancestor| core.starts_with(ancestor))
        .unwrap();
    let relative = cwd
        .strip_prefix(common)
        .unwrap()
        .components()
        .fold(std::path::PathBuf::new(), |path, _| path.join(".."))
        .join(core.strip_prefix(common).unwrap());
    assert!(relative.is_relative());
    let outputs = TempPackage::new("core_interface_paths");
    for (index, input) in [core, relative].into_iter().enumerate() {
        let output = outputs.join(format!("core-{index}.ginif"));
        let result = GinCompiler::compile(&mut Args {
            input: input.clone(),
            output: Some(output.clone()),
            emit: ginc::cli::Emit::Interface,
            profile: Profile::Debug,
            no_cache: true,
            ..Default::default()
        });
        assert!(result.is_success(), "{input:?}: {result:?}");
        assert!(!result.has_typecheck_diagnostics(), "{input:?}: {result:?}");
        assert!(output.is_file());
    }
}

#[test]
fn integer_output_preserves_zero_sign_and_full_width_limits() {
    let package = TempPackage::new("integer_output");
    let core = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../modules/core")
        .canonicalize()
        .unwrap();
    for source in core.collect_gin_files() {
        let path = std::path::Path::new("core").join(source.strip_prefix(&core).unwrap());
        package.write(
            path.to_str().unwrap(),
            &std::fs::read_to_string(source).unwrap(),
        );
    }
    package.write(
        "core/mod.flask",
        include_str!("../../../modules/core/mod.flask"),
    );
    package.write_flask_with_deps(
        "integer_output",
        r#"core: "./core"
"#,
    );
    let input = package.write("main.gin", "use core.io print_int\n#entry\nmain:\n    print_int(0)\n    print_int(9)\n    print_int(10)\n    print_int(-1)\n    print_int(-10)\n    print_int(9223372036854775807)\n    print_int(-9223372036854775808)\n    return 0\n");
    for profile in [Profile::Debug, Profile::Release] {
        let executable = package.join(format!("program-{profile:?}"));
        compile_exe_checked(input.clone(), &executable, profile);
        let output = Command::new(executable).output().unwrap();
        assert!(output.status.success(), "{profile:?}: {output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "0\n9\n10\n-1\n-10\n9223372036854775807\n-9223372036854775808\n",
            "{profile:?}"
        );
    }
}

#[test]
fn loop_conversions_use_the_current_counter_for_owned_arguments() {
    for loop_head in ["while index < 4 do", "for ignored in 0 to 3"] {
        let package = TempPackage::new("loop_conversion");
        package.write_flask("loop_conversion");
        let input = package.write("main.gin", &format!("Count is in 0 to 255\nCount has\n    release(self):\n        return\nobserve(value Count) is in Count: return (number is in Count: value)\n#entry\nmain:\n    index is in Count: 0\n    last is in Count: 0\n    {loop_head}\n        when index >= 4 then\n            return last\n        current is in Count: index\n        last :: observe(current as Count)\n        index :: (next is in Count: index + 1)\n    return last\n"));
        for profile in [Profile::Debug, Profile::Release] {
            let executable = package.join(format!("program-{profile:?}"));
            let result = compile_exe(input.clone(), &executable.clone(), profile);
            assert!(
                result.is_success() && !result.has_typecheck_diagnostics(),
                "{loop_head} {profile:?}: {result:?}"
            );
            assert_eq!(
                Command::new(executable).status().unwrap().code(),
                Some(3),
                "{loop_head} {profile:?}"
            );
        }
    }
}

#[test]
fn statement_calls_preserve_conditional_argument_values() {
    let package = TempPackage::new("conditional_zero_argument");
    package.write_flask("conditional_zero_argument");
    let input = package.write(
        "main.gin",
        r#"Word is in 0 to 255
Word has release(self): return
replace(previous Word, next Word) Word: return next
write(address @Word, value Word):
    pointer: address
    pointer.[0]:: replace(pointer.[0], value)
    return
choose(flag Word) Word:
    value Word: 255
    write(@value, when flag = 0 then flag else 0)
    return value
#entry
main:
    first: choose(0)
    second: choose(1)
    return when first = 0 and second = 0 then 0 else 1
"#,
    );
    for profile in [Profile::Debug, Profile::Release] {
        let output = package.join(format!("program-{profile:?}"));
        compile_exe_checked(input.clone(), &output, profile);
        assert!(Command::new(output).status().unwrap().success());
    }
}
