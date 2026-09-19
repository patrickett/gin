//! Capability-focused Gin source fixtures shared by integration tests.
//!
//! Integration tests write these into temp packages to exercise parsing,
//! resolution, hover, and LSP span mapping against a realistic core-library layout.

pub const BOOL_GIN: &str = r#"use core Happy
use core ToString

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

pub const COPY_GIN: &str = r#"use core.reflect ReflectionType, ReflectionTypeNamed, ReflectionVariantShape, ReflectionBits, ReflectionAddress, ReflectionRawPointer, ReflectionSafeReference, ReflectionObserve, ReflectionMutate, ReflectionUnsupported, ReflectionRecord, ReflectionUnion, ReflectionArray
use core.primitive Bool, List

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
SignedTinyInt is in -128 to 127
--- The 16-bit signed integer type.
SignedSmallInt is in -32768 to 32767
--- The 32-bit signed integer type.
SignedInt is in -2147483648 to 2147483647
--- The 64-bit signed integer type.
SignedBigInt is in -9223372036854775808 to 9223372036854775807
--- The 128-bit signed integer type.
SignedLargeInt is in -170141183460469231731687303715884105728 to 170141183460469231731687303715884105727
--- The 8-bit unsigned integer type.
TinyInt is in 0 to 255
--- The 16-bit unsigned integer type.
SmallInt is in 0 to 65535
--- The 32-bit unsigned integer type.
Int is in 0 to 4294967295
--- The 64-bit unsigned integer type.
BigInt is in 0 to 18446744073709551615
--- The 128-bit unsigned integer type.
LargeInt is in 0 to 340282366920938463463374607431768211455


--- Alias for the 8-bit unsigned integer type.
Byte is TinyInt
"#;

pub const LIST_GIN: &str = r#"use PointerSize

--- A homogeneous list type.
--- List literals use bracket syntax: ['linux', 'macos'].
--- Generic over element type: `List(x)`.
List(x) has pointer @x, length PointerSize
"#;

pub const POINTER_GIN: &str = r#"use '../target/' target
use BigInt, Int

--- The pointer-width unsigned integer type (8 bytes on 64-bit, 4 bytes on 32-bit).
PointerSize is when target.arch is
    'x86_64' or 'arm64' then BigInt
    'wasm32'            then Int
"#;

pub const SIZED_GIN: &str = r#"use core.reflect ReflectionType, ReflectionTypeNamed, ReflectionVariantShape, ReflectionBits, ReflectionAddress, ReflectionRawPointer, ReflectionSafeReference, ReflectionUnsupported, ReflectionRecord, ReflectionUnion, ReflectionArray
use core.primitive BigInt, List

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

pub const STRING_GIN: &str = r#"use core.primitive Byte, PointerSize

--- A UTF-8 string represented by a byte pointer and byte length.
String has pointer @Byte, len PointerSize

ToString has to_string String
"#;

/// Record field types that reference undeclared tags (`List`, `Byte`) without imports.
pub const STRING_GIN_SOURCE: &str = "\
String has bytes List(Byte)\n\
\n\
ToString has to_string String\n\
";

pub const COUNT_GIN: &str = r#"use '../primitive/' Byte

index_bits := #index_bits(@())
index_sign := 1 << (index_bits - 1)
max_count := index_sign - 1
min_offset := 0 - max_count
max_alignment := 1 << (index_bits - 2)

#phantom(unit)
#bits(index_bits)
Count(unit) is in 0 to max_count

#phantom(unit)
#bits(index_bits)
Offset(unit) is in min_offset to max_count

#bits(index_bits)
Alignment is in 1 to max_alignment and max_alignment % self = 0

ByteCount is Count(Byte)
ByteOffset is Offset(Byte)

is_alignment(bytes ref is in 0 to max_count) True thus bytes >= 1 and bytes <= max_alignment and max_alignment % bytes = 0 or False:
    if bytes is in 1 to max_alignment
        candidate is in 1 to max_alignment and max_alignment % candidate = 0: 1
        while candidate < bytes is True
            candidate *: 2
        loop
        if candidate = bytes is True
            True
        return
        False
    return
return False

