mod common;
use common::compile_exe_checked;
use ginc::cli::Profile;
use ginc::driver::{CompilePackage, CompilerDriver, SourceFile, SourcePackage};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use test_fixtures::{TempPackage, host_target_triple};

fn package(name: &str, source: &str) -> (TempPackage, PathBuf) {
    let package = TempPackage::new(name);
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/core");
    package.copy_tree(&core, "core");
    package.write_flask_with_deps(
        name,
        r#"core: "./core"
"#,
    );
    let input = package.write("main.gin", source);
    (package, input)
}

fn compile(package: &TempPackage, input: &Path, profile: Profile) -> PathBuf {
    let output = package.join(format!("program-{profile:?}"));
    compile_exe_checked(input.to_owned(), &output, profile);
    output
}

#[test]
fn utf8_validation_matches_rust_including_error_positions_and_lengths() {
    let mut corpus = Vec::new();
    let mut count = 0;
    let mut add = |bytes: &[u8]| {
        let mut record = [0; 12];
        record[0] = bytes.len().try_into().unwrap();
        match std::str::from_utf8(bytes) {
            Ok(_) => record[2] = 255,
            Err(error) => {
                record[1] = error.valid_up_to().try_into().unwrap();
                record[2] = error.error_len().unwrap_or(0).try_into().unwrap();
            }
        }
        record[4..4 + bytes.len()].copy_from_slice(bytes);
        corpus.extend_from_slice(&record);
        count += 1;
    };
    add(b"");
    for first in 0..=255 {
        add(&[first]);
        for second in 0..=255 {
            add(&[first, second]);
        }
    }
    for text in ["hello\0", "é", "€", "𐀀", "\u{10ffff}", "é€"] {
        add(text.as_bytes());
    }
    for first in [0xe0, 0xed, 0xef, 0xf0, 0xf4] {
        for second in 0..=255 {
            add(&[first, second, 0x80]);
            add(&[first, second, 0x80, 0x80]);
        }
    }
    for bytes in [
        &[0xe2, 0x82, b'x'][..],
        &[0xf0, 0x90, 0x80, b'x'],
        &[b'a', 0xe2, 0x82],
        &[b'a', 0xc0, 0xaf],
    ] {
        add(bytes);
    }
    let mut random = 0x591c_7a83_290d_e4b1_u64;
    for length in 0..=8 {
        for _ in 0..512 {
            let mut bytes = [0; 8];
            for byte in &mut bytes[..length] {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                *byte = random as u8;
            }
            add(&bytes[..length]);
        }
    }
    let source =
        include_str!("../../../crates/test-fixtures/fixtures/utf8_differential_records.gin")
            .replace("{last}", &(count - 1).to_string());
    let (package, input) = package("utf8_differential", &source);
    for profile in [Profile::Debug, Profile::Release] {
        let output = compile(&package, &input, profile);
        let mut child = Command::new(output).stdin(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(&corpus).unwrap();
        assert!(child.wait().unwrap().success(), "{profile:?}");
    }
}

#[test]
fn strings_preserve_utf8_across_construction_copy_append_and_byte_recovery() {
    let (package, input) = package(
        "utf8_strings",
        include_str!("../../../crates/test-fixtures/fixtures/utf8_string_lifecycle.gin"),
    );
    for profile in [Profile::Debug, Profile::Release] {
        let executable = compile(&package, &input, profile);
        assert!(Command::new(executable).status().unwrap().success());
    }
}

#[test]
fn owned_utf8_conversion_preserves_storage_and_allocator_on_both_paths() {
    let (package, input) = package(
        "utf8_allocator",
        include_str!("../../../crates/test-fixtures/fixtures/utf8_allocator_paths.gin"),
    );
    for profile in [Profile::Debug, Profile::Release] {
        let executable = compile(&package, &input, profile);
        assert!(Command::new(executable).status().unwrap().success());
    }
}

#[test]
fn byte_array_print_batches_newline_and_preserves_partial_writes() {
    let source = r#"use core.io print
use core.slice Byte
#entry
main:
    print('')
    print('x')
    print('Hello, world!')
    print('hé🙂')
    print('a\0b')
    print('line\n')
    print('\n')
    bytes: 'ab'
    bytes.[1]:: Byte.replace(bytes.[1], 122 as Byte)
    print(bytes)
    invalid [in Byte; 1]: [255]
    print(invalid)
    return
"#;
    let expected = b"\nx\nHello, world!\nh\xc3\xa9\xf0\x9f\x99\x82\na\0b\nline\n\n\n\naz\n\xff\n";
    for (limit, writes) in [
        ("size", 9),
        (
            "when (MemorySize.as_num(size) as MemorySize) > 2 then 2 else size",
            22,
        ),
    ] {
        let (package, input) = package("batched_byte_array_print", source);
        package.write(
            "core/io/platform/platform.gin",
            &format!(
                r#"use core.alloc.system BytePointer, MemorySize, SystemResult
use core.target.kernel read_number, receive, send, write_number

read(fd SystemResult, pointer BytePointer, size MemorySize) SystemResult:
    return receive(read_number, fd, pointer, size)

write(fd SystemResult, pointer BytePointer, size MemorySize) SystemResult:
    marker := 'W'
    send(write_number, 2, (@marker) as BytePointer, 1)
    return send(write_number, fd, pointer, {limit})
"#
            ),
        );
        for profile in [Profile::Debug, Profile::Release] {
            let executable = compile(&package, &input, profile);
            let output = Command::new(executable).output().unwrap();
            assert!(output.status.success(), "{limit}/{profile:?}: {output:?}");
            assert_eq!(output.stdout, expected, "{limit}/{profile:?}");
            assert_eq!(output.stderr, vec![b'W'; writes], "{limit}/{profile:?}");
        }
    }
}

#[test]
fn byte_array_print_retries_interruptions_and_stops_on_zero_or_error() {
    let source = r#"use core.io print, write_all
use core.slice Byte
#entry
main:
    empty := ''
    pointer: (@empty) as @Byte
    write_all(pointer: pointer, size: 0)
    print('Hello, world!')
    print('done')
    return
"#;
    let (package, input) = package("byte_array_print_results", source);
    package.write("core/io/platform/platform.gin", SCRIPTED_WRITE_PLATFORM);
    let mut interrupted = vec![3; 12];
    interrupted[0] = 1;
    interrupted[2] = 1;
    let cases = [
        (
            interrupted.as_slice(),
            b"Hello, world!\ndone\n".as_slice(),
            12,
        ),
        ([0, 3, 3, 3].as_slice(), b"done\n".as_slice(), 4),
        ([2, 3, 3, 3].as_slice(), b"done\n".as_slice(), 4),
        ([3, 2, 3, 3, 3].as_slice(), b"Hedone\n".as_slice(), 5),
    ];
    for profile in [Profile::Debug, Profile::Release] {
        let executable = compile(&package, &input, profile);
        for (commands, expected, writes) in cases {
            let mut child = Command::new(&executable)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(commands).unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{profile:?}/{commands:?}: {output:?}"
            );
            assert_eq!(output.stdout, expected, "{profile:?}/{commands:?}");
            assert_eq!(
                output.stderr,
                vec![b'W'; writes],
                "{profile:?}/{commands:?}"
            );
        }
    }
}

