//! Contract tests for unified [`QueryEngine::hover`] (parse → import → typed).

use analysis::{CacheEngine, HoverContent, QueryEngine};
use ast::source::SourceExt;
use crossbeam_channel::unbounded;
use parser::query::SourceParseExt;
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

const DEFAULT_GIN: &str = r#"--- Provides a default value for a type.
Default(value) has default value
"#;

const MAYBE_GIN: &str = r#"use core.Happy

Maybe(x) is    --- Used to represent values that may or may not be present.
    Some(x) or --- Has some value `x`
    None       --- Has no value
Maybe.Happy has value: Maybe.Some
"#;

const TYPE_GIN: &str = r#"use '../primitive/'.(BigInt, Bool, List)
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
--- User-written `Type.Reflectable has ...` is a compile error.
Reflectable has shape Type
"#;

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

const LIST_GIN: &str = r#"use Pointer, PointerSize

--- A homogeneous list type.
--- List literals use bracket syntax: ['linux', 'macos'].
--- Generic over element type: `List(x)`.
List(x) has pointer Pointer(x), length PointerSize
"#;

const STRING_GIN: &str = r#"use core.primitive.(Byte, List)

--- Canonical `String` type used for printing/codegen.
String has bytes List(Byte)

ToString has to_string String
"#;

#[test]
fn hover_use_keyword_via_engine() {
    let path = std::path::PathBuf::from("/tmp/hover_use_keyword.gin");
    let source = "use core.io\n\nmain:\n    return 0\n";

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();
    engine.set_contents(&path, source.to_string());

    let byte = source.find("use").expect("`use` in source") as u32;
    let HoverContent {
        markdown,
        byte_range,
    } = engine.hover(&path, byte).expect("hover on `use` keyword");

    let expected_markdown = "`use` \u{2014} Import items from other modules into scope";
    assert_eq!(
        markdown, expected_markdown,
        "expected use-keyword docs, got:\n{markdown}"
    );
    assert!(
        byte_range.is_some(),
        "keyword hover should include a highlight range"
    );
}

#[test]
fn hover_import_symbol_matches_definition_hover() {
    let bool_gin = BOOL_GIN;
    let pkg = TempPackage::new("hover_unified");
    pkg.write(
        "flask.jsonc",
        r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": {
    "core": { "path": "core" }
  }
}
"#,
    );
    pkg.write(
        "core/flask.jsonc",
        r#"{"name":"core","version":"0.0.0","authors":[]}"#,
    );
    pkg.write("core/bool.gin", bool_gin);
    let path = pkg.write("main.gin", "use core.true\n\nmain:\n    true\nreturn\n");

    let main_source = std::fs::read_to_string(&path).unwrap();
    let true_byte = main_source.find("true").expect("`true` in use path") as u32;

    let bool_po = bool_gin.parse_source_full();
    let true_def_byte = bool_gin.find("true  :=").unwrap();
    let bool_typed =
        typecheck::transform::transform_file(bool_po.ast.clone(), typecheck::FileId(0));
    let (line, character) = bool_gin.byte_offset_to_position(true_def_byte);
    let expected = bool_typed
        .hover_at(bool_gin, line, character)
        .expect("hover on `true` in bool.gin");

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    let HoverContent { markdown, .. } = engine
        .hover(&path, true_byte)
        .expect("hover on imported `true`");
    assert_eq!(
        markdown, expected,
        "import hover markdown must match definition hover"
    );
}

// ── Bind hover regression: every bind kind must produce non-empty hover ──

fn hover_at(source: &str, needle: &str, fixture: &str) -> HoverContent {
    let pkg = TempPackage::new(&format!("bind_hover_{fixture}"));
    pkg.write_flask("bind_hover");
    let path = pkg.write("main.gin", source);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    let byte = source
        .find(needle)
        .unwrap_or_else(|| panic!("`{needle}` in source")) as u32;

    engine
        .hover(&path, byte)
        .unwrap_or_else(|| panic!("hover on `{needle}` in {fixture}: returned None"))
}