Alignment has
    new(bytes is in 1 to max_alignment and max_alignment % bytes = 0) Self:
    return bytes as Alignment

    try_new(bytes is in 0 to max_count):
        if is_alignment(ref bytes) is True
            Valid(bytes as Alignment)
        return
    return Invalid
"#;

pub const TYPE_GIN: &str = r#"use '../primitive.' BigInt, Bool, List
use '../string/' String

--- Structural description of a Gin type, used for compile-time reflection.
Type is Bits(width BigInt, interpretation IntInterpretation)
     or Address(space BigInt, pointee Type)
     or Product(name String, fields List(ReflectionTypeNamed))
     or Sum(name String, variants List(ReflectionVariantShape))
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

pub const MACHINE_X86: &str = r#"architecture := (
    target: #target.triple
    arch: "x86_64"
    dialect: "intel"
    inputs: (inputs0: "rdi", inputs1: "rsi")
    references: (references0: "rax", references1: "rcx")
    scratch: (scratch0: "rax", scratch1: "rcx", scratch2: "rdx", scratch3: "r8", scratch4: "r9", scratch5: "r10", scratch6: "r11")
    result: "rax"
    length: "rdx"
    copy: "mov {destination}, {left}"
    add: "add {destination}, {right}"
    load: "mov {destination}, qword ptr [{left}]"
    store: "mov qword ptr [{destination}], {left}"
    nop: "nop"
    add_inout: 1
    add_clobbers: ["cc"]
)
"#;

pub const MACHINE_ARM: &str = r#"architecture := (
    target: #target.triple
    arch: "arm64"
    dialect: "att"
    inputs: (inputs0: "x0", inputs1: "x1")
    references: (references0: "x9", references1: "x10")
    scratch: (scratch0: "x9", scratch1: "x10", scratch2: "x11", scratch3: "x12", scratch4: "x13", scratch5: "x14", scratch6: "x15")
    result: "x9"
    length: "x2"
    copy: "mov {destination}, {left}"
    add: "add {destination}, {left}, {right}"
    load: "ldr {destination}, [{left}]"
    store: "str {left}, [{destination}]"
    nop: "nop"
    add_inout: 0
    add_clobbers: ()
)
"#;

pub const RUNTIME: &str = r#"
RuntimeByte is in 0 to 255
RuntimePointer is @RuntimeByte
RuntimeWord is in 0 to 18446744073709551615
RuntimeInt is in -9223372036854775808 to 9223372036854775807

runtime_reserve_spec := when #target.triple is
    'aarch64-apple-darwin' then "gin-asm 1\ntarget \"aarch64-apple-darwin\"\ndialect att\ninput \"size\" fixed \"x9\"\nresult fixed \"x0\"\nclobber \"x1\"\nclobber \"x2\"\nclobber \"x3\"\nclobber \"x4\"\nclobber \"x5\"\nclobber \"x16\"\nclobber \"cc\"\noption side_effects\ntemplate \"mov x0, #0\\nmov x1, x9\\nmov x2, #3\\nmov x3, #4098\\nmov x4, #-1\\nmov x5, #0\\nmov x16, #197\\nsvc #128\\nb.cc 1f\\nmov x0, #0\\n1:\""
    'aarch64-unknown-linux-gnu' then "gin-asm 1\ntarget \"aarch64-unknown-linux-gnu\"\ndialect att\ninput \"size\" fixed \"x9\"\nresult fixed \"x0\"\nclobber \"x1\"\nclobber \"x2\"\nclobber \"x3\"\nclobber \"x4\"\nclobber \"x5\"\nclobber \"x8\"\nclobber \"cc\"\noption side_effects\ntemplate \"mov x0, #0\\nmov x1, x9\\nmov x2, #3\\nmov x3, #34\\nmov x4, #-1\\nmov x5, #0\\nmov x8, #222\\nsvc #0\\ncmn x0, #4095\\nb.cc 1f\\nmov x0, #0\\n1:\""
    'x86_64-unknown-linux-gnu' then "gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\ninput \"size\" fixed \"rsi\"\nresult fixed \"rax\"\nclobber \"rdi\"\nclobber \"rdx\"\nclobber \"r10\"\nclobber \"r8\"\nclobber \"r9\"\nclobber \"rcx\"\nclobber \"r11\"\nclobber \"cc\"\noption side_effects\ntemplate \"mov rax, 9\\nxor edi, edi\\nmov edx, 3\\nmov r10d, 34\\nmov r8, -1\\nxor r9d, r9d\\nsyscall\\ncmp rax, -4095\\njb 9f\\nxor eax, eax\\n9:\""
    'x86_64-apple-darwin' then "gin-asm 1\ntarget \"x86_64-apple-darwin\"\ndialect intel\ninput \"size\" fixed \"rsi\"\nresult fixed \"rax\"\nclobber \"rdi\"\nclobber \"rdx\"\nclobber \"r10\"\nclobber \"r8\"\nclobber \"r9\"\nclobber \"rcx\"\nclobber \"r11\"\nclobber \"cc\"\noption side_effects\ntemplate \"mov rax, 33554629\\nxor edi, edi\\nmov edx, 3\\nmov r10d, 4098\\nmov r8, -1\\nxor r9d, r9d\\nsyscall\\njnc 9f\\nxor eax, eax\\n9:\""
    else ""

