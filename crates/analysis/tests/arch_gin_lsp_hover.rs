//! LSP-style hover for an `Architecture` const union (canonical gin_core arch fixture).

use analysis::{CacheEngine, QueryEngine};
use crossbeam_channel::unbounded;
use test_fixtures::TempPackage;

const SIZED_GIN: &str = r#"use '../reflect/'.(Type, NamedTy, VariantShape)
use '../primitive/'.(BigInt, List)

--- Compile-time byte size of a type.
Size is Const(BigInt) or Dynamic

Sized has (size Size)

-- generic, if Sized is in scope it applies to all types
x.Sized(size: compute_size(x))

compute_size(x Type) Size := when x is
    Primitive(w, _)     then Const(w / 8)
    Ptr(_)              then Const(8)
    Ref(_, _)           then Const(8)
    Opaque(_)           then Dynamic
    Record(_, fields)   then sum_named(fields)
    Tuple(elems)        then sum_types(elems)
    Union(_, variants)  then union_size(variants)
    Array(elem, n)      then mul_size(compute_size(elem), n)

sum_named(fields List(NamedTy)) Size := when fields is
    []                  then Const(0)
    [f, ...rest]        then add(compute_size(f.ty), sum_named(rest))

sum_types(elems List(Type)) Size := when elems is
    []                  then Const(0)
    [t, ...rest]        then add(compute_size(t), sum_types(rest))

add(a Size, b Size) Size := when (a, b) is
    (Const(x), Const(y)) then Const(x + y)
                         else Dynamic

mul_size(size Size, n BigInt) Size := when size is
    Const(x) then Const(x * n)
             else Dynamic

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

union_size(variants List(VariantShape)) Size := add(union_disc(variants), union_max_payload(variants))
"#;

const ARCH_GIN: &str = r#"--- Target CPU architectures for conditional compilation.
Architecture is 'x86_64'
             or 'arm64'
             or 'wasm32'
"#;

const BOOL_GIN: &str = r#"use core.Happy
use core.ToString

--- `Bool` represents a value, which could only be either `True` or `False`.
---
--- ## Basic usage
---
--- `Bool` implements various traits, such as BitAnd, BitOr, Not, etc.,
--- which allow us to perform boolean operations using &, | and !.
---
--- `if` requires a `Bool` value as its conditional.
Bool is True or False
Bool.Happy(value: Bool.True)
Bool.ToString(to_string: when self then 'true' else 'false')


false := Bool.False
true  := Bool.True


-- is_empty(v Maybe(x)) Bool:
--     if v is None return True
-- return False
"#;

const COPY_GIN: &str = r#"use core.reflect.(Type, NamedTy, VariantShape)
use core.primitive.(Bool, List)

--- Types that can be implicitly copied.
--- Opt out with `Type.Copy(can_copy: False)`.
Copy has (can_copy Bool)

-- Blanket: copyability follows structural rules on reflected shape.
x.Copy(can_copy: is_copy(x))

is_copy(x Type) Bool := when x is
    Primitive(_, _)     then True
    Ptr(_)              then False
    Ref(_, False)       then True
    Ref(_, True)        then False
    Opaque(_)           then False
    Record(_, fields)   then all_named_copy(fields)
    Tuple(elems)        then all_types_copy(elems)
    Union(_, variants)  then all_variants_copy(variants)
    Array(elem, _)      then is_copy(elem)

all_named_copy(fields List(NamedTy)) Bool := when fields is
    []                  then True
    [f, ...rest]        then is_copy(f.ty) and all_named_copy(rest)

all_types_copy(elems List(Type)) Bool := when elems is
    []                  then True
    [t, ...rest]        then is_copy(t) and all_types_copy(rest)

all_variants_copy(variants List(VariantShape)) Bool := when variants is
    []                  then True
    [v, ...rest]        then all_named_copy(v.fields) and all_variants_copy(rest)
"#;

const TYPE_GIN: &str = r#"use '../primitive/'.(BigInt, Bool, List)
use '../string/'.String

--- Structural description of a Gin type, used for compile-time reflection.
Type is Primitive(width BigInt, signed Bool)
     or Record(name String, fields List(NamedTy))
     or Union(name String, variants List(VariantShape))
     or Tuple(elems List(Type))
     or Ptr(inner Type)
     or Ref(inner Type, mutable Bool)
     or Array(elem Type, size BigInt)
     or Opaque(name String)

NamedTy has (name String, ty Type)
VariantShape has (name String, fields List(NamedTy))

--- Reserved: the compiler synthesizes `shape` for every type.
--- User-written `Type.Reflectable(...)` is a compile error.
Reflectable has (shape Type)
"#;

#[test]
fn marker_fixture_imports_sized_trait() {
    let sized_source = SIZED_GIN;
    let file = parser::cursor::TokenCursor::parse_source(sized_source);
    let traits = file.imported_trait_names();
    assert!(
        traits.iter().any(|t| t.as_str() == "Sized"),
        "imports: {:?}",
        traits.iter().map(|t| t.as_str()).collect::<Vec<_>>()
    );
}

fn register_arch_package(
    engine: &mut CacheEngine,
    pkg: &TempPackage,
    arch_path: std::path::PathBuf,
    arch_source: &str,
) {
    for (content, rel) in [
        (TYPE_GIN, "reflect/type.gin"),
        (BOOL_GIN, "primitive/bool.gin"),
        (SIZED_GIN, "marker/sized.gin"),
        (COPY_GIN, "marker/copy.gin"),
    ] {
        let p = pkg.write(rel, content);
        engine.add_file(p).unwrap();
    }
    engine.add_file(arch_path.clone()).unwrap();
    engine.set_contents(&arch_path, arch_source.to_string());
}

#[test]
fn lsp_hover_on_architecture_in_package_fixture() {
    let arch_source = ARCH_GIN;
    let pkg = TempPackage::new("architecture");
    pkg.write_flask("core");
    let path = pkg.write("target/arch/arch.gin", arch_source);
    let byte = arch_source
        .find("Architecture is")
        .expect("Architecture declaration") as u32;

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    register_arch_package(&mut engine, &pkg, path.clone(), arch_source);

    let hover = engine
        .hover(&path, byte)
        .expect("hover on Architecture in package fixture")
        .markdown;

    assert_eq!(
        hover,
        "```gin\ncore.target.arch\n```\n\n```gin\nArchitecture is 'x86_64'\n             or 'arm64'\n             or 'wasm32'\n```\n---\n\nTarget CPU architectures for conditional compilation."
    );
}

#[test]
fn lsp_hover_on_arch_bind_in_package_fixture() {
    let mut arch_source = String::from(ARCH_GIN);
    arch_source.push_str("\narch Architecture\n");
    let pkg = TempPackage::new("arch_bind");
    pkg.write_flask("core");
    let path = pkg.write("target/arch/arch.gin", &arch_source);
    let byte = arch_source
        .find("arch Architecture")
        .expect("`arch Architecture` in fixture") as u32;

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    register_arch_package(&mut engine, &pkg, path.clone(), &arch_source);

    let hover = engine
        .hover(&path, byte)
        .expect("hover on `arch` in package fixture")
        .markdown;

    assert_eq!(
        hover,
        "```gin\ncore.target.arch\n```\n\n```gin\narch Architecture\n```"
    );
}