#[test]
fn hover_local_constant_bind() {
    let source = "main:\n    x := 1\n    return 0\n";
    let HoverContent { markdown, .. } = hover_at(source, "x :=", "local_constant");
    assert_eq!(markdown, "x 1\n---\n\n");
}

#[test]
fn hover_local_rebindable_bind() {
    let source = "main:\n    x: 1\n    return 0\n";
    let HoverContent { markdown, .. } = hover_at(source, "x: 1", "local_rebindable");
    // `:` binds show the const value when available.
    assert_eq!(markdown, "x 1\n---\n\n");
}

#[test]
fn hover_top_level_constant_bind() {
    let source = "two := 1 + 1\n";
    let HoverContent { markdown, .. } = hover_at(source, "two", "top_level_constant");
    assert_eq!(markdown, "```gin\nbind_hover\n```\n\n```gin\ntwo 2\n```");
}

#[test]
fn hover_function_bind() {
    let source = "consume(x) Int := x\n";
    let HoverContent { markdown, .. } = hover_at(source, "consume", "function");
    assert_eq!(
        markdown,
        "```gin\nbind_hover\n```\n\n```gin\nconsume(x) Int\n```"
    );
}

#[test]
fn hover_main_function_bind() {
    let source = "main:\n    return 0\n";
    let HoverContent { markdown, .. } = hover_at(source, "main", "main_block");
    assert_eq!(markdown, "```gin\nbind_hover\n```\n\n```gin\nmain\n```");
}

#[test]
fn hover_single_line_main_bind() {
    let source = "main: return 0\n";
    let HoverContent { markdown, .. } = hover_at(source, "main", "main_single_line");
    assert_eq!(markdown, "```gin\nbind_hover\n```\n\n```gin\nmain\n```");
}

#[test]
fn hover_default_in_core_default_shows_core_default_module_path() {
    let default_src = DEFAULT_GIN;
    let pkg = TempPackage::new("core_default_hover");
    pkg.write_nested_flask("core", "core");
    let path = pkg.write("core/default/default.gin", default_src);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    let byte = default_src.find("Default").expect("`Default` in source") as u32;

    let HoverContent { markdown, .. } = engine.hover(&path, byte).expect("hover on `Default`");

    assert_eq!(
        markdown,
        [
            "```gin",
            "core.default",
            "```",
            "",
            "```gin",
            "Default(value) has default value",
            "```",
            "---",
            "",
            "Provides a default value for a type.",
        ]
        .join("\n"),
    );
}