runtime_release_spec := when #target.triple is
    'aarch64-apple-darwin' then "gin-asm 1\ntarget \"aarch64-apple-darwin\"\ndialect att\ninput \"pointer\" fixed \"x9\"\ninput \"size\" fixed \"x10\"\nresult fixed \"x0\"\nclobber \"x1\"\nclobber \"x16\"\nclobber \"cc\"\nmemory write pointer=\"pointer\" length=\"size\"\noption side_effects\ntemplate \"mov x0, x9\\nmov x1, x10\\nmov x16, #73\\nsvc #128\""
    'aarch64-unknown-linux-gnu' then "gin-asm 1\ntarget \"aarch64-unknown-linux-gnu\"\ndialect att\ninput \"pointer\" fixed \"x9\"\ninput \"size\" fixed \"x10\"\nresult fixed \"x0\"\nclobber \"x1\"\nclobber \"x8\"\nclobber \"cc\"\nmemory write pointer=\"pointer\" length=\"size\"\noption side_effects\ntemplate \"mov x0, x9\\nmov x1, x10\\nmov x8, #215\\nsvc #0\""
    'x86_64-unknown-linux-gnu' then "gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\ninput \"pointer\" fixed \"rdi\"\ninput \"size\" fixed \"rsi\"\nresult fixed \"rax\"\nclobber \"rcx\"\nclobber \"r11\"\nclobber \"cc\"\nmemory write pointer=\"pointer\" length=\"size\"\noption side_effects\ntemplate \"mov rax, 11\\nsyscall\""
    'x86_64-apple-darwin' then "gin-asm 1\ntarget \"x86_64-apple-darwin\"\ndialect intel\ninput \"pointer\" fixed \"rdi\"\ninput \"size\" fixed \"rsi\"\nresult fixed \"rax\"\nclobber \"rcx\"\nclobber \"r11\"\nclobber \"cc\"\nmemory write pointer=\"pointer\" length=\"size\"\noption side_effects\ntemplate \"mov rax, 33554505\\nsyscall\""
    else ""

runtime_nonzero(size RuntimeWord) RuntimeWord:
    count is in 0 to 18446744073709551615: size
return when count = 0 is True then 1 else size

runtime_reserve(size RuntimeWord) @RuntimeByte: #asm(runtime_reserve_spec, size: runtime_nonzero(size))
runtime_release(pointer @RuntimeByte, size RuntimeWord):
    result RuntimeInt: #asm(runtime_release_spec, pointer: pointer, size: runtime_nonzero(size))
return

