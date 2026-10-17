use ginc::cli::Args;
use ginc::compile::GinCompiler;
use std::process::Command;
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn imported_overloads_select_types_and_execute_in_either_declaration_order() {
    for reverse in [false, true] {
        let package = TempPackage::new("function_overloads");
        package.write_flask("overload_consumer");
        let mut overloads = [
            "measure(value own Buffer) Buffer: return value\n",
            "measure(bytes [in Byte; count]) Length: return count as Length\n",
        ];
        if reverse {
            overloads.reverse();
        }
        package.write("arrays/arrays.gin", &format!(
            "{}Length is in 0 to 18446744073709551615\nLength has\n    finish(self):\n        return\nBuffer has\n    finish(self):\n        return\n    length Length\n{}{}forward(bytes [in Byte; count]) Length: return measure(bytes)\nmix(left [in Byte; count], right Length) Length: return right\nmix(left Length, right [in Byte; count]) Length: return left\n", test_fixtures::preludes::BYTE_PRELUDE, overloads[0], overloads[1]
        ));
        let input = package.write("main.gin", r#"use 'arrays' measure, forward, mix, Buffer
use 'arrays' as arrays
#entry
main:
    empty is in 0 to 18446744073709551615: measure('')
    short is in 0 to 18446744073709551615: measure('abc')
    unicode is in 0 to 18446744073709551615: forward('é🦀')
    qualified is in 0 to 18446744073709551615: arrays.measure('12345')
    named is in 0 to 18446744073709551615: mix(right: 9, left: 'abc')
    buffer: measure(Buffer(length: 7))
    length is in 0 to 18446744073709551615: buffer.length
    buffer.finish
    return when empty = 0 and short = 3 and unicode = 6 and length = 7 and qualified = 5 and named = 9 then 0 else 1
"#);
        let executable = package.join("program");
        let mut args = Args {
            input,
            output: Some(executable.clone()),
            target: Some(host_target_triple()),
            no_cache: true,
            gin_home: Some(flask::GinHome::new(package.join("home"))),
            ..Default::default()
        };
        let result = GinCompiler::compile(&mut args);
        assert!(result.is_success(), "{result:?}");
        assert!(!result.has_typecheck_diagnostics(), "{result:?}");
        let output = Command::new(executable).output().unwrap();
        assert!(output.status.success(), "{output:?}");
    }
}

#[test]
fn equally_sized_array_specializations_emit_distinct_functions() {
    let package = TempPackage::new("specialized_overload_identity");
    package.write_flask("identity_consumer");
    let input = package.write(
        "main.gin",
        &format!("{}Word is in 0 to 65535\nWord has\n    finish(self):\n        return\nchoose(bytes [in Byte; count], value Byte) Byte: return 11\nchoose(bytes [in Byte; count], value Word) Byte: return 22\nFactory has\n    pick(value Byte) Byte: return 33\n    pick(value Word) Byte: return 44\n    pick(left Byte, right Byte) Byte: return 55\n#entry\nmain:\n    first is in Byte: choose('abc', 1 as Byte)\n    second is in Byte: choose('xyz', 1 as Word)\n    narrow is in Byte: Factory.pick(1 as Byte)\n    wide is in Byte: Factory.pick(1 as Word)\n    pair is in Byte: Factory.pick(1 as Byte, 2 as Byte)\n    return when first = 11 and second = 22 and narrow = 33 and wide = 44 and pair = 55 then 0 else 1\n", test_fixtures::preludes::BYTE_PRELUDE),
    );
    let executable = package.join("program");
    let mut args = Args {
        input,
        output: Some(executable.clone()),
        target: Some(host_target_triple()),
        no_cache: true,
        gin_home: Some(flask::GinHome::new(package.join("home"))),
        ..Default::default()
    };
    let result = GinCompiler::compile(&mut args);
    assert!(result.is_success(), "{result:?}");
    assert!(!result.has_typecheck_diagnostics(), "{result:?}");
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success(), "{output:?}");
}
