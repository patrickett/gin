//! Shared parse → transform helpers for AST integration tests.
//!
//! Marker tests share a single parsed semantic bundle via [`marker_fixtures`]
//! so each test does not reload the modules and re-run compile-time folding.
//! For self-contained snippets (inline `Type` / `List` / …), prefer [`transform_source`].

#![allow(dead_code)]

use std::sync::{Arc, OnceLock};

use ast::FileAst;
use diagnostic::Diagnostic;
use flask::CompileTarget;
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::prepare_parse_ast;
use typecheck::transform::{TransformCtx, stage_declare, transform};
use typecheck::{BindBody, DefId, ExprId, FileId, TypedFileAst};

pub fn typed_file(source: &str) -> TypedFileAst {
    transform_source(source)
}

pub fn transform_source(source: &str) -> TypedFileAst {
    let file_ast = TokenCursor::parse_source(source);
    transform(&file_ast, FileId(0), &TransformCtx::new())
}

pub fn transform_prepared(source: &str) -> TypedFileAst {
    let mut file_ast = TokenCursor::parse_source(source);
    let _ = prepare_parse_ast(&mut file_ast, &CompileTarget::Library);
    transform(&file_ast, FileId(0), &TransformCtx::new())
}

pub fn body_expr_id(typed: &TypedFileAst, def_name: &str) -> Option<ExprId> {
    let def_id = DefId::named(Intern::new(def_name.to_string()));
    let bind = typed.defs.get(&def_id)?;
    match &bind.body {
        BindBody::Expr(eid) => Some(*eid),
        BindBody::Body { exprs, ret } => ret.or_else(|| exprs.last().copied()),
        BindBody::Unlowered => None,
    }
}

pub fn unknown_symbol_names(typed: &TypedFileAst) -> Vec<String> {
    typed
        .all_flaws()
        .into_iter()
        .filter(|&(_, flaw)| flaw.code.slug() == "type-unknown-symbol")
        .map(|(_, flaw)| flaw.arg("name").unwrap_or("").to_string())
        .collect()
}

pub fn has_flaw_with_slug(typed: &TypedFileAst, slug: &str) -> bool {
    typed.all_flaws().iter().any(|(_, f)| f.code.slug() == slug)
}

pub fn has_flaw_matching(typed: &TypedFileAst, pred: impl Fn(&Diagnostic) -> bool) -> bool {
    typed.all_flaws().iter().any(|(_, f)| pred(f))
}

struct MarkerPackageFixtures {
    eval_ast: Arc<FileAst>,
    eval_typed: TypedFileAst,
    transform_ctx: Arc<TransformCtx>,
}

static MARKER_FIXTURES: OnceLock<MarkerPackageFixtures> = OnceLock::new();

/// Parsed marker bundle + cross-file declare context (built once per test process).
fn marker_fixtures() -> &'static MarkerPackageFixtures {
    MARKER_FIXTURES.get_or_init(|| {
        // Keep this fixture intentionally small: these tests only need the `Type`
        // surface plus a handful of primitive tags for hover/pattern binding.
        let eval_ast = Arc::new(load_marker_eval_ast_light());
        // Declare-only: types + variant_map for hover/patterns; skip comptime fold.
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

fn marker_transform_ctx() -> Arc<TransformCtx> {
    Arc::clone(&marker_fixtures().transform_ctx)
}

pub fn transform_with_markers(source: &str) -> TypedFileAst {
    let file_ast = TokenCursor::parse_source(source);
    transform(&file_ast, FileId(0), marker_transform_ctx().as_ref())
}

/// Transform `source` with marker types/variants from the fixture package in scope.
pub fn transform_with_marker_package(source: &str) -> TypedFileAst {
    let file_ast = TokenCursor::parse_source(source);
    transform(
        &file_ast,
        FileId(0),
        marker_fixtures().transform_ctx.as_ref(),
    )
}

/// Inline Gin source providing primitive types (BigInt, Bool, List, Pointer, String, etc.).
const LIGHT_MARKER_SRC: &str = "\
BigInt is in 0 to 18446744073709551615
Int is in 0 to 4294967295

Bool is True or False

List(x) has pointer @x, length BigInt
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

Copy has can_copy Bool: True
";

fn load_marker_eval_ast_light() -> FileAst {
    let mut eval = FileAst::default();
    eval.merge_from(TokenCursor::parse_source(LIGHT_MARKER_SRC));
    eval
}

/// Like `transform_source` but prepends a small record tag (`Cell has n Int`
/// and a numeric `Int` tag so tests that exercise typed local bindings have a
/// known schema available without loading the full fixture package.
pub fn transform_source_with_typed_locals(source: &str) -> TypedFileAst {
    transform_source(&format!("Int is in 1 to 400\n\nCell has n Int\n\n{source}"))
}