runtime_read_spec := when #target.triple is
    'aarch64-apple-darwin' then "gin-asm 1\ntarget \"aarch64-apple-darwin\"\ndialect att\ninput \"fd\" fixed \"x9\"\ninput \"pointer\" fixed \"x10\"\ninput \"size\" fixed \"x11\"\nresult fixed \"x0\"\nclobber \"x1\"\nclobber \"x2\"\nclobber \"x16\"\nclobber \"cc\"\nmemory write pointer=\"pointer\" length=\"size\"\noption side_effects\ntemplate \"mov x0, x9\\nmov x1, x10\\nmov x2, x11\\nmov x16, #3\\nsvc #128\""
    'aarch64-unknown-linux-gnu' then "gin-asm 1\ntarget \"aarch64-unknown-linux-gnu\"\ndialect att\ninput \"fd\" fixed \"x9\"\ninput \"pointer\" fixed \"x10\"\ninput \"size\" fixed \"x11\"\nresult fixed \"x0\"\nclobber \"x1\"\nclobber \"x2\"\nclobber \"x8\"\nclobber \"cc\"\nmemory write pointer=\"pointer\" length=\"size\"\noption side_effects\ntemplate \"mov x0, x9\\nmov x1, x10\\nmov x2, x11\\nmov x8, #63\\nsvc #0\""
    'x86_64-unknown-linux-gnu' then "gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\ninput \"fd\" fixed \"rdi\"\ninput \"pointer\" fixed \"rsi\"\ninput \"size\" fixed \"rdx\"\nresult fixed \"rax\"\nclobber \"rcx\"\nclobber \"r11\"\nclobber \"cc\"\nmemory write pointer=\"pointer\" length=\"size\"\noption side_effects\ntemplate \"mov rax, 0\\nsyscall\""
    'x86_64-apple-darwin' then "gin-asm 1\ntarget \"x86_64-apple-darwin\"\ndialect intel\ninput \"fd\" fixed \"rdi\"\ninput \"pointer\" fixed \"rsi\"\ninput \"size\" fixed \"rdx\"\nresult fixed \"rax\"\nclobber \"rcx\"\nclobber \"r11\"\nclobber \"cc\"\nmemory write pointer=\"pointer\" length=\"size\"\noption side_effects\ntemplate \"mov rax, 33554435\\nsyscall\""
    else ""

runtime_write_spec := when #target.triple is
    'aarch64-apple-darwin' then "gin-asm 1\ntarget \"aarch64-apple-darwin\"\ndialect att\ninput \"fd\" fixed \"x9\"\ninput \"pointer\" fixed \"x10\"\ninput \"size\" fixed \"x11\"\nresult fixed \"x0\"\nclobber \"x1\"\nclobber \"x2\"\nclobber \"x16\"\nclobber \"cc\"\nmemory read pointer=\"pointer\" length=\"size\"\noption side_effects\ntemplate \"mov x0, x9\\nmov x1, x10\\nmov x2, x11\\nmov x16, #4\\nsvc #128\""
    'aarch64-unknown-linux-gnu' then "gin-asm 1\ntarget \"aarch64-unknown-linux-gnu\"\ndialect att\ninput \"fd\" fixed \"x9\"\ninput \"pointer\" fixed \"x10\"\ninput \"size\" fixed \"x11\"\nresult fixed \"x0\"\nclobber \"x1\"\nclobber \"x2\"\nclobber \"x8\"\nclobber \"cc\"\nmemory read pointer=\"pointer\" length=\"size\"\noption side_effects\ntemplate \"mov x0, x9\\nmov x1, x10\\nmov x2, x11\\nmov x8, #64\\nsvc #0\""
    'x86_64-unknown-linux-gnu' then "gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\ninput \"fd\" fixed \"rdi\"\ninput \"pointer\" fixed \"rsi\"\ninput \"size\" fixed \"rdx\"\nresult fixed \"rax\"\nclobber \"rcx\"\nclobber \"r11\"\nclobber \"cc\"\nmemory read pointer=\"pointer\" length=\"size\"\noption side_effects\ntemplate \"mov rax, 1\\nsyscall\""
    'x86_64-apple-darwin' then "gin-asm 1\ntarget \"x86_64-apple-darwin\"\ndialect intel\ninput \"fd\" fixed \"rdi\"\ninput \"pointer\" fixed \"rsi\"\ninput \"size\" fixed \"rdx\"\nresult fixed \"rax\"\nclobber \"rcx\"\nclobber \"r11\"\nclobber \"cc\"\nmemory read pointer=\"pointer\" length=\"size\"\noption side_effects\ntemplate \"mov rax, 33554436\\nsyscall\""
    else ""

