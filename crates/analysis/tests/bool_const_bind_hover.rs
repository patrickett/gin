//! Hover on `false := Bool.False` / `true := Bool.True` in `modules/gin_core/primitive/bool.gin`.
//!
//! These constant value binds have no explicit type annotation; their "type" is
//! the const RHS variant. The LSP hover popup must surface that variant so the
//! user sees `false Bool.False` (not just `false`).

use analysis::{HoverContent, PackageCache};
use test_fixtures::TempPackage;
use test_fixtures::gin_core::BOOL_GIN;

#[test]
fn hover_false_shows_bool_false_variant() {
    let bool_src = BOOL_GIN;
    let pkg = TempPackage::new("bool_false_hover");
    pkg.write_flask("core");
    let path = pkg.write("primitive/bool.gin", bool_src);

    let engine = PackageCache::for_test();
    engine.add_file(path.clone()).unwrap();

    let byte = bool_src.find("false :=").expect("`false :=` in bool.gin") as u32;
    let HoverContent { markdown, .. } = engine
        .hover(&path, byte)
        .expect("hover on `false := Bool.False`");

    assert_eq!(
        markdown,
        "\
```gin
core.primitive
```

```gin
false Bool.False
```"
    );
}

#[test]
fn hover_true_shows_bool_true_variant() {
    let bool_src = BOOL_GIN;
    let pkg = TempPackage::new("bool_true_hover");
    pkg.write_flask("core");
    let path = pkg.write("primitive/bool.gin", bool_src);

    let engine = PackageCache::for_test();
    engine.add_file(path.clone()).unwrap();

    let byte = bool_src.find("true  :=").expect("`true  :=` in bool.gin") as u32;
    let HoverContent { markdown, .. } = engine
        .hover(&path, byte)
        .expect("hover on `true := Bool.True`");

    assert_eq!(
        markdown,
        "\
```gin
core.primitive
```

```gin
true Bool.True
```"
    );
}
