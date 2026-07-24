//! Hover on `false := Bool.False` / `true := Bool.True` in `modules/gin_core/primitive/bool.gin`.
//!
//! These constant value binds have no explicit type annotation; their "type" is
//! the const RHS variant. The LSP hover popup must surface that variant so the
//! user sees `false Bool.False` (not just `false`).

use analysis::{CacheEngine, HoverContent, QueryEngine};
use crossbeam_channel::unbounded;
use test_fixtures::TempPackage;

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

#[test]
fn hover_false_shows_bool_false_variant() {
    let bool_src = BOOL_GIN;
    let pkg = TempPackage::new("bool_false_hover");
    pkg.write_flask("core");
    let path = pkg.write("primitive/bool.gin", bool_src);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
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

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
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