runtime_read(fd RuntimeInt, pointer @RuntimeByte, size RuntimeWord) RuntimeInt: #asm(runtime_read_spec, fd: fd, pointer: pointer, size: size)
runtime_write(fd RuntimeInt, pointer @RuntimeByte, size RuntimeWord) RuntimeInt: #asm(runtime_write_spec, fd: fd, pointer: pointer, size: size)


runtime_copy_pointer(value ref RuntimePointer) RuntimePointer: deref value

runtime_number_spec := when #target.triple is
    'aarch64-apple-darwin' then "gin-asm 1\ntarget \"aarch64-apple-darwin\"\ndialect att\ninput \"pointer\" fixed \"x9\"\nresult fixed \"x0\"\nclobber \"x1\"\nclobber \"x2\"\nclobber \"x11\"\nclobber \"x12\"\nclobber \"x13\"\nclobber \"x14\"\nclobber \"x15\"\nclobber \"x16\"\nclobber \"cc\"\noption side_effects\ntemplate \"mov x12, #0\\nmov x13, #1\\nmov x14, #0\\n1:\\nmov x0, #0\\nmov x1, x9\\nmov x2, #1\\nmov x16, #3\\nsvc #128\\ncmp x0, #1\\nb.ne 4f\\nldrb w15, [x9]\\ncbnz x14, 2f\\ncmp w15, #32\\nb.ls 1b\\nmov x14, #1\\ncmp w15, #45\\nb.ne 2f\\nmov x13, #-1\\nb 1b\\n2:\\nsub w15, w15, #48\\ncmp w15, #9\\nb.hi 4f\\nmov x11, #10\\nmadd x12, x12, x11, x15\\nb 1b\\n4:\\nmul x0, x12, x13\""
    'aarch64-unknown-linux-gnu' then "gin-asm 1\ntarget \"aarch64-unknown-linux-gnu\"\ndialect att\ninput \"pointer\" fixed \"x9\"\nresult fixed \"x0\"\nclobber \"x1\"\nclobber \"x2\"\nclobber \"x11\"\nclobber \"x12\"\nclobber \"x13\"\nclobber \"x14\"\nclobber \"x15\"\nclobber \"x8\"\nclobber \"cc\"\noption side_effects\ntemplate \"mov x12, #0\\nmov x13, #1\\nmov x14, #0\\n1:\\nmov x0, #0\\nmov x1, x9\\nmov x2, #1\\nmov x8, #63\\nsvc #0\\ncmp x0, #1\\nb.ne 4f\\nldrb w15, [x9]\\ncbnz x14, 2f\\ncmp w15, #32\\nb.ls 1b\\nmov x14, #1\\ncmp w15, #45\\nb.ne 2f\\nmov x13, #-1\\nb 1b\\n2:\\nsub w15, w15, #48\\ncmp w15, #9\\nb.hi 4f\\nmov x11, #10\\nmadd x12, x12, x11, x15\\nb 1b\\n4:\\nmul x0, x12, x13\""
    'x86_64-unknown-linux-gnu' then "gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\ninput \"pointer\" fixed \"r12\"\nresult fixed \"rax\"\nclobber \"rdi\"\nclobber \"rsi\"\nclobber \"rdx\"\nclobber \"r8\"\nclobber \"r10\"\nclobber \"rcx\"\nclobber \"r11\"\nclobber \"cc\"\nclobber \"r9\"\noption side_effects\ntemplate \"xor r8d, r8d\\nmov r9, 1\\nxor r10d, r10d\\n9:\\nxor edi, edi\\nmov rsi, r12\\nmov edx, 1\\nmov eax, 0\\nsyscall\\ncmp rax, 1\\njne 4f\\nmovzx edx, byte ptr [r12]\\ntest r10, r10\\njnz 2f\\ncmp edx, 32\\njbe 9b\\nmov r10, 1\\ncmp edx, 45\\njne 2f\\nmov r9, -1\\njmp 9b\\n2:\\nsub edx, 48\\ncmp edx, 9\\nja 4f\\nimul r8, r8, 10\\nadd r8, rdx\\njmp 9b\\n4:\\nmov rax, r8\\nimul rax, r9\""
    'x86_64-apple-darwin' then "gin-asm 1\ntarget \"x86_64-apple-darwin\"\ndialect intel\ninput \"pointer\" fixed \"r12\"\nresult fixed \"rax\"\nclobber \"rdi\"\nclobber \"rsi\"\nclobber \"rdx\"\nclobber \"r8\"\nclobber \"r10\"\nclobber \"rcx\"\nclobber \"r11\"\nclobber \"cc\"\nclobber \"r9\"\noption side_effects\ntemplate \"xor r8d, r8d\\nmov r9, 1\\nxor r10d, r10d\\n9:\\nxor edi, edi\\nmov rsi, r12\\nmov edx, 1\\nmov eax, 33554435\\nsyscall\\ncmp rax, 1\\njne 4f\\nmovzx edx, byte ptr [r12]\\ntest r10, r10\\njnz 2f\\ncmp edx, 32\\njbe 9b\\nmov r10, 1\\ncmp edx, 45\\njne 2f\\nmov r9, -1\\njmp 9b\\n2:\\nsub edx, 48\\ncmp edx, 9\\nja 4f\\nimul r8, r8, 10\\nadd r8, rdx\\njmp 9b\\n4:\\nmov rax, r8\\nimul rax, r9\""
    else ""

