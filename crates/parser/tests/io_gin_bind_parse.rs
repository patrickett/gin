//! Inline `io.gin`-like source: constant `write_spec :=` vs rebindable runtime `write …:`.

use parser::query::SourceParseExt;
use typecheck::ComptimeClass;
use typecheck::comptime_classify::{BindComptimeExt, FileAstComptimeExt};

const IO_SRC: &str = "\
Int is in 0...4294967295

write_spec := 'svc #0x80'

write(fd Int, buf Pointer(Int), len Int) Int:
    result := asm(write_spec, fd, buf, len)
    return result

print(s String):
    write(1, s.pointer, s.len)
    return

println(s String):
    newline := '\\n'
    print(s)
    print(newline)
    return

eprint(s String):
    write(2, s.pointer, s.len)
    return

eprintln(s String):
    eprint(s)
    eprint('\\n')
    return
";

fn parse_io_src() -> ast::FileAst {
    IO_SRC.parse_source_full().ast
}

#[test]
fn write_spec_is_constant_foldable_value() {
    let mut ast = parse_io_src();
    let spec = ast
        .defs
        .get(&internment::Intern::new("write_spec".to_string()))
        .expect("write_spec");
    assert!(spec.is_constant, "write_spec should use `:=`");
    ast.apply_comptime_classification();
    let spec = ast
        .defs
        .get(&internment::Intern::new("write_spec".to_string()))
        .unwrap();
    assert!(
        spec.is_compile_time,
        "AsmBuilder chain should be comptime-foldable"
    );
    assert_eq!(
        spec.classify(&ast),
        ComptimeClass::FoldableValue,
        "write_spec is a foldable top-level value, not a runtime fn"
    );
}

#[test]
fn write_is_rebindable_runtime_fn() {
    let mut ast = parse_io_src();
    let write = ast
        .defs
        .get(&internment::Intern::new("write".to_string()))
        .expect("write");
    assert!(!write.is_constant, "write should use `:` not `:=`");
    ast.apply_comptime_classification();
    let write = ast
        .defs
        .get(&internment::Intern::new("write".to_string()))
        .unwrap();
    assert!(
        !write.is_compile_time,
        "syscall/asm body must stay runtime-classified"
    );
    assert_eq!(write.classify(&ast), ComptimeClass::RuntimeFn);
}

#[test]
fn print_helpers_are_rebindable_runtime() {
    let mut ast = parse_io_src();
    for name in ["print", "println", "eprint", "eprintln"] {
        let bind = ast
            .defs
            .get(&internment::Intern::new(name.to_string()))
            .expect(name);
        assert!(!bind.is_constant, "{name} should use `:`");
        ast.apply_comptime_classification();
        let bind = ast
            .defs
            .get(&internment::Intern::new(name.to_string()))
            .unwrap();
        assert!(!bind.is_compile_time, "{name} should be runtime");
    }
}