const SCRIPTED_WRITE_PLATFORM: &str = r#"use core.alloc.system BytePointer, MemorySize, SystemResult
use core.slice Byte
use core.target.kernel read_number, receive, send, write_number

read(fd SystemResult, pointer BytePointer, size MemorySize) SystemResult:
    return receive(read_number, fd, pointer, size)

write(fd SystemResult, pointer BytePointer, size MemorySize) SystemResult:
    marker := 'W'
    send(write_number, 2, (@marker) as BytePointer, 1)
    control [in Byte; 1]: [0]
    receive(read_number, 0, (@control) as BytePointer, 1)
    when control.[0] = 0 then
        return 0
    when control.[0] = 1 then
        return -4
    when control.[0] = 2 then
        return -5
    limit MemorySize: when (MemorySize.as_num(size) as MemorySize) > 2 then 2 else size
    return send(write_number, fd, pointer, limit)
"#;

#[test]
fn migrated_examples_preserve_output_and_allocator_cleanup() {
    for (name, expected) in [
        ("machine_string", "Hello, Gin!\n"),
        (
            "vector_runtime",
            "513\n1025\none\nOne\nthree\n-9223372036854775808\n10\n",
        ),
        ("allocator_string_runtime", ""),
    ] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../modules")
            .join(name)
            .join("main.gin");
        let source = std::fs::read_to_string(path).unwrap();
        let (package, input) = package(name, &source);
        for profile in [Profile::Debug, Profile::Release] {
            let executable = compile(&package, &input, profile);
            let output = Command::new(executable).output().unwrap();
            assert!(output.status.success(), "{name}/{profile:?}: {output:?}");
            assert_eq!(output.stdout, expected.as_bytes(), "{name}/{profile:?}");
        }
    }
}