runtime_number RuntimeInt:
    pointer RuntimePointer: runtime_reserve(1)
    result RuntimeInt: #asm(runtime_number_spec, pointer: runtime_copy_pointer(ref pointer))
    runtime_release(pointer, 1)
return result

runtime_print_spec := when #target.triple is
    'aarch64-apple-darwin' then "gin-asm 1\ntarget \"aarch64-apple-darwin\"\ndialect att\ninput \"value\" fixed \"x9\"\ninput \"pointer\" fixed \"x10\"\nresult fixed \"x0\"\nclobber \"x1\"\nclobber \"x2\"\nclobber \"x11\"\nclobber \"x12\"\nclobber \"x13\"\nclobber \"x14\"\nclobber \"x15\"\nclobber \"x16\"\nclobber \"cc\"\noption side_effects\ntemplate \"add x11, x10, #31\\nmov w15, #10\\nstrb w15, [x11]\\ncmp x9, #0\\ncneg x12, x9, lt\\nmov x13, #10\\n1:\\nudiv x14, x12, x13\\nmsub x15, x14, x13, x12\\nadd w15, w15, #48\\nstrb w15, [x11, #-1]!\\nmov x12, x14\\ncbnz x12, 1b\\ntbz x9, #63, 2f\\nmov w15, #45\\nstrb w15, [x11, #-1]!\\n2:\\nmov x0, #1\\nmov x1, x11\\nadd x2, x10, #32\\nsub x2, x2, x11\\nmov x16, #4\\nsvc #128\\nmov x0, x9\""
    'aarch64-unknown-linux-gnu' then "gin-asm 1\ntarget \"aarch64-unknown-linux-gnu\"\ndialect att\ninput \"value\" fixed \"x9\"\ninput \"pointer\" fixed \"x10\"\nresult fixed \"x0\"\nclobber \"x1\"\nclobber \"x2\"\nclobber \"x11\"\nclobber \"x12\"\nclobber \"x13\"\nclobber \"x14\"\nclobber \"x15\"\nclobber \"x8\"\nclobber \"cc\"\noption side_effects\ntemplate \"add x11, x10, #31\\nmov w15, #10\\nstrb w15, [x11]\\ncmp x9, #0\\ncneg x12, x9, lt\\nmov x13, #10\\n1:\\nudiv x14, x12, x13\\nmsub x15, x14, x13, x12\\nadd w15, w15, #48\\nstrb w15, [x11, #-1]!\\nmov x12, x14\\ncbnz x12, 1b\\ntbz x9, #63, 2f\\nmov w15, #45\\nstrb w15, [x11, #-1]!\\n2:\\nmov x0, #1\\nmov x1, x11\\nadd x2, x10, #32\\nsub x2, x2, x11\\nmov x8, #64\\nsvc #0\\nmov x0, x9\""
    'x86_64-unknown-linux-gnu' then "gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\ninput \"value\" fixed \"r9\"\ninput \"pointer\" fixed \"r12\"\nresult fixed \"rax\"\nclobber \"rdi\"\nclobber \"rsi\"\nclobber \"rdx\"\nclobber \"r8\"\nclobber \"r10\"\nclobber \"rcx\"\nclobber \"r11\"\nclobber \"cc\"\noption side_effects\ntemplate \"mov r8, r9\\nmov rax, r9\\ntest rax, rax\\njns 9f\\nneg rax\\n9:\\nlea rsi, [r12 + 31]\\nmov byte ptr [rsi], 10\\nmov ecx, 10\\n2:\\nxor edx, edx\\ndiv rcx\\nadd dl, 48\\ndec rsi\\nmov byte ptr [rsi], dl\\ntest rax, rax\\njnz 2b\\ntest r8, r8\\njns 3f\\ndec rsi\\nmov byte ptr [rsi], 45\\n3:\\nlea rdx, [r12 + 32]\\nsub rdx, rsi\\nmov edi, 1\\nmov eax, 1\\nsyscall\\nmov rax, r8\""
    'x86_64-apple-darwin' then "gin-asm 1\ntarget \"x86_64-apple-darwin\"\ndialect intel\ninput \"value\" fixed \"r9\"\ninput \"pointer\" fixed \"r12\"\nresult fixed \"rax\"\nclobber \"rdi\"\nclobber \"rsi\"\nclobber \"rdx\"\nclobber \"r8\"\nclobber \"r10\"\nclobber \"rcx\"\nclobber \"r11\"\nclobber \"cc\"\noption side_effects\ntemplate \"mov r8, r9\\nmov rax, r9\\ntest rax, rax\\njns 9f\\nneg rax\\n9:\\nlea rsi, [r12 + 31]\\nmov byte ptr [rsi], 10\\nmov ecx, 10\\n2:\\nxor edx, edx\\ndiv rcx\\nadd dl, 48\\ndec rsi\\nmov byte ptr [rsi], dl\\ntest rax, rax\\njnz 2b\\ntest r8, r8\\njns 3f\\ndec rsi\\nmov byte ptr [rsi], 45\\n3:\\nlea rdx, [r12 + 32]\\nsub rdx, rsi\\nmov edi, 1\\nmov eax, 33554436\\nsyscall\\nmov rax, r8\""
    else ""