#[test]
fn hover_false_constant_bind_shows_correct_type() {
    let bool_src = BOOL_GIN;
    let pkg = TempPackage::new("false_bind");
    pkg.write_nested_flask("core", "core");
    let path = pkg.write("core/primitive/bool.gin", bool_src);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    // Hover on `false` in `false := Bool.False`
    let byte = bool_src.find("false :=").expect("`false :=` start") as u32;
    let HoverContent { markdown, .. } = engine.hover(&path, byte).expect("hover on `false` bind");

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
fn hover_variant_in_union_declaration_shows_qualified_union_path() {
    let bool_src = BOOL_GIN;
    let pkg = TempPackage::new("variant_in_union");
    pkg.write_nested_flask("core", "core");
    let path = pkg.write("core/primitive/bool.gin", bool_src);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    // Hover on `True` inside `Bool is True or False`.
    let byte = bool_src.find("True").expect("`True` in bool.gin") as u32;
    let HoverContent { markdown, .. } = engine.hover(&path, byte).expect("hover on `True` variant");

    assert_eq!(
        markdown,
        "\
```gin
core.primitive.Bool
```

```gin
True
```"
    );
}

/// Variant hover on `Some(x)` in `Maybe` must show the variant with its field
/// and doc comment.
#[test]
fn hover_variant_with_fields_shows_shape_and_doc() {
    let maybe_src = MAYBE_GIN;
    let pkg = TempPackage::new("maybe_variant");
    pkg.write_nested_flask("core", "core");
    let path = pkg.write("core/maybe/maybe.gin", maybe_src);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    // Hover on `Some` in `Some(x)`
    let byte = maybe_src.find("Some(x)").expect("`Some(x)` in maybe.gin") as u32;
    let HoverContent { markdown, .. } = engine.hover(&path, byte).expect("hover on `Some` variant");

    assert_eq!(
        markdown,
        "\
```gin
core.maybe.Maybe
```

```gin
Some(x)
```
---

Has some value `x`"
    );
}

/// Hovering on a tag name referenced in a qualified path expression (e.g. `Bool`
/// in `Bool.False`) must show the tag declaration, not the variant/expression hover.
#[test]
fn hover_reflect_type_union_shows_multiline_named_payload_variants() {
    let type_src = TYPE_GIN;
    let pkg = TempPackage::new("reflect_type_hover");
    pkg.write_nested_flask("core", "core");
    let path = pkg.write("core/reflect/type.gin", type_src);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    for (content, rel) in [
        (INT_GIN, "primitive/int.gin"),
        (BOOL_GIN, "primitive/bool.gin"),
        (LIST_GIN, "primitive/list.gin"),
        (STRING_GIN, "string/string.gin"),
    ] {
        let p = pkg.write(&format!("core/{rel}"), content);
        engine.add_file(p).unwrap();
    }
    engine.add_file(path.clone()).unwrap();

    let byte = type_src
        .find("Type is")
        .expect("`Type is` in reflect/type.gin") as u32;
    let HoverContent { markdown, .. } = engine.hover(&path, byte).expect("hover on `Type`");

    assert_eq!(
        markdown,
        "\
```gin
core.reflect
```

```gin
Type is Primitive(width BigInt, signed Bool)
     or Record(name String, fields List(NamedTy))
     or Union(name String, variants List(VariantShape))
     or Tuple(elems List(Type))
     or Ptr(inner Type)
     or Ref(inner Type, mutable Bool)
     or Array(elem Type, size BigInt)
     or Opaque(name String)
```
---

Structural description of a Gin type, used for compile-time reflection."
    );
}

#[test]
fn hover_tag_name_in_qualified_path_shows_tag_declaration() {
    let bool_src = BOOL_GIN;
    let pkg = TempPackage::new("tag_path");
    pkg.write_nested_flask("core", "core");
    let path = pkg.write("core/primitive/bool.gin", bool_src);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    // Hover on `Bool` in `false := Bool.False` (qualified path usage)
    let byte = bool_src
        .find("Bool.False")
        .expect("`Bool.False` in bool.gin") as u32;
    let HoverContent { markdown, .. } = engine
        .hover(&path, byte)
        .expect("hover on `Bool` in qualified path");

    assert_eq!(
        markdown,
        "\
```gin
core.primitive
```

```gin
Bool is True or False
```
---

`Bool` represents a value, which could only be either `True` or `False`.

## Basic usage

`Bool` implements various traits, such as BitAnd, BitOr, Not, etc.,
which allow us to perform boolean operations using &, | and !.

`if` requires a `Bool` value as its conditional."
    );
}

/// Hovering a tag name that is declared in another file within the same package
/// must show the tag declaration hover (same as hovering it in its defining file).
///
/// Uses two files under a single package root (no dependency resolution needed).
#[test]
fn hover_cross_file_tag_shows_tag_declaration() {
    let pkg = TempPackage::new("cross_tag");
    pkg.write_flask("cross_tag");
    // Declare a tag in one file
    pkg.write("a.gin", "Bool is True or False");
    // Second file that references `Bool` (via same-package cross-file resolution)
    let source = "main:\n    b Bool := 1\n    return b\n";
    let path = pkg.write("main.gin", source);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(pkg.join("a.gin")).unwrap();
    engine.add_file(path.clone()).unwrap();

    // Hover on `Bool` in `b Bool := true` (cross-file reference)
    let bool_byte = source.find("Bool").expect("`Bool` in main.gin") as u32;
    let HoverContent { markdown, .. } = engine
        .hover(&path, bool_byte)
        .expect("hover on cross-file `Bool`");

    assert_eq!(
        markdown,
        "\
```gin\ncross_tag\n```\n\n```gin\nBool is True or False\n```"
    );
}

/// Tag name referenced inside another tag's body (e.g. `Bool` in
/// `has b Bool`) must resolve to the tag declaration.
#[test]
fn hover_tag_name_in_another_tags_body_shows_tag_declaration() {
    let pkg = TempPackage::new("nested_tag_ref");
    pkg.write_flask("nested_tag_ref");
    let source = "Bool is True or False\n\nContainer(x) has b Bool\n";
    let path = pkg.write("main.gin", source);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    // Hover on `Bool` in `has b Bool`
    let byte = source.rfind("Bool").expect("`Bool` in body") as u32;
    let HoverContent { markdown, .. } = engine
        .hover(&path, byte)
        .expect("hover on `Bool` in tag body");

    assert_eq!(
        markdown,
        "\
```gin\nnested_tag_ref\n```\n\n```gin\nBool is True or False\n```"
    );
}

/// Hovering a record field name inside a tag body (e.g. `pointer` in
/// `has pointer Pointer(x)`) should show the field and its type.
#[test]
fn hover_record_field_in_tag_body_shows_field() {
    let pkg = TempPackage::new("field_hover");
    pkg.write_flask("field_hover");
    let source = "Pointer(x) is @x\n\nContainer(x) has ptr Pointer(x)\n";
    let path = pkg.write("main.gin", source);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    // Hover on `ptr` in `has ptr Pointer(x)`
    let byte = source.rfind("ptr").expect("`ptr` in body") as u32;
    let HoverContent { markdown, .. } = engine.hover(&path, byte).expect("hover on `ptr` field");

    assert_eq!(
        markdown,
        "\
```gin\nfield_hover.Container\n```\n\n```gin\nptr Pointer(x)\n```"
    );
}

/// Hovering record fields when the named type is in the same file.
#[test]
fn hover_record_field_same_file_shows_named_type() {
    let source = "Pointer(x) is @x\n\nContainer(x) has ptr Pointer(x)\n";
    let pkg = TempPackage::new("same_file");
    pkg.write_flask("same_file");
    let path = pkg.write("main.gin", source);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    let ptr_byte = source.rfind("ptr").expect("`ptr` field") as u32;
    let HoverContent { markdown, .. } =
        engine.hover(&path, ptr_byte).expect("hover on `ptr` field");

    assert_eq!(
        markdown,
        "\
```gin\nsame_file.Container\n```\n\n```gin\nptr Pointer(x)\n```"
    );
}

#[test]
fn hover_narrowed_type_in_when_then_body() {
    let source = "\
Int is in -9223372036854775808...9223372036854775807\n\nmain:\n    x := 42\n    when x is\n        Int then x + 1\n            else 0\n    return x\n";
    let HoverContent { markdown, .. } = hover_at(source, "x + 1", "narrowed");
    assert_eq!(markdown, "x Int\n---\n\n");
}

/// Hover on a literal `42` expression — shows the literal value directly.
#[test]
fn hover_literal_shows_value() {
    let source = "main:\n    return 42\n";
    let HoverContent { markdown, .. } = hover_at(source, "42", "literal");
    assert_eq!(markdown, "42\n---\n\n");
}

/// Hover on a function call name in an expression — should route to the
/// definition via reference-to-def, showing the function signature.
#[test]
fn hover_fn_call_reference_shows_signature() {
    let source = "\
identity(x) Int := x\n\n\
main:\n    identity(4)\n    return 0\n";
    let pkg = TempPackage::new("fn_call_ref");
    pkg.write_flask("fn_call_ref");
    let path = pkg.write("main.gin", source);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    let call_byte = source.rfind("identity").expect("`identity` in call") as u32;
    let HoverContent { markdown, .. } = engine
        .hover(&path, call_byte)
        .expect("hover on `identity` call");

    assert_eq!(
        markdown,
        "\
```gin\nfn_call_ref\n```\n\n```gin\nidentity(x) Int\n```"
    );
}

/// Hovering on a reference to a top-level definition from inside a function
/// body should show the same hover as the definition itself.
#[test]
fn hover_top_level_reference_from_body_shows_definition_hover() {
    let source = "two := 1 + 1\n\nmain:\n    return two\n";
    // Use the same hover_at helper but hover on `two` inside the body.
    let pkg = TempPackage::new("top_ref");
    pkg.write_flask("top_ref");
    let path = pkg.write("main.gin", source);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    // Hover on the `two` inside `return two` (second occurrence).
    let def_byte = source.find("two :=").expect("`two :=`") as u32;
    let ref_byte = source.rfind("two").expect("`two` in return") as u32;

    let def_hover = engine.hover(&path, def_byte).expect("hover on def `two`");
    let ref_hover = engine.hover(&path, ref_byte).expect("hover on ref `two`");

    assert_eq!(def_hover.markdown, ref_hover.markdown);
    assert_eq!(
        ref_hover.markdown,
        "\
```gin
top_ref
```

```gin
two 2\n```"
    );
}

#[test]
fn interface_method_signatures_display_correctly() {
    let source = r#"
Allocator has
    --- Attempt to allocate.
    allocate(ref self, l Layout) Slice(Byte) or AllocError,
    --- Free a block.
    deallocate(ref self, p Pointer(Byte), l Layout),
)
"#;
    let pkg = TempPackage::new("interface_methods");
    pkg.write_flask("interface_methods");
    let path = pkg.write("main.gin", source);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    // Hover on `Allocator` tag name
    let byte = source.find("Allocator").expect("`Allocator` in source") as u32;
    let hover = engine.hover(&path, byte).expect("hover on `Allocator` tag");
    let HoverContent { markdown, .. } = hover;

    // The hover should show the declaration text
    assert_eq!(
        markdown,
        [
            "```gin",
            "interface_methods",
            "```",
            "",
            "```gin",
            "Allocator has",
            "    allocate(ref self, l Layout) Slice(Byte) or AllocError",
            "    deallocate(ref self, p Pointer(Byte), l Layout)",
            "```",
        ]
        .join("\n"),
        "hover on Allocator tag"
    );
}

