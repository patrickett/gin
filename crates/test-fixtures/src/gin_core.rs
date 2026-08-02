//! Canonical `modules/gin_core` source fixtures shared by integration tests.
//!
//! Integration tests write these into temp packages to exercise parsing,
//! resolution, hover, and LSP span mapping against a realistic core-library layout.

pub const BOOL_GIN: &str = r#"use core.Happy
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
Bool has Happy and ToString
    Happy.value: Bool.True
    ToString.to_string: when self then 'true' else 'false'


false := Bool.False
true  := Bool.True


-- is_empty(v Maybe(x)) Bool:
--     if v is None return True
-- return False
"#;

pub const COPY_GIN: &str = r#"use core.reflect.(Type, NamedTy, VariantShape)
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

pub const INT_GIN: &str = r#"--- The 8-bit signed integer type.
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

pub const LIST_GIN: &str = r#"use Pointer, PointerSize

--- A homogeneous list type.
--- List literals use bracket syntax: ['linux', 'macos'].
--- Generic over element type: `List(x)`.
List(x) has pointer Pointer(x), length PointerSize
"#;

pub const POINTER_GIN: &str = r#"use '../target/'.target
use BigInt, Int

Pointer(x) is @x

--- The pointer-width unsigned integer type (8 bytes on 64-bit, 4 bytes on 32-bit).
PointerSize is when target.arch is
    'x86_64' or 'arm64' then BigInt
    'wasm32'            then Int
"#;

pub const SIZED_GIN: &str = r#"use '../reflect/'.(Type, NamedTy, VariantShape)
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

pub const STRING_GIN: &str = r#"use core.primitive.(Byte, List)

--- Canonical `String` type used for printing/codegen.
String has bytes List(Byte)

ToString has to_string String
"#;

/// Record field types that reference undeclared tags (`List`, `Byte`) without imports.
pub const STRING_GIN_SOURCE: &str = "\
String has bytes List(Byte)\n\
\n\
ToString has to_string String\n\
";

pub const TYPE_GIN: &str = r#"use '../primitive/'.(BigInt, Bool, List)
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
--- User-written `Type has Reflectable` is a compile error.
Reflectable has shape Type
"#;