runtime_print(value RuntimeInt) RuntimeInt:
    pointer RuntimePointer: runtime_reserve(32)
    result RuntimeInt: #asm(runtime_print_spec, value: value, pointer: runtime_copy_pointer(ref pointer))
    runtime_release(pointer, 32)
return result

runtime_copy_input_spec := when #target.triple is
    'aarch64-apple-darwin' then "gin-asm 1\ntarget \"aarch64-apple-darwin\"\ndialect att\ninput \"pointer\" fixed \"x9\"\nresult fixed \"x0\"\nclobber \"x1\"\nclobber \"x2\"\nclobber \"x11\"\nclobber \"x12\"\nclobber \"x13\"\nclobber \"x14\"\nclobber \"x15\"\nclobber \"x16\"\nclobber \"cc\"\noption side_effects\ntemplate \"1:\\nmov x0, #0\\nmov x1, x9\\nmov x2, #1\\nmov x16, #3\\nsvc #128\\ncmp x0, #1\\nb.ne 2f\\nmov x0, #1\\nmov x1, x9\\nmov x2, #1\\nmov x16, #4\\nsvc #128\\nb 1b\\n2:\\nmov x0, #0\""
    'aarch64-unknown-linux-gnu' then "gin-asm 1\ntarget \"aarch64-unknown-linux-gnu\"\ndialect att\ninput \"pointer\" fixed \"x9\"\nresult fixed \"x0\"\nclobber \"x1\"\nclobber \"x2\"\nclobber \"x11\"\nclobber \"x12\"\nclobber \"x13\"\nclobber \"x14\"\nclobber \"x15\"\nclobber \"x8\"\nclobber \"cc\"\noption side_effects\ntemplate \"1:\\nmov x0, #0\\nmov x1, x9\\nmov x2, #1\\nmov x8, #63\\nsvc #0\\ncmp x0, #1\\nb.ne 2f\\nmov x0, #1\\nmov x1, x9\\nmov x2, #1\\nmov x8, #64\\nsvc #0\\nb 1b\\n2:\\nmov x0, #0\""
    'x86_64-unknown-linux-gnu' then "gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\ninput \"pointer\" fixed \"r12\"\nresult fixed \"rax\"\nclobber \"rdi\"\nclobber \"rsi\"\nclobber \"rdx\"\nclobber \"r8\"\nclobber \"r10\"\nclobber \"rcx\"\nclobber \"r11\"\nclobber \"cc\"\nclobber \"r9\"\noption side_effects\ntemplate \"9:\\nxor edi, edi\\nmov rsi, r12\\nmov edx, 1\\nmov eax, 0\\nsyscall\\ncmp rax, 1\\njne 2f\\nmov edi, 1\\nmov rsi, r12\\nmov edx, 1\\nmov eax, 1\\nsyscall\\njmp 9b\\n2:\\nxor eax, eax\""
    'x86_64-apple-darwin' then "gin-asm 1\ntarget \"x86_64-apple-darwin\"\ndialect intel\ninput \"pointer\" fixed \"r12\"\nresult fixed \"rax\"\nclobber \"rdi\"\nclobber \"rsi\"\nclobber \"rdx\"\nclobber \"r8\"\nclobber \"r10\"\nclobber \"rcx\"\nclobber \"r11\"\nclobber \"cc\"\nclobber \"r9\"\noption side_effects\ntemplate \"9:\\nxor edi, edi\\nmov rsi, r12\\nmov edx, 1\\nmov eax, 33554435\\nsyscall\\ncmp rax, 1\\njne 2f\\nmov edi, 1\\nmov rsi, r12\\nmov edx, 1\\nmov eax, 33554436\\nsyscall\\njmp 9b\\n2:\\nxor eax, eax\""
    else ""