#[test]
fn hover_has_method_name_returns_method_hover_without_extra_space() {
    let source = r#"
Allocator has
    --- Attempt to allocate.
    reserve(ref self, l Layout) Slice(Byte) or AllocError,
    --- Free a block.
    release(ref self, p Pointer(Byte), l Layout),
)
"#;
    let pkg = TempPackage::new("interface_method_hover");
    pkg.write_flask("interface_method_hover");
    let path = pkg.write("main.gin", source);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    let byte = source.find("release").expect("`release` in source") as u32;
    let HoverContent { markdown, .. } = engine
        .hover(&path, byte)
        .expect("hover on `release` method");

    assert_eq!(
        markdown,
        [
            "```gin",
            "interface_method_hover.Allocator",
            "```",
            "",
            "```gin",
            "release(ref self, p Pointer(Byte), l Layout)",
            "```",
            "---",
            "",
            "Free a block.",
        ]
        .join("\n")
    );
    assert!(
        !markdown.contains("release ("),
        "method hover should not add a space after the name"
    );
}

/// Hovering on an interface member name should show its doc comment.
#[test]
fn hover_interface_member_shows_doc_comment() {
    let source = r#"
Allocator has
    --- Attempt to allocate a block of memory.
    allocate(ref self, l Layout) Slice(Byte) or AllocError,
    --- Free a block of memory.
    deallocate(ref self, p Pointer(Byte), l Layout),
)
"#;
    let pkg = TempPackage::new("interface_member_doc");
    pkg.write_flask("interface_member_doc");
    let path = pkg.write("main.gin", source);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    // Hover on `allocate` method name
    let byte = source.find("allocate").expect("`allocate` in source") as u32;
    let hover = engine
        .hover(&path, byte)
        .expect("hover on `allocate` method");

    assert_eq!(
        hover.markdown,
        [
            "```gin",
            "interface_member_doc.Allocator",
            "```",
            "",
            "```gin",
            "allocate(ref self, l Layout) Slice(Byte) or AllocError",
            "```",
            "---",
            "",
            "Attempt to allocate a block of memory.",
        ]
        .join("\n"),
        "hover on `allocate` method"
    );
}

