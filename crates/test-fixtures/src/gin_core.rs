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

pub const COPY_GIN: &str = r#"use core.reflect.(
    ReflectionType
    ReflectionTypeNamed
    ReflectionVariantShape
    ReflectionBits
    ReflectionAddress
    ReflectionRawPointer
    ReflectionSafeReference
    ReflectionObserve
    ReflectionMutate
    ReflectionUnsupported
    ReflectionRecord
    ReflectionUnion
    ReflectionTuple
    ReflectionArray
)
use core.primitive.(Bool, List)

--- Types that can be implicitly copied.
--- Opt out with `Copy.can_copy: False`.
Copy has can_copy Bool: is_copy(Self)

is_copy(x ReflectionType) Bool := when x is
    ReflectionBits(_, _)            then True
    ReflectionAddress(_, _)          then False
    ReflectionRawPointer(_)          then False
    ReflectionSafeReference(_, ReflectionObserve) then True
    ReflectionSafeReference(_, ReflectionMutate)  then False
    ReflectionUnsupported(_)              then False
    ReflectionRecord(_, fields)      then all_named_copy(fields)
    ReflectionTuple(elems)           then all_types_copy(elems)
    ReflectionUnion(_, variants)     then all_variants_copy(variants)
    ReflectionArray(elem, _)         then is_copy(elem)

all_named_copy(fields List(ReflectionTypeNamed)) Bool := when fields is
    []                  then True
    [f, ...rest]        then is_copy(f.ty) and all_named_copy(rest)

all_types_copy(elems List(ReflectionType)) Bool := when elems is
    []                  then True
    [t, ...rest]        then is_copy(t) and all_types_copy(rest)

all_variants_copy(variants List(ReflectionVariantShape)) Bool := when variants is
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
#default(IntegerLiteral)
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

pub const SIZED_GIN: &str = r#"use core.reflect.(
    ReflectionType
    ReflectionTypeNamed
    ReflectionVariantShape
    ReflectionBits
    ReflectionAddress
    ReflectionRawPointer
    ReflectionSafeReference
    ReflectionUnsupported
    ReflectionRecord
    ReflectionUnion
    ReflectionTuple
    ReflectionArray
)
use core.primitive.(BigInt, List)

--- Compile-time byte size of a type.
Size is Const(BigInt) or Dynamic

Sized has size Size: compute_size(Self)

compute_size(x ReflectionType) Size := when x is
    ReflectionBits(w, _)           then Const(w / 8)
    ReflectionAddress(_, _)         then Const(8)
    ReflectionRawPointer(_)         then Const(8)
    ReflectionSafeReference(_, _)   then Const(8)
    ReflectionUnsupported(_)             then Dynamic
    ReflectionRecord(_, fields)     then sum_named(fields)
    ReflectionTuple(elems)          then sum_types(elems)
    ReflectionUnion(_, variants)    then union_size(variants)
    ReflectionArray(elem, n)        then mul_size(compute_size(elem), n)

sum_named(fields List(ReflectionTypeNamed)) Size := when fields is
    []                  then Const(0)
    [f, ...rest]        then add(compute_size(f.ty), sum_named(rest))

sum_types(elems List(ReflectionType)) Size := when elems is
    []                  then Const(0)
    [t, ...rest]        then add(compute_size(t), sum_types(rest))

add(a Size, b Size) Size := when (a, b) is
    (Const(x), Const(y)) then Const(x + y)
                         else Dynamic

mul_size(size Size, n BigInt) Size := when size is
    Const(x) then Const(x * n)
             else Dynamic

union_disc(variants List(ReflectionVariantShape)) Size := when variants is
    []           then Const(0)
    [_]          then Const(1)
                 else Const(2)

union_max_payload(variants List(ReflectionVariantShape)) Size := when variants is
    []              then Const(0)
    [v, ...rest]    then max_size(sum_named(v.fields), union_max_payload(rest))

max_size(a Size, b Size) Size := when (a, b) is
    (Const(x), Const(y)) then Const(when x > y then x else y)
                         else Dynamic

union_size(variants List(ReflectionVariantShape)) Size := add(union_disc(variants), union_max_payload(variants))
"#;

pub const STRING_GIN: &str = r#"use core.primitive.(Byte, Pointer, PointerSize)

--- A UTF-8 string represented by a byte pointer and byte length.
String has pointer Pointer(Byte), len PointerSize

ToString has to_string String
"#;

/// Record field types that reference undeclared tags (`List`, `Byte`) without imports.
pub const STRING_GIN_SOURCE: &str = "\
String has bytes List(Byte)\n\
\n\
ToString has to_string String\n\
";

pub const TYPE_GIN: &str = r#"use '../primitive.'.(BigInt, Bool, List)
use '../string/'.String

--- Structural description of a Gin type, used for compile-time reflection.
Type is Bits(width BigInt, interpretation IntInterpretation)
     or Address(space BigInt, pointee Type)
     or Product(name String, fields List(ReflectionTypeNamed))
     or Sum(name String, variants List(ReflectionVariantShape))
     or Tuple(elems List(Type))
     or SafeReference(pointee Type, permission IntPermission)
     or RawPointer(pointee Type)
     or Array(elem Type, size BigInt)
     or Opaque(name String)
     or Unsupported(reason String)
     or Unit

IntInterpretation is ReflectionSigned or ReflectionUnsigned
IntPermission is ReflectionObserve or ReflectionMutate

ReflectionTypeNamed is name String, ty Type
ReflectionVariantShape is name String, fields List(ReflectionTypeNamed)

#ReflectionTrait
ReflectionShape is shape Type

#ReflectionAnonymous
ReflectionAnonymous is Type

Reflectable has shape Type

#ReflectionType
ReflectionType is Type

#ReflectionNamed
ReflectionTypeNamed is name String, ty Type

#ReflectionBits
ReflectionBits is Bits

#ReflectionAddress
ReflectionAddress is Address

#ReflectionProduct
ReflectionProduct is Product

#ReflectionSum
ReflectionSum is Sum

#ReflectionInteger
ReflectionInteger is Bits

#ReflectionRecord
ReflectionRecord is Product

#ReflectionUnion
ReflectionUnion is Sum

#ReflectionTuple
ReflectionTuple is Tuple

#ReflectionSafeReference
ReflectionSafeReference is SafeReference

#ReflectionRawPointer
ReflectionRawPointer is RawPointer

#ReflectionUnsigned
ReflectionUnsigned is Unit

#ReflectionSigned
ReflectionSigned is Unit

#ReflectionObserve
ReflectionObserve is IntPermission

#ReflectionMutate
ReflectionMutate is IntPermission

#ReflectionTypeArgument
ReflectionTypeArgument is arg Type

#ReflectionConstArgument
ReflectionConstArgument is arg BigInt

#ReflectionDeclarationKey
ReflectionDeclarationKey is value String

#ReflectionRecursiveReference
ReflectionRecursiveReference is target String, depth Int

#ReflectionUnsupported
ReflectionUnsupported is Unsupported

#ReflectionUnit
ReflectionUnit is Unit

#ReflectionArray
ReflectionArray is Array
"#;
