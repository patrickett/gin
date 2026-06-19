//! Package hover on `int.gin` must not pick up `'x86_64'` from other modules.

use analysis::{CacheEngine, QueryEngine};
use crossbeam_channel::unbounded;
use test_fixtures::TempPackage;

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

const POINTER_GIN: &str = r#"use '../target/'.target
use BigInt, Int

Pointer(x) is @x

--- The pointer-width unsigned integer type (8 bytes on 64-bit, 4 bytes on 32-bit).
PointerSize is when target.arch is
    'x86_64' or 'arm64' then BigInt
    'wasm32'            then Int
"#;

#[test]
fn package_hover_signed_int_range_not_arch_literal() {
    let int_src = INT_GIN;
    let pointer_src = POINTER_GIN;
    let pkg = TempPackage::new("int_hover");
    pkg.write_flask("core");
    let int_path = pkg.write("primitive/int.gin", int_src);
    let pointer_path = pkg.write("primitive/pointer.gin", pointer_src);

    let line = "SignedInt is in -2147483648...2147483647";
    let byte = int_src.find(line).expect("SignedInt line") as u32
        + line.find("2147483647").expect("range end") as u32;

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(int_path.clone()).unwrap();
    engine.add_file(pointer_path).unwrap();

    let hover = engine
        .hover(&int_path, byte)
        .expect("hover on SignedInt range")
        .markdown;

    assert_eq!(
        hover,
        "```gin\ncore.primitive\n```\n\n```gin\nSignedInt is in -2147483648...2147483647\n```\n---\n\nThe 32-bit signed integer type.",
        "expected full hover markdown"
    );
}
