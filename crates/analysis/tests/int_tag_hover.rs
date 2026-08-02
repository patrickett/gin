//! Package hover on `int.gin` must not pick up `'x86_64'` from other modules.

use analysis::PackageCache;
use test_fixtures::TempPackage;
use test_fixtures::gin_core::{INT_GIN, POINTER_GIN};

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

    let engine = PackageCache::for_test();
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
