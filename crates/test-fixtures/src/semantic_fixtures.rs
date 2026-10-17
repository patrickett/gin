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

pub const POINTER_GIN: &str = r#"use '../target/' target
use BigInt, Int

--- The pointer-width unsigned integer type (8 bytes on 64-bit, 4 bytes on 32-bit).
PointerSize is when target.arch is
    'x86_64' or 'aarch64' then BigInt
    'wasm32'             then Int
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
Count(unit) has
    finish(self):
        return

#phantom(unit)
#bits(index_bits)
Offset(unit) is in min_offset to max_count
Offset(unit) has
    finish(self):
        return

#bits(index_bits)
Alignment is in 1 to max_alignment and max_alignment % self = 0

ByteCount is Count(Byte)
ByteOffset is Offset(Byte)

is_alignment(bytes ref is in 0 to max_count) True thus bytes >= 1 and bytes <= max_alignment and max_alignment % bytes = 0 or False:
    when bytes is in 1 to max_alignment then
        candidate is in 1 to max_alignment and max_alignment % candidate = 0: 1
        while candidate < bytes do
            candidate *: 2
        when candidate = bytes then
            return True
        return False
    return False

Alignment has
    finish(self):
        return
    new(bytes is in 1 to max_alignment and max_alignment % bytes = 0) Self:
        return bytes as Alignment

    try_new(bytes is in 0 to max_count):
        when is_alignment(bytes) is True then
            return Valid(bytes as Alignment)
        return Invalid
"#;

pub const RUNTIME: &str = r#"
RuntimeByte is in 0 to 255
RuntimeByte has
    finish(self):
        return
RuntimePointer is @RuntimeByte
RuntimePointer has
    finish(self):
        return
RuntimeWord is in 0 to 18446744073709551615
RuntimeWord has
    finish(self):
        return
RuntimeInt is in -9223372036854775808 to 9223372036854775807
RuntimeInt has
    finish(self):
        return

#phantom(input, output)
RuntimeSignature(input, output) has
    version RuntimeByte
    finish(self):
        return
RuntimeAllocateInputs has
    size RuntimeWord
    finish(self):
        return
runtime_allocate_signature RuntimeSignature(RuntimeAllocateInputs, @RuntimeByte): RuntimeSignature(version: 1)
RuntimeDeallocateInputs has
    pointer @RuntimeByte, size RuntimeWord
    finish(self):
        return
runtime_deallocate_signature RuntimeSignature(RuntimeDeallocateInputs, RuntimeInt): RuntimeSignature(version: 1)
RuntimeReadInputs has
    fd RuntimeInt, pointer @RuntimeByte, size RuntimeWord
    finish(self):
        return
runtime_read_signature RuntimeSignature(RuntimeReadInputs, RuntimeInt): RuntimeSignature(version: 1)
RuntimeWriteInputs has
    fd RuntimeInt, pointer @RuntimeByte, size RuntimeWord
    finish(self):
        return
runtime_write_signature RuntimeSignature(RuntimeWriteInputs, RuntimeInt): RuntimeSignature(version: 1)
RuntimeWordBitsInputs has
    value RuntimeInt
    finish(self):
        return
runtime_word_bits_signature RuntimeSignature(RuntimeWordBitsInputs, RuntimeWord): RuntimeSignature(version: 1)
RuntimeSignedBitsInputs has
    value RuntimeWord
    finish(self):
        return
runtime_signed_bits_signature RuntimeSignature(RuntimeSignedBitsInputs, RuntimeInt): RuntimeSignature(version: 1)
RuntimeNumberOperationInputs has
    pointer RuntimePointer
    finish(self):
        return
runtime_number_operation_signature RuntimeSignature(RuntimeNumberOperationInputs, RuntimeInt): RuntimeSignature(version: 1)
RuntimePrintOperationInputs has
    value RuntimeInt, pointer RuntimePointer
    finish(self):
        return
runtime_print_operation_signature RuntimeSignature(RuntimePrintOperationInputs, RuntimeInt): RuntimeSignature(version: 1)
RuntimeCopyInputOperationInputs has
    pointer RuntimePointer
    finish(self):
        return
runtime_copy_input_operation_signature RuntimeSignature(RuntimeCopyInputOperationInputs, RuntimeInt): RuntimeSignature(version: 1)

