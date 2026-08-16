//! Audit binder conventions for `:=` (constant) vs `:` (rebindable), comptime vs runtime.
//!
//! Sources are inlined to avoid depending on external `gin_core` files that are
//! still being worked on. Each inline snippet reproduces just enough of the
//! original to exercise the convention being audited.

use flask::CompileTarget;
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::prepare_parse_ast;

fn prepare_and_check(source: &str, name: &str) -> (ast::FileAst, ast::Bind) {
    let mut ast = TokenCursor::parse_source(source);
    let _ = prepare_parse_ast(&mut ast, &CompileTarget::Library);
    let bind = ast
        .defs
        .get(&Intern::from_ref(name))
        .cloned()
        .unwrap_or_else(|| panic!("missing def `{name}`"));
    (ast, bind)
}

#[test]
fn marker_copy_is_constant_comptime_fn() {
    // Minimal: just needs a Type tag + Bool so the binder convention is clear.
    let source = "\
Type is Primitive(width BigInt, signed Bool) or Ptr(inner Type)
Bool is True or False
BigInt is in 0...18446744073709551615

is_copy(x Type) Bool := when x is
    Primitive(_, _)     then True
    Ptr(_)              then False
";
    let (ast, bind) = prepare_and_check(source, "is_copy");
    assert!(bind.is_constant(), "is_copy uses `:=`");
    assert!(ast.defs.contains_key(&Intern::from_ref("is_copy")));
}

#[test]
fn marker_sized_helpers_are_constant_comptime_fns() {
    let source = "\
Type is Primitive(width BigInt, signed Bool)
Bool is True or False
BigInt is in 0...18446744073709551615
Size is Const(BigInt) or Dynamic

compute_size(x Type) Size := when x is
    Primitive(w, _)     then Const(w / 8)
";
    let (ast, bind) = prepare_and_check(source, "compute_size");
    assert!(bind.is_constant(), "compute_size uses `:=`");
    assert!(ast.defs.contains_key(&Intern::from_ref("compute_size")));
}

#[test]
fn target_is_constant_foldable_value() {
    let source = "\
Target has arch Str, vendor Str, os Str
target := Target('x86_64', 'unknown', 'unknown')
";
    let (ast, bind) = prepare_and_check(source, "target");
    assert!(bind.is_constant());
    assert!(ast.defs.contains_key(&Intern::from_ref("target")));
}

#[test]
fn bool_sentinels_are_constant_values() {
    let source = "\
Bool is True or False
false := Bool.False
true  := Bool.True
";
    let mut ast = TokenCursor::parse_source(source);
    let _ = prepare_parse_ast(&mut ast, &CompileTarget::Library);
    for name in ["true", "false"] {
        let bind = ast
            .defs
            .get(&Intern::from_ref(name))
            .unwrap_or_else(|| panic!("missing def `{name}`"));
        assert!(bind.is_constant(), "{name} uses `:=`");
    }
}

#[test]
fn io_write_runtime_write_spec_foldable() {
    let source = "\
Int is in 0...4294967295

write_spec := 4

write(fd Int, buf Pointer(Int), len Int) Int:
    result := write_spec + fd + buf + len
    return result
";
    let mut ast = TokenCursor::parse_source(source);
    let _ = prepare_parse_ast(&mut ast, &CompileTarget::Library);
    assert!(
        ast.defs
            .get(&Intern::from_ref("write_spec"))
            .unwrap()
            .is_constant()
    );
    assert!(
        !ast.defs
            .get(&Intern::from_ref("write"))
            .unwrap()
            .is_constant()
    );
}
