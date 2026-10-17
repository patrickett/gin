//! Synthetic Gin sources shared by the lexer and parser benchmarks.

/// Simple Tag `is` declare (~8 lines).
pub const MAYBE_GIN: &str = "\
Maybe(x) is
    Some(x) or
    None
";

/// Range declares with doc comments (~24 lines).
pub const INT_GIN: &str = "\
--- The 8-bit signed integer type.
SignedTinyInt is in -128 to 127
--- The 16-bit signed integer type.
SignedSmallInt is in -32768 to 32767
--- The 32-bit signed integer type.
SignedInt is in -2147483648 to 2147483647
--- The 64-bit signed integer type.
SignedBigInt is in -9223372036854775808 to 9223372036854775807
--- The 8-bit unsigned integer type.
TinyInt is in 0 to 255
--- The 16-bit unsigned integer type.
SmallInt is in 0 to 65535
--- The 32-bit unsigned integer type.
Int is in 0 to 4294967295
--- The 64-bit unsigned integer type.
BigInt is in 0 to 18446744073709551615
--- Alias for the 8-bit unsigned integer type.
Byte is TinyInt
";

/// Auto trait + when-is (~32 lines).
pub const COPY_GIN: &str = "\
Type is Primitive(width BigInt, signed Bool)
     or Record(name String, fields List(NamedTy))
     or Union(name String, variants List(VariantShape))
     or Ptr(inner Type)
     or Opaque(name String)

NamedTy has name String, ty Type
VariantShape has name String, fields List(NamedTy)
Bool is True or False
BigInt is in 0 to 18446744073709551615
List(x) has pointer @x, length BigInt
String has bytes List(BigInt)

Copy has can_copy Bool: is_copy(Self)

is_copy(x Type) Bool := when x is
    Primitive(_, _)     then True
    Ptr(_)              then False
    Opaque(_)           then False
    Record(_, fields)   then all_named_copy(fields)
    Union(_, variants)  then all_variants_copy(variants)

all_named_copy(fields List(NamedTy)) Bool := when fields is
    []                  then True
    [f, ...rest]        then is_copy(f.ty) and all_named_copy(rest)

all_variants_copy(variants List(VariantShape)) Bool := when variants is
    []                  then True
    [v, ...rest]        then all_named_copy(v.fields) and all_variants_copy(rest)
";

/// When-is recursion + add/mul helpers (~53 lines).
pub const SIZED_GIN: &str = "\
Type is Primitive(width BigInt, signed Bool)
     or Record(name String, fields List(NamedTy))
     or Union(name String, variants List(VariantShape))
     or Ptr(inner Type)
     or Opaque(name String)

NamedTy has name String, ty Type
VariantShape has name String, fields List(NamedTy)
Bool is True or False
BigInt is in 0 to 18446744073709551615
List(x) has pointer @x, length BigInt
String has bytes List(BigInt)

Size is Const(BigInt) or Dynamic
Sized has size Size: compute_size(Self)

compute_size(x Type) Size := when x is
    Primitive(w, _)     then Const(w / 8)
    Ptr(_)              then Const(8)
    Opaque(_)           then Dynamic
    Record(_, fields)   then sum_named(fields)
    Union(_, variants)  then union_size(variants)

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
