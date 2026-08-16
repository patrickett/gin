//! Shared parse → transform helpers for AST integration tests.
//!
//! Marker/reflect tests share a single parsed `gin_core` bundle via [`marker_fixtures`]
//! so each test does not reload seven modules and re-run compile-time folding.
//! For self-contained snippets (inline `Type` / `List` / …), prefer [`transform_source`].

#![allow(dead_code)]

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use ast::FileAst;
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::CompileTimeTraitRegistry;
use typecheck::transform::{TransformCtx, stage_declare, transform};
use typecheck::{FileId, TypedFileAst};

pub fn typed_file(source: &str) -> TypedFileAst {
    transform_source(source)
}

pub fn transform_source(source: &str) -> TypedFileAst {
    let file_ast = TokenCursor::parse_source(source);
    transform(&file_ast, FileId(0), &TransformCtx::new())
}

/// Empty typed AST for unit tests that only need trait dispatch on `Ty` values.
pub fn empty_typed() -> TypedFileAst {
    TypedFileAst::new(FileId(0), ast::span::SpanTable::default())
}

struct MarkerPackageFixtures {
    eval_ast: Arc<FileAst>,
    eval_typed: TypedFileAst,
    transform_ctx: Arc<TransformCtx>,
}

struct FullMarkerPackageFixtures {
    transform_ctx: Arc<TransformCtx>,
}

static MARKER_FIXTURES: OnceLock<MarkerPackageFixtures> = OnceLock::new();
static FULL_MARKER_FIXTURES: OnceLock<FullMarkerPackageFixtures> = OnceLock::new();

/// Parsed `gin_core` marker bundle + cross-file declare context (built once per test process).
fn marker_fixtures() -> &'static MarkerPackageFixtures {
    MARKER_FIXTURES.get_or_init(|| {
        // Keep this fixture intentionally small: these tests only need the reflect `Type`
        // surface plus a handful of primitive tags for hover/pattern binding.
        let eval_ast = Arc::new(load_marker_eval_ast_light());
        // Declare-only: types + variant_map for hover/patterns; skip comptime fold on copy.gin.
        let eval_typed = stage_declare(eval_ast.as_ref(), FileId(1), &TransformCtx::new());
        let mut transform_ctx = TransformCtx::from_typed_asts(&[&eval_typed]);
        transform_ctx.compile_time_eval_ast = Arc::clone(&eval_ast);
        transform_ctx.ide_package = true;
        MarkerPackageFixtures {
            eval_ast,
            eval_typed,
            transform_ctx: Arc::new(transform_ctx),
        }
    })
}

/// Load `gin_core` marker/reflect sources into a merged eval AST and trait registry.
pub fn marker_trait_registry() -> CompileTimeTraitRegistry {
    // Trait tests want auto trait defaults and compile-time bodies from marker modules.
    // Keep this separate from the lightweight hover/pattern fixture.
    let eval = Arc::new(load_marker_eval_ast_full());
    let mut imported = HashSet::new();
    for name in [
        "Copy",
        "Sized",
        "Reflectable",
        "Type",
        "NamedTy",
        "VariantShape",
        "Size",
        "BigInt",
        "Bool",
        "True",
        "False",
        "Const",
        "Dynamic",
        "List",
        "String",
    ] {
        imported.insert(Intern::new(name.to_string()));
    }
    CompileTimeTraitRegistry {
        imported_traits: imported,
        eval_ast: eval,
    }
}

pub fn marker_transform_ctx() -> Arc<TransformCtx> {
    Arc::clone(&marker_fixtures().transform_ctx)
}

pub fn transform_with_markers(source: &str) -> TypedFileAst {
    let file_ast = TokenCursor::parse_source(source);
    transform(&file_ast, FileId(0), marker_transform_ctx().as_ref())
}

/// Transform `source` with gin_core types/variants from the marker eval package in scope.
pub fn transform_with_marker_package(source: &str) -> TypedFileAst {
    let file_ast = TokenCursor::parse_source(source);
    transform(
        &file_ast,
        FileId(0),
        marker_fixtures().transform_ctx.as_ref(),
    )
}

fn full_marker_fixtures() -> &'static FullMarkerPackageFixtures {
    FULL_MARKER_FIXTURES.get_or_init(|| {
        let mut transform_ctx = TransformCtx::from_typed_asts(&[marker_eval_typed_for_tests()]);
        transform_ctx.ide_package = true;
        FullMarkerPackageFixtures {
            transform_ctx: Arc::new(transform_ctx),
        }
    })
}

/// No-eval marker type surface for hover tests that need `Type` / `List`.
pub fn transform_with_full_marker_eval(source: &str) -> TypedFileAst {
    let file_ast = TokenCursor::parse_source(source);
    transform(
        &file_ast,
        FileId(0),
        full_marker_fixtures().transform_ctx.as_ref(),
    )
}