// The classify_hover tests in `ast/tests/hover_classify_tests.rs` verify the
// module path detection at the AST level. Full end-to-end integration tests
// (cross-file package resolution + module doc display on hover) are tracked
// as follow-up work once the cross-file test infrastructure supports it.
//
// See `ast/tests/hover_classify_tests.rs`:
//   - `module_path_classified`
//   - `module_path_root_classified`
//   - `module_path_not_classified_for_final_segment`

/// Hovering on `self` inside a has method body shows the full type with the
/// receiver type (e.g. `self Region`), not just the keyword doc.
#[test]
fn hover_self_in_has_method_body_shows_type() {
    let source = "Region has\n    reset(self) Region: self\n";
    let pkg = TempPackage::new("self_hover");
    pkg.write_flask("self_hover");
    let path = pkg.write("main.gin", source);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    // Hover on the `self` inside the method body (not the parameter self).
    let byte = source.rfind("self").expect("body `self`") as u32;
    let hover = engine.hover(&path, byte).expect("hover on body self");
    assert_eq!(
        hover.markdown,
        ["```gin", "self Region", "```"].join("\n"),
        "hover on `self` in has method body should show `self Region`"
    );
}

/// Hovering on `ref self` in a has method body shows `ref self Type`.
#[test]
fn hover_ref_self_in_has_method_body_shows_ref_type() {
    let source = "Region has\n    reset(ref self) Region: self\n";
    let pkg = TempPackage::new("ref_self_hover");
    pkg.write_flask("ref_self_hover");
    let path = pkg.write("main.gin", source);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    let byte = source.rfind("self").expect("body `self`") as u32;
    let hover = engine.hover(&path, byte).expect("hover on body self");
    assert_eq!(
        hover.markdown,
        ["```gin", "ref self Region", "```"].join("\n"),
        "hover on `self` in has method body should show `ref self Region`"
    );
}

