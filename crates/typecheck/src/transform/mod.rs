//! 3-stage transformation pipeline: `FileAst → TypedFileAst`.
//!
//! Stages:
//! 1. **Declare** — Resolve tag/bind declarations, build file-level index, detect declaration flaws.
//! 2. **Lower** — Lower parse expressions to typed expressions, attach type flaws.
//! 3. **Flow** — Compute flow contexts, attach flow flaws (ownership, bounds).

use crate::analysis::desugar_threads::stage_desugar_threads;
use crate::analysis::infer_convention::infer_param_conventions;
use ast::BlanketImpl;
use ast::FileAst;
use ast::prelude::*;

use crate::typed::{DefId, FileId, TagId, TypedFileAst, VariantMapEntry};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

mod bounds_check;
mod declare;
mod flow;
mod inline_comptime_calls;
mod lower_exprs;
mod lower_kind;
mod lower_tag;
mod lower_ty;
mod lower_util;
mod lower_when;

pub use declare::*;
pub use flow::*;
pub use lower_exprs::*;

/// Cross-file type environment for the transformation pipeline.
///
/// Carries resolved type information from other files so that expressions in
/// this file can resolve cross-file references.
pub struct TransformCtx {
    /// Tag types from other files: tag name → resolved Ty.
    pub cross_file_tag_types: HashMap<TagId, crate::ty::Ty>,
    /// Function return types from other files: def name → return Ty.
    pub cross_file_fn_return_types: HashMap<DefId, crate::ty::Ty>,
    /// Variant map entries from other files.
    pub cross_file_variant_map: HashMap<Intern<String>, Vec<VariantMapEntry>>,
    /// Blanket impls from all package files (for cross-file trait dispatch).
    pub package_blanket_impls: Vec<BlanketImpl>,
    /// Merged package AST used to evaluate compile-time trait bodies.
    pub compile_time_eval_ast: Arc<FileAst>,
    /// IDE package pipeline: skip comptime inlining and heavy Reflectable synthesis.
    pub ide_package: bool,
}

impl TransformCtx {
    pub fn new() -> Self {
        Self {
            cross_file_tag_types: HashMap::new(),
            cross_file_fn_return_types: HashMap::new(),
            cross_file_variant_map: HashMap::new(),
            package_blanket_impls: Vec::new(),
            compile_time_eval_ast: Arc::new(FileAst::empty_for_tests()),
            ide_package: false,
        }
    }

    pub fn with_package_compile_time(
        package_blanket_impls: Vec<BlanketImpl>,
        compile_time_eval_ast: FileAst,
    ) -> Self {
        Self::with_package_compile_time_arc(package_blanket_impls, Arc::new(compile_time_eval_ast))
    }

    pub fn with_package_compile_time_arc(
        package_blanket_impls: Vec<BlanketImpl>,
        compile_time_eval_ast: Arc<FileAst>,
    ) -> Self {
        Self {
            package_blanket_impls,
            compile_time_eval_ast,
            ..Self::new()
        }
    }
}

impl Default for TransformCtx {
    fn default() -> Self {
        Self::new()
    }
}

impl TransformCtx {
    /// This collects tag types, function return types, and variant maps from all files
    /// so that a new file being transformed can resolve references to symbols in these files.
    pub fn from_typed_asts(asts: &[&TypedFileAst]) -> Self {
        let mut ctx = Self::new();
        // Each `ast.variant_map` carries both its own variants and the cross-file
        // entries spliced in during `stage_declare`, so we deduplicate by routing
        // every variant entry through the tag that owns it. Without this each
        // additional file would re-extend the variant Vec with prior files' entries.
        for ast in asts {
            for (tag_id, tag) in &ast.tags {
                ctx.cross_file_tag_types
                    .insert(*tag_id, tag.resolved_ty.clone());
            }
            for (def_id, bind) in &ast.defs {
                ctx.cross_file_fn_return_types
                    .insert(*def_id, bind.return_type.clone());
            }
        }

        ctx.cross_file_variant_map = crate::collect_package_variant_map(asts);
        ctx
    }
}

/// Stage 1 only: resolve declarations and build the file-level index.
pub fn transform_declare(file_ast: &FileAst, file_id: FileId, ctx: &TransformCtx) -> TypedFileAst {
    stage_declare(file_ast, file_id, ctx)
}

/// Stages 2–3 on an AST produced by [`transform_declare`].
///
/// Merges cross-file defs from `ctx` into `typed.fn_return_types` before lowering so
/// forward references across files resolve regardless of compilation order.
pub fn transform_finish(typed: &mut TypedFileAst, file_ast: &FileAst, ctx: &TransformCtx) {
    for (def_id, return_ty) in &ctx.cross_file_fn_return_types {
        typed
            .fn_return_types
            .entry(*def_id)
            .or_insert_with(|| return_ty.clone());
    }

    stage_lower(typed, file_ast, ctx);

    let mut conventions: HashMap<DefId, crate::analysis::infer_convention::ConventionInference> =
        HashMap::new();
    for (name, bind) in &file_ast.defs {
        let conv = infer_param_conventions(bind);
        conventions.insert(DefId(*name), conv);
    }

    let _thread_flaws = stage_desugar_threads(typed, &conventions);
    stage_flow(typed, &conventions);
    if !ctx.ide_package {
        inline_comptime_calls::stage_inline_comptime_calls(typed, file_ast, ctx);
    }
}

/// Options for [`transform_package`].
#[derive(Clone, Copy, Debug, Default)]
pub struct PackageTransformOptions {
    /// When true, omit merged eval AST and comptime inlining (LSP / diagnostics).
    pub ide_package: bool,
}