runtime_copy_input RuntimeInt:
    pointer RuntimePointer: runtime_reserve(1)
    result RuntimeInt: #asm(runtime_copy_input_spec, pointer: runtime_copy_pointer(ref pointer))
    runtime_release(pointer, 1)
return result

runtime_bits_spec := when #target.triple is
    'aarch64-apple-darwin' then "gin-asm 1\ntarget \"aarch64-apple-darwin\"\ndialect att\ninput \"value\" fixed \"x9\"\nresult fixed \"x0\"\ntemplate \"mov x0, x9\""
    'aarch64-unknown-linux-gnu' then "gin-asm 1\ntarget \"aarch64-unknown-linux-gnu\"\ndialect att\ninput \"value\" fixed \"x9\"\nresult fixed \"x0\"\ntemplate \"mov x0, x9\""
    'x86_64-unknown-linux-gnu' then "gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\ninput \"value\" fixed \"r9\"\nresult fixed \"rax\"\ntemplate \"mov rax, r9\""
    'x86_64-apple-darwin' then "gin-asm 1\ntarget \"x86_64-apple-darwin\"\ndialect intel\ninput \"value\" fixed \"r9\"\nresult fixed \"rax\"\ntemplate \"mov rax, r9\""
    else ""

runtime_word_bits(value RuntimeInt) RuntimeWord: #asm(runtime_bits_spec, value: value)
runtime_signed_bits(value RuntimeWord) RuntimeInt: #asm(runtime_bits_spec, value: value)
"#;

pub const INDEXED_COPY: &str = r#"Byte is in 0 to 255
replace(previous Byte, next Byte) Byte: next
copy(source ref Buffer(Byte,4), destination mut Buffer(Byte,4), count is in 0 to 4):
    for index in 0 to count
        destination.(index) :: replace(destination.(index), source.(index))
    loop
return
forward(source ref Buffer(Byte,4), destination mut Buffer(Byte,4), count is in 0 to 4): copy(ref source, mut destination, count)
run(count is in 0 to 4) Buffer(Byte,4):
    source Buffer(Byte,4): [3,17,128,255]
    destination Buffer(Byte,4): [41;4]
    forward(ref source, mut destination, count)
return destination

read_copied(count is in 0 to 4, index is in 0 to 3) Byte: run(count).(index)

#fixed_array(element,count)
Buffer(element Type,count BigInt) has
"#;
