//! Cross-file forward references must not produce `UnknownSymbol` regardless of file order.

mod support;

use support::transform_source;

use parser::cursor::TokenCursor;
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform_package};

fn unknown_symbol_names(typed: &typecheck::TypedFileAst) -> Vec<String> {
    typed
        .all_flaws()
        .into_iter()
        .filter(|&(_, flaw)| flaw.code.slug() == "type-unknown-symbol")
        .map(|(_, flaw)| flaw.arg("name").unwrap_or("").to_string())
        .collect()
}

fn transform_two_files(
    caller_src: &str,
    callee_src: &str,
) -> (typecheck::TypedFileAst, typecheck::TypedFileAst) {
    let file_asts = [
        (TokenCursor::parse_source(caller_src), FileId(0)),
        (TokenCursor::parse_source(callee_src), FileId(1)),
    ];
    let ctx = TransformCtx::new();
    let mut results = transform_package(
        &file_asts,
        &ctx,
        typecheck::transform::PackageTransformOptions::FULL,
    );
    assert_eq!(results.len(), 2);
    let callee = results.pop().unwrap();
    let caller = results.pop().unwrap();
    (caller, callee)
}

#[test]
fn caller_before_callee_no_unknown_foo() {
    let caller = "bar() Int := foo() + 1\n";
    let callee = "foo() Int := 42\n";
    let (caller_typed, _) = transform_two_files(caller, callee);
    let unknown = unknown_symbol_names(&caller_typed);
    assert!(
        !unknown.iter().any(|n| n == "foo"),
        "expected `foo` to resolve when callee is second file, got: {unknown:?}"
    );
}

#[test]
fn callee_before_caller_no_unknown_foo() {
    let caller = "bar() Int := foo() + 1\n";
    let callee = "foo() Int := 42\n";
    let file_asts = [
        (TokenCursor::parse_source(callee), FileId(0)),
        (TokenCursor::parse_source(caller), FileId(1)),
    ];
    let results = transform_package(
        &file_asts,
        &TransformCtx::new(),
        typecheck::transform::PackageTransformOptions::FULL,
    );
    let caller_typed = &results[1];
    let unknown = unknown_symbol_names(caller_typed);
    assert!(
        !unknown.iter().any(|n| n == "foo"),
        "expected `foo` to resolve when callee is first file, got: {unknown:?}"
    );
}

#[test]
fn same_file_forward_ref_union_size_before_definition() {
    let source = r#"
Size is Const(BigInt) or Dynamic

compute_size(x Type) Size := when x is
    Union(_, variants) then union_size(variants)
    _                    then Dynamic

union_size(variants List(Int)) Size := Const(0)
"#;
    let typed = transform_source(source);
    let unknown: Vec<_> = typed
        .all_flaws()
        .into_iter()
        .filter(|&(_, f)| f.code.slug() == "type-unknown-symbol")
        .map(|(_, f)| f.arg("name").unwrap_or("").to_string())
        .collect();
    assert!(
        !unknown.iter().any(|n| n.as_str() == "union_size"),
        "union_size should resolve regardless of definition order, got: {unknown:?}"
    );
}

#[test]
fn inline_forward_refs_not_unknown() {
    use flask::CompileTarget;
    use typecheck::FileId;
    use typecheck::prepare_file_ast;
    use typecheck::transform::{TransformCtx, transform};

    let source = "\
Size is Const(BigInt) or Dynamic
BigInt is in 0...18446744073709551615

Sized has (size Size)
x.Sized(size: compute_size(x))

compute_size(x Type) Size := when x is
    Primitive(w, _)     then Const(w / 8)
    Record(_, fields)   then sum_named(fields)
    Union(_, variants)  then union_size(variants)

Type is Primitive(width BigInt, signed Bool)
     or Record(name String, fields List(NamedTy))
     or Union(name String, variants List(VariantShape))
     or Opaque(name String)

NamedTy has (name String, ty Type)
VariantShape has (name String, fields List(NamedTy))
Bool is True or False
List(x) has (pointer Pointer(x), length BigInt)
String has (bytes List(BigInt))

sum_named(fields List(NamedTy)) Size := when fields is
    []                  then Const(0)
    [f, ...rest]        then add(compute_size(f.ty), sum_named(rest))

add(a Size, b Size) Size := when (a, b) is
    (Const(x), Const(y)) then Const(x + y)
                         else Dynamic

union_size(variants List(VariantShape)) Size := add(union_disc(variants), union_max_payload(variants))

union_disc(variants List(VariantShape)) Size := when variants is
    []           then Const(0)
    [_]          then Const(1)
                 else Const(2)

union_max_payload(variants List(VariantShape)) Size := when variants is
    []              then Const(0)
    [v, ...rest]    then max_size(sum_named(v.fields), union_max_payload(rest))

max_size(a Size, b Size) Size := when (a, b) is
    (Const(x), Const(y)) then Const(when x > y then x else y)
                         else Dynamic
";
    let mut file_ast = TokenCursor::parse_source(source);
    let _ = prepare_file_ast(&mut file_ast, &CompileTarget::Library);
    let typed = transform(&file_ast, FileId(0), &TransformCtx::new());
    let unknown = unknown_symbol_names(&typed);
    for name in ["union_size", "mul_size", "max_size"] {
        assert!(
            !unknown.iter().any(|n| n == name),
            "`{name}` should resolve regardless of definition order, got: {unknown:?}"
        );
    }
}