/// Hovering on `self` inside a has method with a block body shows the type
/// (e.g. `self Type`, `ref self Type`, `mut self Type`).
#[test]
fn hover_self_in_has_block_body_shows_ref_type() {
    let source = "Region has\n    reset(ref self) Region:\n        self\n";
    let pkg = TempPackage::new("block_self_hover");
    pkg.write_flask("block_self_hover");
    let path = pkg.write("main.gin", source);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    let byte = source.rfind("self").expect("body `self`") as u32;
    let hover = engine.hover(&path, byte).expect("hover on body self");
    assert_eq!(
        hover.markdown,
        ["```gin", "ref self Region", "```"].join("\n"),
        "hover on `self` in has block body should show `ref self Region`"
    );
}

/// Hovering on `self` in a block body with multiple expressions and a return
/// (mirroring the real region.gin `allocate` method) shows the full type.
#[test]
fn hover_self_in_block_body_with_multiple_exprs_and_return() {
    let source = r#"Region has
    base Pointer(Byte)
    cursor Pointer(Byte)
    end Pointer(Byte)

    allocate(ref self, size PointerSize) Pointer(Byte):
        self
        -- a comment
    return self.cursor
"#;
    let pkg = TempPackage::new("multi_expr_self");
    pkg.write_flask("multi_expr_self");
    let path = pkg.write("main.gin", source);

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    engine.add_file(path.clone()).unwrap();

    // Hover on the standalone `self` expression at line 6, character 8.
    let (line, character) = (6u32, 8u32);
    let byte_offset = source.position_to_byte_offset(line, character).unwrap();
    let hover = engine
        .hover(&path, byte_offset as u32)
        .expect("hover on body self");
    assert_eq!(
        hover.markdown,
        ["```gin", "ref self Region", "```"].join("\n"),
        "hover on `self` in block body with return should show `ref self Region`"
    );
}