runtime_reserve_spec := when #target.triple is
    'aarch64-apple-darwin' then [signature: runtime_allocate_signature, target: #target.triple, bytes: RuntimeEncoding0.encode, inputs: [size: [architecture: "aarch64", bank: "general", index: 9, width: 64]], outputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "aarch64", bank: "general", index: 1, width: 64], [architecture: "aarch64", bank: "general", index: 2, width: 64], [architecture: "aarch64", bank: "general", index: 3, width: 64], [architecture: "aarch64", bank: "general", index: 4, width: 64], [architecture: "aarch64", bank: "general", index: 5, width: 64], [architecture: "aarch64", bank: "general", index: 16, width: 64], [architecture: "aarch64", bank: "flags", index: 0, width: 4]], reads: [], writes: []]
    'aarch64-unknown-linux-gnu' then [signature: runtime_allocate_signature, target: #target.triple, bytes: RuntimeEncoding1.encode, inputs: [size: [architecture: "aarch64", bank: "general", index: 9, width: 64]], outputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "aarch64", bank: "general", index: 1, width: 64], [architecture: "aarch64", bank: "general", index: 2, width: 64], [architecture: "aarch64", bank: "general", index: 3, width: 64], [architecture: "aarch64", bank: "general", index: 4, width: 64], [architecture: "aarch64", bank: "general", index: 5, width: 64], [architecture: "aarch64", bank: "general", index: 8, width: 64], [architecture: "aarch64", bank: "flags", index: 0, width: 4]], reads: [], writes: []]
    'x86_64-unknown-linux-gnu' then [signature: runtime_allocate_signature, target: #target.triple, bytes: RuntimeEncoding2.encode, inputs: [size: [architecture: "x86_64", bank: "general", index: 6, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 7, width: 64], [architecture: "x86_64", bank: "general", index: 2, width: 64], [architecture: "x86_64", bank: "general", index: 10, width: 64], [architecture: "x86_64", bank: "general", index: 8, width: 64], [architecture: "x86_64", bank: "general", index: 9, width: 64], [architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4]], reads: [], writes: []]
    'x86_64-apple-darwin' then [signature: runtime_allocate_signature, target: #target.triple, bytes: RuntimeEncoding3.encode, inputs: [size: [architecture: "x86_64", bank: "general", index: 6, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 7, width: 64], [architecture: "x86_64", bank: "general", index: 2, width: 64], [architecture: "x86_64", bank: "general", index: 10, width: 64], [architecture: "x86_64", bank: "general", index: 8, width: 64], [architecture: "x86_64", bank: "general", index: 9, width: 64], [architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4]], reads: [], writes: []]
    else [signature: runtime_allocate_signature, target: #target.triple, bytes: RuntimeEncoding3.encode, inputs: [size: [architecture: "x86_64", bank: "general", index: 6, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 7, width: 64], [architecture: "x86_64", bank: "general", index: 2, width: 64], [architecture: "x86_64", bank: "general", index: 10, width: 64], [architecture: "x86_64", bank: "general", index: 8, width: 64], [architecture: "x86_64", bank: "general", index: 9, width: 64], [architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4]], reads: [], writes: []]

runtime_release_spec := when #target.triple is
    'aarch64-apple-darwin' then [signature: runtime_deallocate_signature, target: #target.triple, bytes: RuntimeEncoding4.encode, inputs: [pointer: [architecture: "aarch64", bank: "general", index: 9, width: 64], size: [architecture: "aarch64", bank: "general", index: 10, width: 64]], outputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "aarch64", bank: "general", index: 1, width: 64], [architecture: "aarch64", bank: "general", index: 16, width: 64], [architecture: "aarch64", bank: "flags", index: 0, width: 4]], reads: [], writes: [item0: [pointer: 0, length: 1]]]
    'aarch64-unknown-linux-gnu' then [signature: runtime_deallocate_signature, target: #target.triple, bytes: RuntimeEncoding5.encode, inputs: [pointer: [architecture: "aarch64", bank: "general", index: 9, width: 64], size: [architecture: "aarch64", bank: "general", index: 10, width: 64]], outputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "aarch64", bank: "general", index: 1, width: 64], [architecture: "aarch64", bank: "general", index: 8, width: 64], [architecture: "aarch64", bank: "flags", index: 0, width: 4]], reads: [], writes: [item0: [pointer: 0, length: 1]]]
    'x86_64-unknown-linux-gnu' then [signature: runtime_deallocate_signature, target: #target.triple, bytes: RuntimeEncoding6.encode, inputs: [pointer: [architecture: "x86_64", bank: "general", index: 7, width: 64], size: [architecture: "x86_64", bank: "general", index: 6, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4]], reads: [], writes: [item0: [pointer: 0, length: 1]]]
    'x86_64-apple-darwin' then [signature: runtime_deallocate_signature, target: #target.triple, bytes: RuntimeEncoding7.encode, inputs: [pointer: [architecture: "x86_64", bank: "general", index: 7, width: 64], size: [architecture: "x86_64", bank: "general", index: 6, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4]], reads: [], writes: [item0: [pointer: 0, length: 1]]]
    else [signature: runtime_deallocate_signature, target: #target.triple, bytes: RuntimeEncoding7.encode, inputs: [pointer: [architecture: "x86_64", bank: "general", index: 7, width: 64], size: [architecture: "x86_64", bank: "general", index: 6, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4]], reads: [], writes: [item0: [pointer: 0, length: 1]]]

runtime_nonzero(size RuntimeWord) RuntimeWord:
    count is in 0 to 18446744073709551615: size
    return when count = 0 then 1 else size

runtime_allocate(size RuntimeWord) @RuntimeByte: return #emit(runtime_reserve_spec, RuntimeAllocateInputs(size: size))
runtime_reserve(size RuntimeWord) @RuntimeByte: return runtime_allocate(runtime_nonzero(size))
runtime_deallocate(pointer @RuntimeByte, size RuntimeWord) RuntimeInt: return #emit(runtime_release_spec, RuntimeDeallocateInputs(pointer: pointer, size: size))
runtime_release(pointer @RuntimeByte, size RuntimeWord):
    result RuntimeInt: runtime_deallocate(pointer, runtime_nonzero(size))
    return

runtime_read_spec := when #target.triple is
    'aarch64-apple-darwin' then [signature: runtime_read_signature, target: #target.triple, bytes: RuntimeEncoding8.encode, inputs: [fd: [architecture: "aarch64", bank: "general", index: 9, width: 64], pointer: [architecture: "aarch64", bank: "general", index: 10, width: 64], size: [architecture: "aarch64", bank: "general", index: 11, width: 64]], outputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "aarch64", bank: "general", index: 1, width: 64], [architecture: "aarch64", bank: "general", index: 2, width: 64], [architecture: "aarch64", bank: "general", index: 16, width: 64], [architecture: "aarch64", bank: "flags", index: 0, width: 4]], reads: [], writes: [item0: [pointer: 1, length: 2]]]
    'aarch64-unknown-linux-gnu' then [signature: runtime_read_signature, target: #target.triple, bytes: RuntimeEncoding9.encode, inputs: [fd: [architecture: "aarch64", bank: "general", index: 9, width: 64], pointer: [architecture: "aarch64", bank: "general", index: 10, width: 64], size: [architecture: "aarch64", bank: "general", index: 11, width: 64]], outputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "aarch64", bank: "general", index: 1, width: 64], [architecture: "aarch64", bank: "general", index: 2, width: 64], [architecture: "aarch64", bank: "general", index: 8, width: 64], [architecture: "aarch64", bank: "flags", index: 0, width: 4]], reads: [], writes: [item0: [pointer: 1, length: 2]]]
    'x86_64-unknown-linux-gnu' then [signature: runtime_read_signature, target: #target.triple, bytes: RuntimeEncoding10.encode, inputs: [fd: [architecture: "x86_64", bank: "general", index: 7, width: 64], pointer: [architecture: "x86_64", bank: "general", index: 6, width: 64], size: [architecture: "x86_64", bank: "general", index: 2, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4]], reads: [], writes: [item0: [pointer: 1, length: 2]]]
    'x86_64-apple-darwin' then [signature: runtime_read_signature, target: #target.triple, bytes: RuntimeEncoding11.encode, inputs: [fd: [architecture: "x86_64", bank: "general", index: 7, width: 64], pointer: [architecture: "x86_64", bank: "general", index: 6, width: 64], size: [architecture: "x86_64", bank: "general", index: 2, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4]], reads: [], writes: [item0: [pointer: 1, length: 2]]]
    else [signature: runtime_read_signature, target: #target.triple, bytes: RuntimeEncoding11.encode, inputs: [fd: [architecture: "x86_64", bank: "general", index: 7, width: 64], pointer: [architecture: "x86_64", bank: "general", index: 6, width: 64], size: [architecture: "x86_64", bank: "general", index: 2, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4]], reads: [], writes: [item0: [pointer: 1, length: 2]]]

runtime_write_spec := when #target.triple is
    'aarch64-apple-darwin' then [signature: runtime_write_signature, target: #target.triple, bytes: RuntimeEncoding12.encode, inputs: [fd: [architecture: "aarch64", bank: "general", index: 9, width: 64], pointer: [architecture: "aarch64", bank: "general", index: 10, width: 64], size: [architecture: "aarch64", bank: "general", index: 11, width: 64]], outputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "aarch64", bank: "general", index: 1, width: 64], [architecture: "aarch64", bank: "general", index: 2, width: 64], [architecture: "aarch64", bank: "general", index: 16, width: 64], [architecture: "aarch64", bank: "flags", index: 0, width: 4]], reads: [item0: [pointer: 1, length: 2]], writes: []]
    'aarch64-unknown-linux-gnu' then [signature: runtime_write_signature, target: #target.triple, bytes: RuntimeEncoding13.encode, inputs: [fd: [architecture: "aarch64", bank: "general", index: 9, width: 64], pointer: [architecture: "aarch64", bank: "general", index: 10, width: 64], size: [architecture: "aarch64", bank: "general", index: 11, width: 64]], outputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "aarch64", bank: "general", index: 1, width: 64], [architecture: "aarch64", bank: "general", index: 2, width: 64], [architecture: "aarch64", bank: "general", index: 8, width: 64], [architecture: "aarch64", bank: "flags", index: 0, width: 4]], reads: [item0: [pointer: 1, length: 2]], writes: []]
    'x86_64-unknown-linux-gnu' then [signature: runtime_write_signature, target: #target.triple, bytes: RuntimeEncoding14.encode, inputs: [fd: [architecture: "x86_64", bank: "general", index: 7, width: 64], pointer: [architecture: "x86_64", bank: "general", index: 6, width: 64], size: [architecture: "x86_64", bank: "general", index: 2, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4]], reads: [item0: [pointer: 1, length: 2]], writes: []]
    'x86_64-apple-darwin' then [signature: runtime_write_signature, target: #target.triple, bytes: RuntimeEncoding15.encode, inputs: [fd: [architecture: "x86_64", bank: "general", index: 7, width: 64], pointer: [architecture: "x86_64", bank: "general", index: 6, width: 64], size: [architecture: "x86_64", bank: "general", index: 2, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4]], reads: [item0: [pointer: 1, length: 2]], writes: []]
    else [signature: runtime_write_signature, target: #target.triple, bytes: RuntimeEncoding15.encode, inputs: [fd: [architecture: "x86_64", bank: "general", index: 7, width: 64], pointer: [architecture: "x86_64", bank: "general", index: 6, width: 64], size: [architecture: "x86_64", bank: "general", index: 2, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4]], reads: [item0: [pointer: 1, length: 2]], writes: []]

runtime_read(fd RuntimeInt, pointer @RuntimeByte, size RuntimeWord) RuntimeInt: return #emit(runtime_read_spec, RuntimeReadInputs(fd: fd, pointer: pointer, size: size))
runtime_write(fd RuntimeInt, pointer @RuntimeByte, size RuntimeWord) RuntimeInt: return #emit(runtime_write_spec, RuntimeWriteInputs(fd: fd, pointer: pointer, size: size))


runtime_copy_pointer(value ref RuntimePointer) RuntimePointer: return deref value

runtime_number_spec := when #target.triple is
    'aarch64-apple-darwin' then [signature: runtime_number_operation_signature, target: #target.triple, bytes: RuntimeEncoding16.encode, inputs: [pointer: [architecture: "aarch64", bank: "general", index: 9, width: 64]], outputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "aarch64", bank: "general", index: 1, width: 64], [architecture: "aarch64", bank: "general", index: 2, width: 64], [architecture: "aarch64", bank: "general", index: 11, width: 64], [architecture: "aarch64", bank: "general", index: 12, width: 64], [architecture: "aarch64", bank: "general", index: 13, width: 64], [architecture: "aarch64", bank: "general", index: 14, width: 64], [architecture: "aarch64", bank: "general", index: 15, width: 64], [architecture: "aarch64", bank: "general", index: 16, width: 64], [architecture: "aarch64", bank: "flags", index: 0, width: 4]], reads: [], writes: []]
    'aarch64-unknown-linux-gnu' then [signature: runtime_number_operation_signature, target: #target.triple, bytes: RuntimeEncoding17.encode, inputs: [pointer: [architecture: "aarch64", bank: "general", index: 9, width: 64]], outputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "aarch64", bank: "general", index: 1, width: 64], [architecture: "aarch64", bank: "general", index: 2, width: 64], [architecture: "aarch64", bank: "general", index: 11, width: 64], [architecture: "aarch64", bank: "general", index: 12, width: 64], [architecture: "aarch64", bank: "general", index: 13, width: 64], [architecture: "aarch64", bank: "general", index: 14, width: 64], [architecture: "aarch64", bank: "general", index: 15, width: 64], [architecture: "aarch64", bank: "general", index: 8, width: 64], [architecture: "aarch64", bank: "flags", index: 0, width: 4]], reads: [], writes: []]
    'x86_64-unknown-linux-gnu' then [signature: runtime_number_operation_signature, target: #target.triple, bytes: RuntimeEncoding18.encode, inputs: [pointer: [architecture: "x86_64", bank: "general", index: 12, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 7, width: 64], [architecture: "x86_64", bank: "general", index: 6, width: 64], [architecture: "x86_64", bank: "general", index: 2, width: 64], [architecture: "x86_64", bank: "general", index: 8, width: 64], [architecture: "x86_64", bank: "general", index: 10, width: 64], [architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4], [architecture: "x86_64", bank: "general", index: 9, width: 64]], reads: [], writes: []]
    'x86_64-apple-darwin' then [signature: runtime_number_operation_signature, target: #target.triple, bytes: RuntimeEncoding19.encode, inputs: [pointer: [architecture: "x86_64", bank: "general", index: 12, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 7, width: 64], [architecture: "x86_64", bank: "general", index: 6, width: 64], [architecture: "x86_64", bank: "general", index: 2, width: 64], [architecture: "x86_64", bank: "general", index: 8, width: 64], [architecture: "x86_64", bank: "general", index: 10, width: 64], [architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4], [architecture: "x86_64", bank: "general", index: 9, width: 64]], reads: [], writes: []]
    else [signature: runtime_number_operation_signature, target: #target.triple, bytes: RuntimeEncoding19.encode, inputs: [pointer: [architecture: "x86_64", bank: "general", index: 12, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 7, width: 64], [architecture: "x86_64", bank: "general", index: 6, width: 64], [architecture: "x86_64", bank: "general", index: 2, width: 64], [architecture: "x86_64", bank: "general", index: 8, width: 64], [architecture: "x86_64", bank: "general", index: 10, width: 64], [architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4], [architecture: "x86_64", bank: "general", index: 9, width: 64]], reads: [], writes: []]

runtime_number RuntimeInt:
    pointer RuntimePointer: runtime_reserve(1)
    result RuntimeInt: runtime_number_operation(runtime_copy_pointer(pointer))
    runtime_release(pointer, 1)
    return result

runtime_print_spec := when #target.triple is
    'aarch64-apple-darwin' then [signature: runtime_print_operation_signature, target: #target.triple, bytes: RuntimeEncoding20.encode, inputs: [value: [architecture: "aarch64", bank: "general", index: 9, width: 64], pointer: [architecture: "aarch64", bank: "general", index: 10, width: 64]], outputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "aarch64", bank: "general", index: 1, width: 64], [architecture: "aarch64", bank: "general", index: 2, width: 64], [architecture: "aarch64", bank: "general", index: 11, width: 64], [architecture: "aarch64", bank: "general", index: 12, width: 64], [architecture: "aarch64", bank: "general", index: 13, width: 64], [architecture: "aarch64", bank: "general", index: 14, width: 64], [architecture: "aarch64", bank: "general", index: 15, width: 64], [architecture: "aarch64", bank: "general", index: 16, width: 64], [architecture: "aarch64", bank: "flags", index: 0, width: 4]], reads: [], writes: []]
    'aarch64-unknown-linux-gnu' then [signature: runtime_print_operation_signature, target: #target.triple, bytes: RuntimeEncoding21.encode, inputs: [value: [architecture: "aarch64", bank: "general", index: 9, width: 64], pointer: [architecture: "aarch64", bank: "general", index: 10, width: 64]], outputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "aarch64", bank: "general", index: 1, width: 64], [architecture: "aarch64", bank: "general", index: 2, width: 64], [architecture: "aarch64", bank: "general", index: 11, width: 64], [architecture: "aarch64", bank: "general", index: 12, width: 64], [architecture: "aarch64", bank: "general", index: 13, width: 64], [architecture: "aarch64", bank: "general", index: 14, width: 64], [architecture: "aarch64", bank: "general", index: 15, width: 64], [architecture: "aarch64", bank: "general", index: 8, width: 64], [architecture: "aarch64", bank: "flags", index: 0, width: 4]], reads: [], writes: []]
    'x86_64-unknown-linux-gnu' then [signature: runtime_print_operation_signature, target: #target.triple, bytes: RuntimeEncoding22.encode, inputs: [value: [architecture: "x86_64", bank: "general", index: 9, width: 64], pointer: [architecture: "x86_64", bank: "general", index: 12, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 7, width: 64], [architecture: "x86_64", bank: "general", index: 6, width: 64], [architecture: "x86_64", bank: "general", index: 2, width: 64], [architecture: "x86_64", bank: "general", index: 8, width: 64], [architecture: "x86_64", bank: "general", index: 10, width: 64], [architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4]], reads: [], writes: []]
    'x86_64-apple-darwin' then [signature: runtime_print_operation_signature, target: #target.triple, bytes: RuntimeEncoding23.encode, inputs: [value: [architecture: "x86_64", bank: "general", index: 9, width: 64], pointer: [architecture: "x86_64", bank: "general", index: 12, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 7, width: 64], [architecture: "x86_64", bank: "general", index: 6, width: 64], [architecture: "x86_64", bank: "general", index: 2, width: 64], [architecture: "x86_64", bank: "general", index: 8, width: 64], [architecture: "x86_64", bank: "general", index: 10, width: 64], [architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4]], reads: [], writes: []]
    else [signature: runtime_print_operation_signature, target: #target.triple, bytes: RuntimeEncoding23.encode, inputs: [value: [architecture: "x86_64", bank: "general", index: 9, width: 64], pointer: [architecture: "x86_64", bank: "general", index: 12, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 7, width: 64], [architecture: "x86_64", bank: "general", index: 6, width: 64], [architecture: "x86_64", bank: "general", index: 2, width: 64], [architecture: "x86_64", bank: "general", index: 8, width: 64], [architecture: "x86_64", bank: "general", index: 10, width: 64], [architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4]], reads: [], writes: []]

runtime_print(value RuntimeInt) RuntimeInt:
    pointer RuntimePointer: runtime_reserve(32)
    result RuntimeInt: runtime_print_operation(value, runtime_copy_pointer(pointer))
    runtime_release(pointer, 32)
    return result

runtime_copy_input_spec := when #target.triple is
    'aarch64-apple-darwin' then [signature: runtime_copy_input_operation_signature, target: #target.triple, bytes: RuntimeEncoding24.encode, inputs: [pointer: [architecture: "aarch64", bank: "general", index: 9, width: 64]], outputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "aarch64", bank: "general", index: 1, width: 64], [architecture: "aarch64", bank: "general", index: 2, width: 64], [architecture: "aarch64", bank: "general", index: 11, width: 64], [architecture: "aarch64", bank: "general", index: 12, width: 64], [architecture: "aarch64", bank: "general", index: 13, width: 64], [architecture: "aarch64", bank: "general", index: 14, width: 64], [architecture: "aarch64", bank: "general", index: 15, width: 64], [architecture: "aarch64", bank: "general", index: 16, width: 64], [architecture: "aarch64", bank: "flags", index: 0, width: 4]], reads: [], writes: []]
    'aarch64-unknown-linux-gnu' then [signature: runtime_copy_input_operation_signature, target: #target.triple, bytes: RuntimeEncoding25.encode, inputs: [pointer: [architecture: "aarch64", bank: "general", index: 9, width: 64]], outputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "aarch64", bank: "general", index: 1, width: 64], [architecture: "aarch64", bank: "general", index: 2, width: 64], [architecture: "aarch64", bank: "general", index: 11, width: 64], [architecture: "aarch64", bank: "general", index: 12, width: 64], [architecture: "aarch64", bank: "general", index: 13, width: 64], [architecture: "aarch64", bank: "general", index: 14, width: 64], [architecture: "aarch64", bank: "general", index: 15, width: 64], [architecture: "aarch64", bank: "general", index: 8, width: 64], [architecture: "aarch64", bank: "flags", index: 0, width: 4]], reads: [], writes: []]
    'x86_64-unknown-linux-gnu' then [signature: runtime_copy_input_operation_signature, target: #target.triple, bytes: RuntimeEncoding26.encode, inputs: [pointer: [architecture: "x86_64", bank: "general", index: 12, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 7, width: 64], [architecture: "x86_64", bank: "general", index: 6, width: 64], [architecture: "x86_64", bank: "general", index: 2, width: 64], [architecture: "x86_64", bank: "general", index: 8, width: 64], [architecture: "x86_64", bank: "general", index: 10, width: 64], [architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4], [architecture: "x86_64", bank: "general", index: 9, width: 64]], reads: [], writes: []]
    'x86_64-apple-darwin' then [signature: runtime_copy_input_operation_signature, target: #target.triple, bytes: RuntimeEncoding27.encode, inputs: [pointer: [architecture: "x86_64", bank: "general", index: 12, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 7, width: 64], [architecture: "x86_64", bank: "general", index: 6, width: 64], [architecture: "x86_64", bank: "general", index: 2, width: 64], [architecture: "x86_64", bank: "general", index: 8, width: 64], [architecture: "x86_64", bank: "general", index: 10, width: 64], [architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4], [architecture: "x86_64", bank: "general", index: 9, width: 64]], reads: [], writes: []]
    else [signature: runtime_copy_input_operation_signature, target: #target.triple, bytes: RuntimeEncoding27.encode, inputs: [pointer: [architecture: "x86_64", bank: "general", index: 12, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [[architecture: "x86_64", bank: "general", index: 7, width: 64], [architecture: "x86_64", bank: "general", index: 6, width: 64], [architecture: "x86_64", bank: "general", index: 2, width: 64], [architecture: "x86_64", bank: "general", index: 8, width: 64], [architecture: "x86_64", bank: "general", index: 10, width: 64], [architecture: "x86_64", bank: "general", index: 1, width: 64], [architecture: "x86_64", bank: "general", index: 11, width: 64], [architecture: "x86_64", bank: "flags", index: 0, width: 4], [architecture: "x86_64", bank: "general", index: 9, width: 64]], reads: [], writes: []]

runtime_copy_input RuntimeInt:
    pointer RuntimePointer: runtime_reserve(1)
    result RuntimeInt: runtime_copy_input_operation(runtime_copy_pointer(pointer))
    runtime_release(pointer, 1)
    return result

runtime_bits_spec := when #target.triple is
    'aarch64-apple-darwin' then [signature: runtime_word_bits_signature, target: #target.triple, bytes: RuntimeEncoding28.encode, inputs: [value: [architecture: "aarch64", bank: "general", index: 9, width: 64]], outputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], clobbers: [], reads: [], writes: []]
    'aarch64-unknown-linux-gnu' then [signature: runtime_word_bits_signature, target: #target.triple, bytes: RuntimeEncoding29.encode, inputs: [value: [architecture: "aarch64", bank: "general", index: 9, width: 64]], outputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], clobbers: [], reads: [], writes: []]
    'x86_64-unknown-linux-gnu' then [signature: runtime_word_bits_signature, target: #target.triple, bytes: RuntimeEncoding30.encode, inputs: [value: [architecture: "x86_64", bank: "general", index: 9, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [], reads: [], writes: []]
    'x86_64-apple-darwin' then [signature: runtime_word_bits_signature, target: #target.triple, bytes: RuntimeEncoding31.encode, inputs: [value: [architecture: "x86_64", bank: "general", index: 9, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [], reads: [], writes: []]
    else [signature: runtime_word_bits_signature, target: #target.triple, bytes: RuntimeEncoding31.encode, inputs: [value: [architecture: "x86_64", bank: "general", index: 9, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [], reads: [], writes: []]

runtime_signed_bits_code := when #target.triple is
    'aarch64-apple-darwin' then [signature: runtime_signed_bits_signature, target: #target.triple, bytes: RuntimeEncoding28.encode, inputs: [value: [architecture: "aarch64", bank: "general", index: 9, width: 64]], outputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], clobbers: [], reads: [], writes: []]
    'aarch64-unknown-linux-gnu' then [signature: runtime_signed_bits_signature, target: #target.triple, bytes: RuntimeEncoding29.encode, inputs: [value: [architecture: "aarch64", bank: "general", index: 9, width: 64]], outputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], clobbers: [], reads: [], writes: []]
    'x86_64-unknown-linux-gnu' then [signature: runtime_signed_bits_signature, target: #target.triple, bytes: RuntimeEncoding30.encode, inputs: [value: [architecture: "x86_64", bank: "general", index: 9, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [], reads: [], writes: []]
    'x86_64-apple-darwin' then [signature: runtime_signed_bits_signature, target: #target.triple, bytes: RuntimeEncoding31.encode, inputs: [value: [architecture: "x86_64", bank: "general", index: 9, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [], reads: [], writes: []]
    else [signature: runtime_signed_bits_signature, target: #target.triple, bytes: RuntimeEncoding31.encode, inputs: [value: [architecture: "x86_64", bank: "general", index: 9, width: 64]], outputs: [value: [architecture: "x86_64", bank: "general", index: 0, width: 64]], clobbers: [], reads: [], writes: []]

runtime_word_bits(value RuntimeInt) RuntimeWord: return #emit(runtime_bits_spec, RuntimeWordBitsInputs(value: value))
runtime_signed_bits(value RuntimeWord) RuntimeInt: return #emit(runtime_signed_bits_code, RuntimeSignedBitsInputs(value: value))

RuntimeEncodingByte is in 0 to 255
RuntimeEncodingByte has
    finish(self):
        return

RuntimeEncoding0 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 40): return [0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 225 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 98 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 67 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 130 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 4 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 146 as RuntimeEncodingByte, 5 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 176 as RuntimeEncodingByte, 24 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 16 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 212 as RuntimeEncodingByte, 67 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 84 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte]

RuntimeEncoding1 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 44): return [0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 225 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 98 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 67 as RuntimeEncodingByte, 4 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 4 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 146 as RuntimeEncodingByte, 5 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 200 as RuntimeEncodingByte, 27 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 212 as RuntimeEncodingByte, 31 as RuntimeEncodingByte, 252 as RuntimeEncodingByte, 63 as RuntimeEncodingByte, 177 as RuntimeEncodingByte, 67 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 84 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte]

RuntimeEncoding2 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 42): return [72 as RuntimeEncodingByte, 199 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 49 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 186 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 65 as RuntimeEncodingByte, 186 as RuntimeEncodingByte, 34 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 73 as RuntimeEncodingByte, 199 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 69 as RuntimeEncodingByte, 49 as RuntimeEncodingByte, 201 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 5 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 61 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 240 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 114 as RuntimeEncodingByte, 2 as RuntimeEncodingByte, 49 as RuntimeEncodingByte, 192 as RuntimeEncodingByte]

RuntimeEncoding3 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 36): return [72 as RuntimeEncodingByte, 199 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 197 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 2 as RuntimeEncodingByte, 49 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 186 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 65 as RuntimeEncodingByte, 186 as RuntimeEncodingByte, 2 as RuntimeEncodingByte, 16 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 73 as RuntimeEncodingByte, 199 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 69 as RuntimeEncodingByte, 49 as RuntimeEncodingByte, 201 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 5 as RuntimeEncodingByte, 115 as RuntimeEncodingByte, 2 as RuntimeEncodingByte, 49 as RuntimeEncodingByte, 192 as RuntimeEncodingByte]

RuntimeEncoding4 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 20): return [224 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 225 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 10 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 48 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 20 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 212 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 52 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 218 as RuntimeEncodingByte]

RuntimeEncoding5 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 16): return [224 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 225 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 10 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 232 as RuntimeEncodingByte, 26 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 212 as RuntimeEncodingByte]

RuntimeEncoding6 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 9): return [72 as RuntimeEncodingByte, 199 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 11 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 5 as RuntimeEncodingByte]

RuntimeEncoding7 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 9): return [72 as RuntimeEncodingByte, 199 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 73 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 2 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 5 as RuntimeEncodingByte]

RuntimeEncoding8 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 20): return [224 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 225 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 10 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 226 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 11 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 112 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 16 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 212 as RuntimeEncodingByte]

RuntimeEncoding9 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 20): return [224 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 225 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 10 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 226 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 11 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 232 as RuntimeEncodingByte, 7 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 212 as RuntimeEncodingByte]

RuntimeEncoding10 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 9): return [72 as RuntimeEncodingByte, 199 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 5 as RuntimeEncodingByte]

RuntimeEncoding11 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 9): return [72 as RuntimeEncodingByte, 199 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 2 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 5 as RuntimeEncodingByte]

RuntimeEncoding12 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 20): return [224 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 225 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 10 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 226 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 11 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 144 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 16 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 212 as RuntimeEncodingByte]

RuntimeEncoding13 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 20): return [224 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 225 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 10 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 226 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 11 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 8 as RuntimeEncodingByte, 8 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 212 as RuntimeEncodingByte]

RuntimeEncoding14 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 9): return [72 as RuntimeEncodingByte, 199 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 5 as RuntimeEncodingByte]

RuntimeEncoding15 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 9): return [72 as RuntimeEncodingByte, 199 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 4 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 2 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 5 as RuntimeEncodingByte]

RuntimeEncoding16 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 104): return [12 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 45 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 14 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 225 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 34 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 112 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 16 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 212 as RuntimeEncodingByte, 31 as RuntimeEncodingByte, 4 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 241 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 2 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 84 as RuntimeEncodingByte, 47 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 64 as RuntimeEncodingByte, 57 as RuntimeEncodingByte, 14 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 181 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 129 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 113 as RuntimeEncodingByte, 201 as RuntimeEncodingByte, 254 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 84 as RuntimeEncodingByte, 46 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 181 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 113 as RuntimeEncodingByte, 97 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 84 as RuntimeEncodingByte, 13 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 146 as RuntimeEncodingByte, 241 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 23 as RuntimeEncodingByte, 239 as RuntimeEncodingByte, 193 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 81 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 37 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 113 as RuntimeEncodingByte, 136 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 84 as RuntimeEncodingByte, 75 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 140 as RuntimeEncodingByte, 61 as RuntimeEncodingByte, 11 as RuntimeEncodingByte, 155 as RuntimeEncodingByte, 235 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 23 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 125 as RuntimeEncodingByte, 13 as RuntimeEncodingByte, 155 as RuntimeEncodingByte]

RuntimeEncoding17 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 104): return [12 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 45 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 14 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 225 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 34 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 232 as RuntimeEncodingByte, 7 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 212 as RuntimeEncodingByte, 31 as RuntimeEncodingByte, 4 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 241 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 2 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 84 as RuntimeEncodingByte, 47 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 64 as RuntimeEncodingByte, 57 as RuntimeEncodingByte, 14 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 181 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 129 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 113 as RuntimeEncodingByte, 201 as RuntimeEncodingByte, 254 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 84 as RuntimeEncodingByte, 46 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 181 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 113 as RuntimeEncodingByte, 97 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 84 as RuntimeEncodingByte, 13 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 146 as RuntimeEncodingByte, 241 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 23 as RuntimeEncodingByte, 239 as RuntimeEncodingByte, 193 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 81 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 37 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 113 as RuntimeEncodingByte, 136 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 84 as RuntimeEncodingByte, 75 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 140 as RuntimeEncodingByte, 61 as RuntimeEncodingByte, 11 as RuntimeEncodingByte, 155 as RuntimeEncodingByte, 235 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 23 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 125 as RuntimeEncodingByte, 13 as RuntimeEncodingByte, 155 as RuntimeEncodingByte]

RuntimeEncoding18 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 96): return [69 as RuntimeEncodingByte, 49 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 73 as RuntimeEncodingByte, 199 as RuntimeEncodingByte, 193 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 69 as RuntimeEncodingByte, 49 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 49 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 76 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 230 as RuntimeEncodingByte, 186 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 184 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 5 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 131 as RuntimeEncodingByte, 248 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 117 as RuntimeEncodingByte, 53 as RuntimeEncodingByte, 65 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 182 as RuntimeEncodingByte, 20 as RuntimeEncodingByte, 36 as RuntimeEncodingByte, 77 as RuntimeEncodingByte, 133 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 117 as RuntimeEncodingByte, 26 as RuntimeEncodingByte, 131 as RuntimeEncodingByte, 250 as RuntimeEncodingByte, 32 as RuntimeEncodingByte, 118 as RuntimeEncodingByte, 218 as RuntimeEncodingByte, 73 as RuntimeEncodingByte, 199 as RuntimeEncodingByte, 194 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 131 as RuntimeEncodingByte, 250 as RuntimeEncodingByte, 45 as RuntimeEncodingByte, 117 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 73 as RuntimeEncodingByte, 199 as RuntimeEncodingByte, 193 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 235 as RuntimeEncodingByte, 197 as RuntimeEncodingByte, 131 as RuntimeEncodingByte, 234 as RuntimeEncodingByte, 48 as RuntimeEncodingByte, 131 as RuntimeEncodingByte, 250 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 119 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 77 as RuntimeEncodingByte, 107 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 10 as RuntimeEncodingByte, 73 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 208 as RuntimeEncodingByte, 235 as RuntimeEncodingByte, 180 as RuntimeEncodingByte, 76 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 73 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 175 as RuntimeEncodingByte, 193 as RuntimeEncodingByte]

RuntimeEncoding19 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 96): return [69 as RuntimeEncodingByte, 49 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 73 as RuntimeEncodingByte, 199 as RuntimeEncodingByte, 193 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 69 as RuntimeEncodingByte, 49 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 49 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 76 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 230 as RuntimeEncodingByte, 186 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 184 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 2 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 5 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 131 as RuntimeEncodingByte, 248 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 117 as RuntimeEncodingByte, 53 as RuntimeEncodingByte, 65 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 182 as RuntimeEncodingByte, 20 as RuntimeEncodingByte, 36 as RuntimeEncodingByte, 77 as RuntimeEncodingByte, 133 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 117 as RuntimeEncodingByte, 26 as RuntimeEncodingByte, 131 as RuntimeEncodingByte, 250 as RuntimeEncodingByte, 32 as RuntimeEncodingByte, 118 as RuntimeEncodingByte, 218 as RuntimeEncodingByte, 73 as RuntimeEncodingByte, 199 as RuntimeEncodingByte, 194 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 131 as RuntimeEncodingByte, 250 as RuntimeEncodingByte, 45 as RuntimeEncodingByte, 117 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 73 as RuntimeEncodingByte, 199 as RuntimeEncodingByte, 193 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 235 as RuntimeEncodingByte, 197 as RuntimeEncodingByte, 131 as RuntimeEncodingByte, 234 as RuntimeEncodingByte, 48 as RuntimeEncodingByte, 131 as RuntimeEncodingByte, 250 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 119 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 77 as RuntimeEncodingByte, 107 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 10 as RuntimeEncodingByte, 73 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 208 as RuntimeEncodingByte, 235 as RuntimeEncodingByte, 180 as RuntimeEncodingByte, 76 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 73 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 175 as RuntimeEncodingByte, 193 as RuntimeEncodingByte]

RuntimeEncoding20 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 88): return [75 as RuntimeEncodingByte, 125 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 145 as RuntimeEncodingByte, 79 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 82 as RuntimeEncodingByte, 111 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 57 as RuntimeEncodingByte, 63 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 241 as RuntimeEncodingByte, 44 as RuntimeEncodingByte, 165 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 218 as RuntimeEncodingByte, 77 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 142 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 205 as RuntimeEncodingByte, 154 as RuntimeEncodingByte, 207 as RuntimeEncodingByte, 177 as RuntimeEncodingByte, 13 as RuntimeEncodingByte, 155 as RuntimeEncodingByte, 239 as RuntimeEncodingByte, 193 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 17 as RuntimeEncodingByte, 111 as RuntimeEncodingByte, 253 as RuntimeEncodingByte, 31 as RuntimeEncodingByte, 56 as RuntimeEncodingByte, 236 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 14 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 108 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 181 as RuntimeEncodingByte, 105 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 248 as RuntimeEncodingByte, 182 as RuntimeEncodingByte, 175 as RuntimeEncodingByte, 5 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 82 as RuntimeEncodingByte, 111 as RuntimeEncodingByte, 253 as RuntimeEncodingByte, 31 as RuntimeEncodingByte, 56 as RuntimeEncodingByte, 32 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 225 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 11 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 66 as RuntimeEncodingByte, 129 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 145 as RuntimeEncodingByte, 66 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 11 as RuntimeEncodingByte, 203 as RuntimeEncodingByte, 144 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 16 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 212 as RuntimeEncodingByte, 224 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 170 as RuntimeEncodingByte]

RuntimeEncoding21 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 88): return [75 as RuntimeEncodingByte, 125 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 145 as RuntimeEncodingByte, 79 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 82 as RuntimeEncodingByte, 111 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 57 as RuntimeEncodingByte, 63 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 241 as RuntimeEncodingByte, 44 as RuntimeEncodingByte, 165 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 218 as RuntimeEncodingByte, 77 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 142 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 205 as RuntimeEncodingByte, 154 as RuntimeEncodingByte, 207 as RuntimeEncodingByte, 177 as RuntimeEncodingByte, 13 as RuntimeEncodingByte, 155 as RuntimeEncodingByte, 239 as RuntimeEncodingByte, 193 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 17 as RuntimeEncodingByte, 111 as RuntimeEncodingByte, 253 as RuntimeEncodingByte, 31 as RuntimeEncodingByte, 56 as RuntimeEncodingByte, 236 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 14 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 108 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 181 as RuntimeEncodingByte, 105 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 248 as RuntimeEncodingByte, 182 as RuntimeEncodingByte, 175 as RuntimeEncodingByte, 5 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 82 as RuntimeEncodingByte, 111 as RuntimeEncodingByte, 253 as RuntimeEncodingByte, 31 as RuntimeEncodingByte, 56 as RuntimeEncodingByte, 32 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 225 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 11 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 66 as RuntimeEncodingByte, 129 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 145 as RuntimeEncodingByte, 66 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 11 as RuntimeEncodingByte, 203 as RuntimeEncodingByte, 8 as RuntimeEncodingByte, 8 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 212 as RuntimeEncodingByte, 224 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 170 as RuntimeEncodingByte]

RuntimeEncoding22 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 79): return [77 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 200 as RuntimeEncodingByte, 76 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 200 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 133 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 121 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 247 as RuntimeEncodingByte, 216 as RuntimeEncodingByte, 73 as RuntimeEncodingByte, 141 as RuntimeEncodingByte, 116 as RuntimeEncodingByte, 36 as RuntimeEncodingByte, 31 as RuntimeEncodingByte, 198 as RuntimeEncodingByte, 6 as RuntimeEncodingByte, 10 as RuntimeEncodingByte, 185 as RuntimeEncodingByte, 10 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 49 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 247 as RuntimeEncodingByte, 241 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 194 as RuntimeEncodingByte, 48 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 206 as RuntimeEncodingByte, 136 as RuntimeEncodingByte, 22 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 133 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 117 as RuntimeEncodingByte, 238 as RuntimeEncodingByte, 77 as RuntimeEncodingByte, 133 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 121 as RuntimeEncodingByte, 6 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 206 as RuntimeEncodingByte, 198 as RuntimeEncodingByte, 6 as RuntimeEncodingByte, 45 as RuntimeEncodingByte, 73 as RuntimeEncodingByte, 141 as RuntimeEncodingByte, 84 as RuntimeEncodingByte, 36 as RuntimeEncodingByte, 32 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 41 as RuntimeEncodingByte, 242 as RuntimeEncodingByte, 191 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 184 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 5 as RuntimeEncodingByte, 76 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 192 as RuntimeEncodingByte]

RuntimeEncoding23 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 79): return [77 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 200 as RuntimeEncodingByte, 76 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 200 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 133 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 121 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 247 as RuntimeEncodingByte, 216 as RuntimeEncodingByte, 73 as RuntimeEncodingByte, 141 as RuntimeEncodingByte, 116 as RuntimeEncodingByte, 36 as RuntimeEncodingByte, 31 as RuntimeEncodingByte, 198 as RuntimeEncodingByte, 6 as RuntimeEncodingByte, 10 as RuntimeEncodingByte, 185 as RuntimeEncodingByte, 10 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 49 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 247 as RuntimeEncodingByte, 241 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 194 as RuntimeEncodingByte, 48 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 206 as RuntimeEncodingByte, 136 as RuntimeEncodingByte, 22 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 133 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 117 as RuntimeEncodingByte, 238 as RuntimeEncodingByte, 77 as RuntimeEncodingByte, 133 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 121 as RuntimeEncodingByte, 6 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 206 as RuntimeEncodingByte, 198 as RuntimeEncodingByte, 6 as RuntimeEncodingByte, 45 as RuntimeEncodingByte, 73 as RuntimeEncodingByte, 141 as RuntimeEncodingByte, 84 as RuntimeEncodingByte, 36 as RuntimeEncodingByte, 32 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 41 as RuntimeEncodingByte, 242 as RuntimeEncodingByte, 191 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 184 as RuntimeEncodingByte, 4 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 2 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 5 as RuntimeEncodingByte, 76 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 192 as RuntimeEncodingByte]

RuntimeEncoding24 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 56): return [0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 225 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 34 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 112 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 16 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 212 as RuntimeEncodingByte, 31 as RuntimeEncodingByte, 4 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 241 as RuntimeEncodingByte, 225 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 84 as RuntimeEncodingByte, 32 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 225 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 34 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 144 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 16 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 212 as RuntimeEncodingByte, 244 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 23 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte]

RuntimeEncoding25 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 56): return [0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 225 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 34 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 232 as RuntimeEncodingByte, 7 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 212 as RuntimeEncodingByte, 31 as RuntimeEncodingByte, 4 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 241 as RuntimeEncodingByte, 225 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 84 as RuntimeEncodingByte, 32 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 225 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 34 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 8 as RuntimeEncodingByte, 8 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 212 as RuntimeEncodingByte, 244 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 23 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 128 as RuntimeEncodingByte, 210 as RuntimeEncodingByte]

RuntimeEncoding26 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 47): return [49 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 76 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 230 as RuntimeEncodingByte, 186 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 184 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 5 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 131 as RuntimeEncodingByte, 248 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 117 as RuntimeEncodingByte, 22 as RuntimeEncodingByte, 191 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 76 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 230 as RuntimeEncodingByte, 186 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 184 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 5 as RuntimeEncodingByte, 235 as RuntimeEncodingByte, 211 as RuntimeEncodingByte, 49 as RuntimeEncodingByte, 192 as RuntimeEncodingByte]

RuntimeEncoding27 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 47): return [49 as RuntimeEncodingByte, 255 as RuntimeEncodingByte, 76 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 230 as RuntimeEncodingByte, 186 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 184 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 2 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 5 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 131 as RuntimeEncodingByte, 248 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 117 as RuntimeEncodingByte, 22 as RuntimeEncodingByte, 191 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 76 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 230 as RuntimeEncodingByte, 186 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 184 as RuntimeEncodingByte, 4 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 2 as RuntimeEncodingByte, 15 as RuntimeEncodingByte, 5 as RuntimeEncodingByte, 235 as RuntimeEncodingByte, 211 as RuntimeEncodingByte, 49 as RuntimeEncodingByte, 192 as RuntimeEncodingByte]

RuntimeEncoding28 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 4): return [224 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 170 as RuntimeEncodingByte]

RuntimeEncoding29 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 4): return [224 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 9 as RuntimeEncodingByte, 170 as RuntimeEncodingByte]

RuntimeEncoding30 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 3): return [76 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 200 as RuntimeEncodingByte]

RuntimeEncoding31 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 3): return [76 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 200 as RuntimeEncodingByte]

runtime_number_operation(pointer RuntimePointer) RuntimeInt: return #emit(runtime_number_spec, RuntimeNumberOperationInputs(pointer: pointer))

runtime_print_operation(value RuntimeInt, pointer RuntimePointer) RuntimeInt: return #emit(runtime_print_spec, RuntimePrintOperationInputs(value: value, pointer: pointer))

runtime_copy_input_operation(pointer RuntimePointer) RuntimeInt: return #emit(runtime_copy_input_spec, RuntimeCopyInputOperationInputs(pointer: pointer))

#fixed_array(element, count)
RuntimeEncodingBytes(element, count BigInt) has
    finish(self):
        return
"#;

pub const INDEXED_COPY: &str = r#"Byte is in 0 to 255
Byte has
    finish(self):
        return
    copy(self ref) Self: return deref self
replace(previous Byte, next Byte) Byte: return next
copy(source ref Buffer(Byte,4), destination mut Buffer(Byte,4), count is in 0 to 4):
    when count is 0 then
        return
    for index in 0 to count - 1
        destination.[index] :: replace(destination.[index], source.[index].copy)
    return
forward(source ref Buffer(Byte,4), destination mut Buffer(Byte,4), count is in 0 to 4): return copy(source, destination, count)
run(count is in 0 to 4) Buffer(Byte,4):
    source Buffer(Byte,4): [3,17,128,255]
    destination Buffer(Byte,4): [41;4]
    forward(source, destination, count)
    return destination

read_copied(count is in 0 to 4, index is in 0 to 3) Byte: return run(count).[index]

#fixed_array(element,count)
Buffer(element,count BigInt) has
    finish(self):
        return
"#;