/// Merged gin_core AST used by marker/reflect integration tests.
pub fn load_marker_eval_ast_for_tests() -> FileAst {
    marker_fixtures().eval_ast.as_ref().clone()
}

/// Declare-only typed marker eval used by hover tests that need a package index.
pub fn marker_eval_typed_for_tests() -> &'static TypedFileAst {
    &marker_fixtures().eval_typed
}

/// Inline Gin source providing primitive types (BigInt, Bool, List, Pointer, String, etc.).
const LIGHT_MARKER_SRC: &str = "\
BigInt is in 0...18446744073709551615
Int is in 0...4294967295

Bool is True or False

List(x) has pointer Pointer(x), length BigInt
Pointer(x) is @x
PointerSize is BigInt

String has bytes List(BigInt)
ToString has to_string String
Happy has value Bool

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

Reflectable has shape Type

Copy has can_copy Bool: True
";

fn load_marker_eval_ast_light() -> FileAst {
    let mut eval = FileAst::default();
    eval.merge_from(TokenCursor::parse_source(LIGHT_MARKER_SRC));
    eval
}

/// Inline Gin source for the full marker eval bundle (primitives + Copy + Sized).
const FULL_MARKER_SRC: &str = "\
BigInt is in 0...18446744073709551615
Int is in 0...4294967295

Bool is True or False

List(x) has pointer Pointer(x), length BigInt
Pointer(x) is @x
PointerSize is BigInt

String has bytes List(BigInt)
ToString has to_string String
Happy has value Bool

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

Reflectable has shape Type

Size is Const(BigInt) or Dynamic

Sized has size Size: compute_size(Self)

compute_size(x Type) Size := when x is
    Primitive(w, _)     then Const(w / 8)
    Ptr(_)              then Const(8)
    Ref(_, _)           then Const(8)
    Opaque(_)           then Dynamic
    Record(_, fields)   then sum_named(fields)
    Tuple(elems)        then sum_types(elems)
    Union(_, variants)  then union_size(variants)
    Array(elem, n)      then mul_size(compute_size(elem), n)

sum_named(fields List(NamedTy)) Size := when fields is
    []                  then Const(0)
    [f, ...rest]        then add(compute_size(f.ty), sum_named(rest))

sum_types(elems List(Type)) Size := when elems is
    []                  then Const(0)
    [t, ...rest]        then add(compute_size(t), sum_types(rest))

add(a Size, b Size) Size := when (a, b) is
    (Const(x), Const(y)) then Const(x + y)
                         else Dynamic

mul_size(size Size, n BigInt) Size := when size is
    Const(x) then Const(x * n)
             else Dynamic

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

union_size(variants List(VariantShape)) Size := add(union_disc(variants), union_max_payload(variants))

Copy has can_copy Bool: is_copy(Self)

is_copy(x Type) Bool := when x is
    Primitive(_, _)     then True
    Ptr(_)              then False
    Ref(_, False)       then True
    Ref(_, True)        then False
    Opaque(_)           then False
    Record(_, fields)   then all_named_copy(fields)
    Tuple(elems)        then all_types_copy(elems)
    Union(_, variants)  then all_variants_copy(variants)
    Array(elem, _)      then is_copy(elem)

all_named_copy(fields List(NamedTy)) Bool := when fields is
    []                  then True
    [f, ...rest]        then is_copy(f.ty) and all_named_copy(rest)

all_types_copy(elems List(Type)) Bool := when elems is
    []                  then True
    [t, ...rest]        then is_copy(t) and all_types_copy(rest)

all_variants_copy(variants List(VariantShape)) Bool := when variants is
    []                  then True
    [v, ...rest]        then all_named_copy(v.fields) and all_variants_copy(rest)
";

fn load_marker_eval_ast_full() -> FileAst {
    let mut eval = FileAst::default();
    eval.merge_from(TokenCursor::parse_source(FULL_MARKER_SRC));
    eval
}

/// Like `transform_source` but prepends a small record tag (`Cell has n Int`
/// and a numeric `Int` tag so tests that exercise typed local bindings have a
/// known schema available without loading `gin_core`.
pub fn transform_source_with_typed_locals(source: &str) -> TypedFileAst {
    transform_source(&format!("Int is in 1...400\n\nCell has n Int\n\n{source}"))
}

/// Resident set size for the current process (best-effort; 0 if unavailable).
pub fn process_rss_bytes() -> usize {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        let pid = std::process::id().to_string();
        if let Ok(out) = Command::new("ps").args(["-o", "rss=", "-p", &pid]).output()
            && let Ok(text) = std::str::from_utf8(&out.stdout)
            && let Ok(kb) = text.trim().parse::<usize>()
        {
            return kb * 1024;
        }
    }
    #[cfg(target_os = "linux")]
    if let Ok(text) = std::fs::read_to_string("/proc/self/status") {
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                let kb: usize = rest
                    .trim()
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                return kb * 1024;
            }
        }
    }
    let _ = ();
    0
}