impl PackageTransformOptions {
    pub const IDE: Self = Self { ide_package: true };
    pub const FULL: Self = Self { ide_package: false };
}

fn merge_declared_file_into_ctx(ctx: &mut TransformCtx, typed: &TypedFileAst, file_ast: &FileAst) {
    for (tag_id, tag) in &typed.tags {
        ctx.cross_file_tag_types
            .insert(*tag_id, tag.resolved_ty.clone());
    }
    for (def_id, bind) in &typed.defs {
        ctx.cross_file_fn_return_types
            .insert(*def_id, bind.return_type.clone());
    }
    // `typed.variant_map` contains both this file's own variants *and* the cross-file
    // entries spliced in during `stage_declare`. Re-inserting the cross-file entries
    // each iteration caused exponential blow-up (each file doubled the previous file's
    // variant entries), so only merge variants owned by tags declared in this file.
    let own_tags: HashSet<Intern<String>> = file_ast.tags.keys().copied().collect();
    for (variant_name, entries) in &typed.variant_map {
        let own: Vec<_> = entries
            .iter()
            .filter(|(union_name, _, _)| own_tags.contains(union_name))
            .cloned()
            .collect();
        if own.is_empty() {
            continue;
        }
        ctx.cross_file_variant_map
            .entry(*variant_name)
            .or_default()
            .extend(own);
    }
}

/// Map `use dep.(folder.Symbol)` aliases onto types from already-declared package files.
fn enrich_ctx_from_symbol_aliases(file_ast: &FileAst, ctx: &mut TransformCtx) {
    for alias in &file_ast.symbol_aliases {
        let tag_name = alias
            .target
            .segments
            .last()
            .copied()
            .unwrap_or(alias.target.root);
        if let Some(ty) = ctx.cross_file_tag_types.get(&TagId(tag_name)) {
            ctx.cross_file_tag_types
                .insert(TagId(alias.alias), ty.clone());
        }
    }
}

pub fn transform_package(
    file_asts: &[(FileAst, FileId)],
    package_ctx: &TransformCtx,
    opts: PackageTransformOptions,
) -> Vec<TypedFileAst> {
    let mut accumulated_ctx = TransformCtx {
        package_blanket_impls: package_ctx.package_blanket_impls.clone(),
        compile_time_eval_ast: Arc::clone(&package_ctx.compile_time_eval_ast),
        ide_package: opts.ide_package,
        ..TransformCtx::new()
    };

    let mut declared: Vec<TypedFileAst> = Vec::with_capacity(file_asts.len());
    for (file_ast, file_id) in file_asts {
        enrich_ctx_from_symbol_aliases(file_ast, &mut accumulated_ctx);
        let typed = transform_declare(file_ast, *file_id, &accumulated_ctx);
        merge_declared_file_into_ctx(&mut accumulated_ctx, &typed, file_ast);
        declared.push(typed);
    }

    // Per-file declare already runs a tag fixpoint with `accumulated_ctx`. A second
    // package-wide fixpoint re-resolved every tag and blew memory/time on gin_core;
    // cross-file declare order + merged variant index covers IDE needs.

    let declared_refs: Vec<&TypedFileAst> = declared.iter().collect();
    let mut full_ctx = TransformCtx::from_typed_asts(&declared_refs);
    full_ctx.package_blanket_impls = package_ctx.package_blanket_impls.clone();
    full_ctx.compile_time_eval_ast = Arc::clone(&package_ctx.compile_time_eval_ast);
    full_ctx.ide_package = opts.ide_package;

    for (typed, (file_ast, _)) in declared.iter_mut().zip(file_asts.iter()) {
        transform_finish(typed, file_ast, &full_ctx);
    }

    declared
}

/// Attach cross-file tag/variant index for hover on a single transformed file.
///
/// Package transforms must not call this per file (see [`collect_package_variant_map`]).
fn splice_cross_file_for_single_file(typed: &mut TypedFileAst, ctx: &TransformCtx) {
    for (tag_id, ty) in &ctx.cross_file_tag_types {
        typed.tag_types.entry(*tag_id).or_insert_with(|| ty.clone());
    }
    for (variant_name, entries) in &ctx.cross_file_variant_map {
        typed
            .variant_map
            .entry(*variant_name)
            .or_default()
            .extend(entries.iter().cloned());
    }
}

/// This is the main entry point for external callers (ginc, ginlsp, database).
/// It wraps parsing output into a `FileAst`, assigns a `FileId`, and runs
/// the 3-stage pipeline.
pub fn transform_file(file_ast: FileAst, file_id: FileId) -> TypedFileAst {
    let ctx = TransformCtx::new();
    transform(&file_ast, file_id, &ctx)
}

/// Use this when transforming files that may reference symbols from other files.
/// The `ctx` should be built from already-transformed files via `TransformCtx::from_typed_asts`.
pub fn transform_file_with_ctx(
    file_ast: &FileAst,
    file_id: FileId,
    ctx: &TransformCtx,
) -> TypedFileAst {
    transform(file_ast, file_id, ctx)
}

pub fn transform(file_ast: &FileAst, file_id: FileId, ctx: &TransformCtx) -> TypedFileAst {
    let mut typed = transform_declare(file_ast, file_id, ctx);
    // Propagate parse-level diagnostics from the AST.
    typed.parse_warnings = file_ast.parse_warnings.clone();
    transform_finish(&mut typed, file_ast, ctx);
    splice_cross_file_for_single_file(&mut typed, ctx);
    typed
}
