//! Hover on `compute_size` in sized.gin must not show unrelated `Range` tag.

use analysis::{CacheEngine, QueryEngine};
use crossbeam_channel::unbounded;
use std::collections::HashMap;
use test_fixtures::TempPackage;

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

NamedTy has name String, ty Type
VariantShape has name String, fields List(NamedTy)

--- Reserved: the compiler synthesizes `shape` for every type.
--- User-written `Type.Reflectable has ...` is a compile error.
Reflectable has shape Type
"#;

const INT_GIN: &str = r#"--- The 8-bit signed integer type.
SignedTinyInt is in -128...127
--- The 16-bit signed integer type.
SignedSmallInt is in -32768...32767
--- The 32-bit signed integer type.
SignedInt is in -2147483648...2147483647
--- The 64-bit signed integer type.
SignedBigInt is in -9223372036854775808...9223372036854775807
--- The 128-bit signed integer type.
SignedLargeInt is in -170141183460469231731687303715884105728...170141183460469231731687303715884105727
--- The 8-bit unsigned integer type.
TinyInt is in 0...255
--- The 16-bit unsigned integer type.
SmallInt is in 0...65535
--- The 32-bit unsigned integer type.
Int is in 0...4294967295
--- The 64-bit unsigned integer type.
BigInt is in 0...18446744073709551615
--- The 128-bit unsigned integer type.
LargeInt is in 0...340282366920938463463374607431768211455


--- Alias for the 8-bit unsigned integer type.
Byte is TinyInt
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
Bool.Happy has value: Bool.True
Bool.ToString has to_string: when self then 'true' else 'false'


false := Bool.False
true  := Bool.True


-- is_empty(v Maybe(x)) Bool:
--     if v is None return True
-- return False
"#;

const LIST_GIN: &str = r#"use Pointer, PointerSize

--- A homogeneous list type.
--- List literals use bracket syntax: ['linux', 'macos'].
--- Generic over element type: `List(x)`.
List(x) has pointer Pointer(x), length PointerSize
"#;

const RANGE_GIN: &str = r#"use '../'.Bounded

--- Range utilities.
---
--- `Range(x)` is the value type constructed by a range literal like `start...end`.
---
--- ## Examples
---
--- ```gin
--- r := 12...1200
--- ```
---
--- `start` and `end` are the bounds of the range.
Range(x) has start x, end x
Range.Bounded has min: start, max: end

--- create a new range
Range(x).new(start x, end x) Range(x): (start, end)
"#;

const POINTER_GIN: &str = r#"use '../target/'.target
use BigInt, Int

Pointer(x) is @x

--- The pointer-width unsigned integer type (8 bytes on 64-bit, 4 bytes on 32-bit).
PointerSize is when target.arch is
    'x86_64' or 'arm64' then BigInt
    'wasm32'            then Int
"#;

const STRING_GIN: &str = r#"use core.primitive.(Byte, List)

--- Canonical `String` type used for printing/codegen.
String has bytes List(Byte)

ToString has to_string String
"#;

const SIZED_GIN: &str = r#"use '../reflect/'.(Type, NamedTy, VariantShape)
use '../primitive/'.(BigInt, List)

--- Compile-time byte size of a type.
Size is Const(BigInt) or Dynamic

#auto
Sized has size Size: compute_size(Self)

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

const COPY_GIN: &str = r#"use core.reflect.(Type, NamedTy, VariantShape)
use core.primitive.(Bool, List)

--- Types that can be implicitly copied.
--- Opt out with `Copy.can_copy: False`.
#auto
Copy has can_copy Bool: is_copy(Self)

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

const MARKER_PACKAGE: &[(&str, &str)] = &[
    (TYPE_GIN, "reflect/type.gin"),
    (INT_GIN, "primitive/int.gin"),
    (BOOL_GIN, "primitive/bool.gin"),
    (LIST_GIN, "primitive/list.gin"),
    (RANGE_GIN, "primitive/range.gin"),
    (POINTER_GIN, "primitive/pointer.gin"),
    (STRING_GIN, "string/string.gin"),
    (SIZED_GIN, "marker/sized.gin"),
    (COPY_GIN, "marker/copy.gin"),
];

fn write_marker_package(pkg: &TempPackage) {
    for (content, rel) in MARKER_PACKAGE {
        pkg.write(rel, content);
    }
}

#[test]
fn package_diagnostics_copy_gin_is_clean() {
    let pkg = TempPackage::new("copy_marker_diagnostics");
    pkg.write_flask("core");
    write_marker_package(&pkg);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    let rels: Vec<&str> = MARKER_PACKAGE.iter().map(|(_, rel)| *rel).collect();
    let paths: Vec<_> = rels.iter().map(|rel| pkg.root.join(rel)).collect();
    for path in &paths {
        engine.add_file(path.clone()).unwrap();
    }

    let copy_path = pkg.root.join("marker/copy.gin");
    let diagnostics = engine.all_diagnostics(
        &paths,
        &HashMap::from([("core".to_string(), pkg.root.clone())]),
    );
    let copy_diagnostics = diagnostics.get(&copy_path).cloned().unwrap_or_default();
    let semantic_diagnostics: Vec<_> = copy_diagnostics
        .iter()
        .filter(|diagnostic| !diagnostic.code.slug().starts_with("use-"))
        .collect();

    assert!(
        semantic_diagnostics.is_empty(),
        "copy.gin should have no parser/type diagnostics through analysis/LSP path: {semantic_diagnostics:?}"
    );
}

#[test]
fn package_hover_compute_size_name_not_range_tag() {
    let sized_src = SIZED_GIN;
    let pkg = TempPackage::new("sized_compute_hover");
    pkg.write_flask("core");
    write_marker_package(&pkg);
    let sized_path = pkg.root.join("marker/sized.gin");

    let bind_line = "compute_size(x Type) Size := when x is";
    let byte = sized_src.find(bind_line).expect("compute_size bind") as u32
        + bind_line.find("compute_size").expect("name") as u32;

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    for (_, rel) in MARKER_PACKAGE {
        engine.add_file(pkg.root.join(rel)).unwrap();
    }

    let hover = engine
        .hover(&sized_path, byte)
        .expect("hover on compute_size")
        .markdown;

    assert_eq!(
        hover, "```gin\ncore.marker\n```\n\n```gin\ncompute_size(x Type) Size\n```",
        "expected compute_size signature"
    );
}

#[test]
fn package_hover_compute_size_wins_over_phantom_range_declare_span() {
    let sized_src = SIZED_GIN;
    let range_src = RANGE_GIN;
    let pkg = TempPackage::new("sized_phantom_range");
    pkg.write_flask("core");
    let sized_path = pkg.write("marker/sized.gin", sized_src);
    let range_path = pkg.write("primitive/range.gin", range_src);

    let bind_line = "compute_size(x Type) Size := when x is";
    let byte = sized_src.find(bind_line).expect("compute_size bind") as u32
        + bind_line.find("compute_size").expect("name") as u32;

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(sized_path.clone()).unwrap();
    engine.add_file(range_path).unwrap();

    let hover = engine
        .hover(&sized_path, byte)
        .expect("hover on compute_size")
        .markdown;

    assert_eq!(
        hover, "```gin\ncore.marker\n```\n\n```gin\ncompute_size(x Type) Size\n```",
        "def name must win over any declare-span overlap"
    );
}
