//! 3-stage transformation pipeline: `ParseAst → TypedFileAst`.
//!
//! Stages:
//! 1. **Declare** — Resolve tag/bind declarations, build file-level index, detect declaration flaws.
//! 2. **Resolve** — Lower parse expressions to typed expressions, attach type flaws.
//! 3. **Flow** — Compute flow contexts, attach flow flaws (ownership, bounds).

use crate::prelude::*;
use crate::span::SpanTable;
use crate::typed::{DefId, FileId, TagId, TypedFileAst};
use std::collections::{HashMap, HashSet};

mod declare;
mod flow;
mod resolve;

pub use declare::*;
pub use flow::*;
pub use resolve::*;

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
    pub cross_file_variant_map: HashMap<Intern<String>, Vec<crate::analysis::VariantMapEntry>>,
}

impl TransformCtx {
    pub fn new() -> Self {
        Self {
            cross_file_tag_types: HashMap::new(),
            cross_file_fn_return_types: HashMap::new(),
            cross_file_variant_map: HashMap::new(),
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
    pub fn from_typed_asts(asts: &[TypedFileAst]) -> Self {
        let mut ctx = Self::new();
        for ast in asts {
            // Collect tag types
            for (tag_id, tag) in &ast.tags {
                ctx.cross_file_tag_types
                    .insert(*tag_id, tag.resolved_ty.clone());
            }
            // Collect function return types
            for (def_id, bind) in &ast.defs {
                ctx.cross_file_fn_return_types
                    .insert(*def_id, bind.return_type.clone());
            }
            // Collect variant map entries
            for (variant_name, entries) in &ast.variant_map {
                ctx.cross_file_variant_map
                    .entry(*variant_name)
                    .or_default()
                    .extend(entries.iter().cloned());
            }
        }
        ctx
    }
}

/// The original `ParseAst` is consumed — the typed tree becomes the new source of truth.
pub fn transform(parse_ast: ParseAst, file_id: FileId, ctx: &TransformCtx) -> TypedFileAst {
    // Stage 1: Declare — resolve tag/bind declarations, build file-level index.
    // We borrow the ParseAst here so Stage 2 can still read its expressions.
    let mut typed = stage_declare(&parse_ast, file_id, ctx);

    // Stage 2: Resolve — lower expressions to the typed arena, attach type flaws.
    stage_resolve(&mut typed, &parse_ast, ctx);

    // Stage 3: Flow — compute flow contexts, attach flow flaws.
    stage_flow(&mut typed);

    typed
}

/// A `ParseAst` is a parse-time representation (like today's `FileAst` with per-node flaws).
/// This is a simplified version — it mirrors today's `FileAst` structure.
/// Parse-time flaw integration on per-node (via `diagnostic::DiagnosticCode`) will be
/// added in follow-up.
#[derive(Debug, Clone)]
pub struct ParseAst {
    pub module_doc: Option<DocComment>,
    pub uses: Vec<Import>,
    pub tags: crate::TagMap,
    pub defs: crate::DefMap,
    pub private_defs: HashSet<Intern<String>>,
    pub private_tags: HashSet<Intern<String>>,
    pub exprs: Vec<(Expr, SpanId)>,
    pub symbol_aliases: Vec<crate::SymbolAlias>,
    pub symbol_alias_spans: Vec<SpanId>,
    pub span_table: SpanTable,
}

impl ParseAst {
    pub fn from_file_ast(file: FileAst) -> Self {
        Self {
            module_doc: file.module_doc,
            uses: file.uses,
            tags: file.tags,
            defs: file.defs,
            private_defs: file.private_defs,
            private_tags: file.private_tags,
            exprs: file.exprs,
            symbol_aliases: file.symbol_aliases,
            symbol_alias_spans: file.symbol_alias_spans,
            span_table: file.span_table,
        }
    }
}

/// This is the main entry point for external callers (ginc, ginlsp, database).
/// It wraps parsing output into a `ParseAst`, assigns a `FileId`, and runs
/// the 3-stage pipeline.
pub fn transform_file(file_ast: FileAst, file_id: FileId) -> TypedFileAst {
    let parse_ast = ParseAst::from_file_ast(file_ast);
    let ctx = TransformCtx::new();
    transform(parse_ast, file_id, &ctx)
}

/// Use this when transforming files that may reference symbols from other files.
/// The `ctx` should be built from already-transformed files via `TransformCtx::from_typed_asts`.
pub fn transform_file_with_ctx(
    file_ast: FileAst,
    file_id: FileId,
    ctx: &TransformCtx,
) -> TypedFileAst {
    let parse_ast = ParseAst::from_file_ast(file_ast);
    transform(parse_ast, file_id, ctx)
}