#[test]
fn external_code_cannot_construct_destructure_or_mutate_string_storage() {
    for expression in [
        "make(bytes List(Byte)) String: return String(bytes)",
        "unpack(text String) List(Byte):\n    String(bytes) := text\n    return bytes",
        "mutate(text mut String): return text.bytes.push(255 as Byte)",
        "escape(text ref String) SliceView(Byte): return text.as_bytes.view",
    ] {
        let source = format!(
            "use core.string String\nuse core.collection List\nuse core.slice Byte, SliceView\n{expression}\n"
        );
        let (_package, input) = package("utf8_privacy", &source);
        let mut driver = CompilerDriver::default();
        let parsed = driver
            .parse(SourcePackage::new(
                &input,
                vec![SourceFile::new(&input, source)],
            ))
            .unwrap();
        let target = flask::CompileTarget::Concrete(
            flask::TargetTriple::parse(&host_target_triple()).unwrap(),
        );
        let checked =
            driver.check(CompilePackage::new(parsed, target).with_import_resolution(true));
        let flaws: Vec<_> = checked
            .files()
            .iter()
            .flat_map(|file| file.typecheck_diagnostics())
            .filter(|diagnostic| diagnostic.category == diagnostic::Category::Flaw)
            .collect();
        assert!(
            flaws
                .iter()
                .any(|flaw| flaw.code.slug() == "type-private-member"),
            "{expression}: {flaws:?}"
        );
    }
}

#[test]
fn nested_list_cleanup_preserves_capacity_across_raw_resource_releases() {
    let source = r#"use core.collection List
use core.slice Byte
Marker has
    value Byte
    release(self): return
Resource has
    marker @Byte
    value Byte
    release(self):
        Resource(marker, value) := self
        marker.[0] :: Byte.replace(marker.[0], value)
        return
fill(items own List(Resource) and items.capacity >= 2, marker ref Marker):
    for outer in 0 to 2
        items.clear
        while items.length < 2 do
            items.push_reserved(Resource(marker: @marker.value, value: 7 as Byte))
    items.release
    return
#entry
main:
    marker: Marker(value: 0 as Byte)
    attempt: List.with_capacity(Resource, 2)
    when attempt is
        List(items) then fill(items, marker)
        AllocFailure(failure) then
            failure.release
            marker.release
            return 2
    result is in Byte: marker.value
    marker.release
    return when result = 7 then 0 else 1
"#;
    let (package, input) = package("nested_raw_cleanup", source);
    for profile in [Profile::Debug, Profile::Release] {
        let output = Command::new(compile(&package, &input, profile))
            .output()
            .unwrap();
        assert!(output.status.success(), "{profile:?}: {output:?}");
    }
}
