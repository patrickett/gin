//! Hover on `compute_size` in sized.gin must not show unrelated `Range` tag.

use analysis::PackageCache;
use test_fixtures::TempPackage;
use test_fixtures::gin_core::{
    BOOL_GIN, INT_GIN, LIST_GIN, POINTER_GIN, SIZED_GIN, STRING_GIN, TYPE_GIN,
};

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
Range(x) has Bounded
    start x
    end x
    Bounded.min: start
    Bounded.max: end

--- create a new range
Range(x).new(start x, end x) Range(x): (start, end)
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
];

fn write_marker_package(pkg: &TempPackage) {
    for (content, rel) in MARKER_PACKAGE {
        pkg.write(rel, content);
    }
}

#[test]
fn package_hover_compute_size_name_not_range_tag() {
    let sized_src = SIZED_GIN;
    let pkg = TempPackage::new("sized_compute_hover");
    pkg.write_flask("core");
    write_marker_package(&pkg);
    let sized_path = pkg.root.join("marker/sized.gin");

    let bind_line = "compute_size(x ReflectionType) Size := when x is";
    let byte = sized_src.find(bind_line).expect("compute_size bind") as u32
        + bind_line.find("compute_size").expect("name") as u32;

    let engine = PackageCache::for_test();
    for (_, rel) in MARKER_PACKAGE {
        engine.add_file(pkg.root.join(rel)).unwrap();
    }

    let hover = engine
        .hover(&sized_path, byte)
        .expect("hover on compute_size")
        .markdown;

    assert_eq!(
        hover, "```gin\ncore.marker\n```\n\n```gin\ncompute_size(x ReflectionType) Size\n```",
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

    let bind_line = "compute_size(x ReflectionType) Size := when x is";
    let byte = sized_src.find(bind_line).expect("compute_size bind") as u32
        + bind_line.find("compute_size").expect("name") as u32;

    let engine = PackageCache::for_test();
    engine.add_file(sized_path.clone()).unwrap();
    engine.add_file(range_path).unwrap();

    let hover = engine
        .hover(&sized_path, byte)
        .expect("hover on compute_size")
        .markdown;

    assert_eq!(
        hover, "```gin\ncore.marker\n```\n\n```gin\ncompute_size(x ReflectionType) Size\n```",
        "def name must win over any declare-span overlap"
    );
}
