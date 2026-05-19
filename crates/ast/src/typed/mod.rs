use std::collections::{BTreeMap, HashMap, HashSet};

pub(crate) use crate::analysis::FlowContext;
use crate::marker::MarkerRegistry;
use crate::prelude::*;
use crate::span::{SpanId, SpanTable, SubSpan};
use crate::ty::Ty;

mod id;
mod transform;

pub use id::*;
pub use transform::*;

use soa_derive::StructOfArray;

/// A single expression in the typed AST arena.
///
/// All fields are stored in separate vectors via `soa_derive` for cache-friendly
/// iteration and per-field access.
#[derive(Debug, Clone, PartialEq, StructOfArray)]
#[soa_derive(Debug, Clone, PartialEq)]
pub struct TypedExpr {
    pub kind: TypedExprKind,
    pub ty: Ty,
    /// Source location for diagnostics and LSP.
    pub span: SpanId,
    /// Compile-time constant value, if this expression can be folded.
    pub const_value: Option<crate::analysis::ConstValue>,
    /// Type/flow/flaw diagnostics attached to this expression.
    pub flaws: Vec<diagnostic::TypeSymptom>,
    /// Flow-sensitive context at this program point, if computed.
    pub flow: Option<FlowContext>,
}

/// Typed expression variant — post-resolution form of parse-time [`Expr`].
///
/// Key differences from parse-time `Expr`:
/// - `TypeNominal`, `TypeQualified`, `TypeGeneric` — removed (desugared to `Ty`)
/// - `AnonymousTag` — removed (merged into `TagCall` with `args: None`)
/// - `FnCall` — uses `DefId` instead of path
/// - `TagCall` — uses `VariantId` + discriminant
/// - `Cast` — `ty` is `Ty` not `Intern<String>`
/// - All `Box<Typed<Expr>>` → `ExprId`
/// - All `Vec<Typed<Expr>>` → `Vec<ExprId>`
///
/// Typed when-expression — like `WhenExpr` but with `ExprId` children.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedWhenExpr {
    /// Subject expression for pattern matching (`None` for condition-based when).
    pub subject: Option<ExprId>,
    pub arms: Vec<TypedWhenArm>,
    /// Covers from after the `when` keyword to end.
    /// The full expression span is on the `TypedExpr` arena entry.
    pub body_span: SubSpan,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedWhenArm {
    Cond {
        condition: ExprId,
        body: ExprId,
        /// Span of this arm (condition and body).
        arm_span: SubSpan,
    },
    Is {
        pattern: Box<crate::span::Spanned<TypeExpr>>,
        body: ExprId,
        /// Span of this is-arm (pattern and body).
        arm_span: SubSpan,
    },
    Else(ExprId, SubSpan),
}

/// Typed if-expression — like `IfExpr` but with `ExprId` children.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedIfExpr {
    pub condition: TypedIfCondition,
    pub stmts: Vec<ExprId>,
    pub ret: Option<ExprId>,
    /// Covers from condition start to end (excludes the `if` keyword).
    /// The full expression span (including `if`) is on the `TypedExpr` arena entry.
    pub body_span: SubSpan,
}

/// Condition for a typed if-expression.
#[derive(Debug, Clone, PartialEq)]
pub enum TypedIfCondition {
    Bool(ExprId),
    Pattern {
        subject: ExprId,
        pattern: Box<crate::span::Spanned<TypeExpr>>,
    },
}

/// Typed loop — like `Loop` but with `ExprId` children.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedLoop {
    pub kind: TypedLoopKind,
    pub stmts: Vec<ExprId>,
    /// Span of the `loop` keyword only.
    /// The full expression span is on the `TypedExpr` arena entry.
    pub keyword_span: SubSpan,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedLoopKind {
    While {
        condition: ExprId,
    },
    ForIn {
        variable: Intern<String>,
        iterable: ExprId,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedExprKind {
    Lit(Literal),
    Binary {
        op: BinOp,
        lhs: ExprId,
        rhs: ExprId,
    },
    FnCall {
        target: DefId,
        args: Option<Vec<ExprId>>,
    },
    TagCall {
        variant_id: VariantId,
        discriminant: usize,
        args: Option<Vec<ExprId>>,
    },
    Bind {
        name: Intern<String>,
        stmts: Vec<ExprId>,
        body: ExprId,
    },
    When(TypedWhenExpr),
    If(TypedIfExpr),
    Loop(TypedLoop),
    SelfRef {
        target: DefId,
    },
    FormatString(FormatString),
    Range {
        start: ExprId,
        end: ExprId,
    },
    TupleLit(Vec<ExprId>),
    List(Vec<ExprId>),

    Cast {
        expr: ExprId,
        ty: Ty,
    },

    TupleAlloc {
        init: ExprId,
        size: usize,
    },
    TupleGet {
        base: ExprId,
        index: usize,
    },
    TupleSet {
        base: ExprId,
        index: usize,
        value: ExprId,
    },
    BufGet {
        buf: ExprId,
        index: ExprId,
    },
    BufSet {
        buf: ExprId,
        index: ExprId,
        value: ExprId,
    },
    TakePtr(ExprId),
    /// A safe reference: `ref expr` or `mut expr`.
    Ref(ExprId),
    Deref(ExprId),

    Negate(ExprId),

    /// Argument passed with `~` at call site: explicit consume.
    ConsumeArg(ExprId),
    /// Explicit consume: `eat expr`.
    Eat(ExprId),

    Asm(AsmExpr),
}

/// A fully-resolved tag declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedTag {
    /// The resolved type of this tag (e.g., `Ty::Union`, `Ty::Record`, `Ty::ConstUnion`, etc.).
    pub resolved_ty: Ty,
    /// Tag attributes (e.g., `#[os(linux)]`).
    pub attributes: DeclareAttributes,
    /// Marker bindings from `and is` / `and is not` clauses.
    pub marker_bindings: Vec<crate::marker::MarkerBinding>,
    pub doc_comment: Option<DocComment>,
    /// Tag parameters (type variables, defaults), if any.
    pub params: Option<Parameters>,
    /// Formatted declaration text (e.g. "Bool is True or False"), for use in hover.
    pub declaration_text: String,
}

/// A fully-resolved bind (function or value definition).
#[derive(Debug, Clone, PartialEq)]
pub struct TypedBind {
    pub name: Intern<String>,
    /// Span of the name in source.
    pub name_span: SpanId,
    pub body: BindBody,
    /// The resolved return type.
    pub return_type: Ty,
    /// Resolved parameter types.
    pub params: Vec<(Intern<String>, Ty)>,
    /// Receiver type for methods, if any.
    pub receiver_type: Option<Ty>,
    /// Bind attributes (e.g., `#[inline]`, visibility).
    pub attributes: BindAttributes,
    pub doc_comment: Option<DocComment>,
}

/// The body of a [`TypedBind`].
#[derive(Debug, Clone, PartialEq)]
pub enum BindBody {
    Expr(ExprId),
    /// A block body with multiple expressions and an optional return expression.
    Body {
        exprs: Vec<ExprId>,
        ret: Option<ExprId>,
    },
    Extern,
}

/// A fully-resolved import target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedImport {
    /// Resolved to a local definition.
    Local(DefId),
    /// Resolved to a local tag.
    LocalTag(TagId),
    /// Resolved to an external file's definition.
    External { file_id: FileId, def_id: DefId },
    /// Resolved to an external file's tag.
    ExternalTag { file_id: FileId, tag_id: TagId },
}

/// The typed AST for one `.gin` file — all types resolved, all flaws attached.
///
/// This is the source of truth for LSP queries, codegen, and further analysis.
#[derive(Clone)]
pub struct TypedFileAst {
    // --- Source identity ---
    /// Span table mapping SpanId → byte ranges (moved from ParseAst).
    pub span_table: SpanTable,
    /// The file identifier assigned during compilation coordination.
    pub file_id: FileId,

    // --- Declarations (name-keyed, owned structs) ---
    /// Resolved tag declarations.
    pub tags: HashMap<TagId, TypedTag>,
    /// Resolved bind (function/value) declarations.
    pub defs: HashMap<DefId, TypedBind>,
    /// Private tag names.
    pub private_tags: HashSet<TagId>,
    /// Private def names.
    pub private_defs: HashSet<DefId>,

    // --- Expression arena (soa_derive) ---
    /// The expression arena — all expressions in SoA layout.
    pub exprs: crate::typed::TypedExprVec,

    // --- Root expressions ---
    /// Top-level expression IDs (e.g., standalone expressions in the file).
    pub root_exprs: Vec<ExprId>,

    // --- Import resolution ---
    /// Resolved imports: symbol name → resolved target.
    pub resolved_imports: HashMap<Intern<String>, ResolvedImport>,

    // --- Position lookup ---
    /// span.start byte offset → ExprId for O(log n) position-based lookup.
    pub span_to_expr: BTreeMap<u32, ExprId>,

    // --- File-level derived index ---
    // (cache, deterministically reconstructible from declarations)
    /// Tag name → resolved type.
    pub tag_types: HashMap<TagId, Ty>,
    /// Function name → return type.
    pub fn_return_types: HashMap<DefId, Ty>,
    /// Variant name → [(union_name, discriminant, fields)].
    pub variant_map: crate::analysis::VariantMap,
    /// The marker registry — marker definitions and bindings for this package.
    pub marker_registry: MarkerRegistry,
    /// Declaration-level warnings (e.g., unrecognized custom markers).
    pub warnings: Vec<diagnostic::TypeSymptom>,
}

impl std::fmt::Debug for TypedFileAst {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypedFileAst")
            .field("file_id", &self.file_id)
            .field("tags", &self.tags)
            .field("defs", &self.defs)
            .field("private_tags", &self.private_tags)
            .field("private_defs", &self.private_defs)
            .field("exprs", &self.exprs)
            .field("root_exprs", &self.root_exprs)
            .field("resolved_imports", &self.resolved_imports)
            .field("span_to_expr", &self.span_to_expr)
            .field("tag_types", &self.tag_types)
            .field("fn_return_types", &self.fn_return_types)
            .field("variant_map", &self.variant_map)
            .finish()
    }
}

impl PartialEq for TypedFileAst {
    fn eq(&self, other: &Self) -> bool {
        self.file_id == other.file_id
            && self.tags == other.tags
            && self.defs == other.defs
            && self.private_tags == other.private_tags
            && self.private_defs == other.private_defs
            && self.exprs == other.exprs
            && self.root_exprs == other.root_exprs
            && self.resolved_imports == other.resolved_imports
            && self.span_to_expr == other.span_to_expr
            && self.tag_types == other.tag_types
            && self.fn_return_types == other.fn_return_types
            && self.variant_map == other.variant_map
    }
}

impl TypedFileAst {
    pub fn new(file_id: FileId, span_table: SpanTable) -> Self {
        Self {
            span_table,
            file_id,
            tags: HashMap::new(),
            defs: HashMap::new(),
            private_tags: HashSet::new(),
            private_defs: HashSet::new(),
            exprs: TypedExprVec::new(),
            root_exprs: Vec::new(),
            resolved_imports: HashMap::new(),
            span_to_expr: BTreeMap::new(),
            tag_types: HashMap::new(),
            fn_return_types: HashMap::new(),
            variant_map: HashMap::new(),
            marker_registry: MarkerRegistry::new(),
            warnings: Vec::new(),
        }
    }

    pub fn tag(&self, id: &TagId) -> Option<&TypedTag> {
        self.tags.get(id)
    }

    pub fn def(&self, id: &DefId) -> Option<&TypedBind> {
        self.defs.get(id)
    }

    pub fn expr(&self, id: ExprId) -> Option<TypedExprRef<'_>> {
        self.exprs.get(id.as_usize())
    }

    pub fn fn_return_type(&self, id: &DefId) -> Option<&Ty> {
        self.fn_return_types.get(id)
    }

    pub fn tag_type(&self, id: &TagId) -> Option<&Ty> {
        self.tag_types.get(id)
    }

    pub fn lookup_variant(
        &self,
        name: &Intern<String>,
    ) -> Option<&Vec<crate::analysis::VariantMapEntry>> {
        self.variant_map.get(name)
    }

    pub fn expr_at_byte(&self, byte_offset: u32) -> Option<ExprId> {
        self.span_to_expr
            .range(..=byte_offset)
            .next_back()
            .map(|(_, id)| *id)
    }

    pub fn expr_at_source_pos(&self, source: &str, line: u32, character: u32) -> Option<ExprId> {
        let byte_offset = crate::source::position_to_byte_offset(source, line, character)?;
        self.expr_at_byte(byte_offset as u32)
    }

    /// Get hover type information at a source position.
    /// Returns a string describing the type at that position, if available.
    /// Falls back to tag declaration info if no expression is at the position.
    pub fn hover_at(&self, source: &str, line: u32, character: u32) -> Option<String> {
        // First, try to find an expression at this position.
        if let Some(expr_id) = self.expr_at_source_pos(source, line, character)
            && let Some(expr_ref) = self.expr(expr_id)
        {
            let ty_str = format_ty_for_hover(expr_ref.ty);
            let is_copy = crate::analysis::is_copyable(expr_ref.ty, &self.marker_registry);
            let copy_badge = if is_copy {
                "[`Copy`](gin:copy)".to_string()
            } else {
                "[`not Copy`](gin:copy)".to_string()
            };
            let mut result = ty_str.clone();
            match expr_ref.kind {
                TypedExprKind::Bind { name, .. } => {
                    result = format!("`{}`: `{}` {}", name.as_str(), ty_str, copy_badge);
                }
                TypedExprKind::FnCall { target, .. } => {
                    let fn_name = target.0.as_str();
                    result = format!("`{}`: `{}` {}", fn_name, ty_str, copy_badge);
                }
                TypedExprKind::TagCall { variant_id, .. } => {
                    result = format!(
                        "`{}` (variant of `{}`) {}",
                        variant_id.name.as_str(),
                        variant_id.union.0.as_str(),
                        copy_badge
                    );
                }
                _ => {
                    result = format!("{} {}", ty_str, copy_badge);
                }
            }
            return Some(result);
        }

        // Fallback: check if the word at this position is a tag or variant name.
        let byte_offset = crate::source::position_to_byte_offset(source, line, character)?;
        let word = crate::source::word_at_byte_offset(source, byte_offset)?;
        let word_interned = Intern::new(word.clone());
        let tag_id = TagId(word_interned);

        if let Some(tag) = self.tags.get(&tag_id) {
            let mut result = format!("```gin\n{}\n```", tag.declaration_text);
            if let Some(ref doc) = tag.doc_comment {
                result = format!("{}\n---\n{}", result, doc.value);
            }
            // Show markers section — formatted as clickable links
            if !tag.marker_bindings.is_empty() {
                result.push_str("\n\n---\nMarkers");
                for binding in &tag.marker_bindings {
                    let prefix = if binding.positive { "" } else { "not " };
                    let marker = binding.marker_name.as_str();
                    // Link to the marker via gin_core module path convention
                    // The database layer replaces `gin:marker/Name` with a file:// URI.
                    result.push_str(&format!(
                        "\n  - [`{}{}`](gin:marker/{})",
                        prefix, marker, marker
                    ));
                }
            }
            // Show copyability badge.
            let size = crate::ty::ty_byte_size_static(&tag.resolved_ty);
            let align = crate::ty::ty_alignment(&tag.resolved_ty);
            let is_copy = crate::analysis::is_copyable(&tag.resolved_ty, &self.marker_registry);
            if is_copy {
                result.push_str("\n\n---\n\n[`Copy`](gin:copy)");
            } else {
                result.push_str("\n\n---\n\n[`not Copy`](gin:copy)");
            }

            // Show size/align when the type has a concrete layout.
            if size > 0 || align > 0 {
                result.push_str(&format!("\n\n---\n\nsize = {size}, align = {align}"));
            }
            return Some(result);
        }

        // Check if the word is a variant of any known union.
        if let Some(candidates) = self.variant_map.get(&word_interned)
            && let Some((union_name, _, _)) = candidates.first()
        {
            let ty_str = format_ty_for_hover(
                self.tag_types
                    .get(&TagId(*union_name))
                    .unwrap_or(&Ty::Opaque(*union_name)),
            );
            return Some(format!(
                "`{}` (variant of `{}`): {}",
                word,
                union_name.as_str(),
                ty_str
            ));
        }

        None
    }

    /// Resolve the type of a field access expression at a source position.
    /// For a position at `expr.field`, finds the expression before the dot,
    /// looks up its type, and if it's a Record, returns the field's type
    /// formatted via [`format_ty_for_hover`].
    pub fn dot_type(&self, source: &str, line: u32, character: u32) -> Option<String> {
        let byte_offset = crate::source::position_to_byte_offset(source, line, character)?;

        // Check that there's a dot before the cursor position.
        let dot_pos = byte_offset.checked_sub(1)?;
        if source.as_bytes().get(dot_pos) != Some(&b'.') {
            return None;
        }

        // Extract the field name (the word at the cursor position).
        let field_name = crate::source::word_at_byte_offset(source, byte_offset)?;

        // Find the expression whose span covers the dot position.
        let expr_id = self.expr_at_byte(dot_pos as u32)?;
        let expr_ref = self.expr(expr_id)?;

        // If the expression has a Record type, look up the field by name.
        match &expr_ref.ty {
            Ty::Record { fields, .. } => {
                let interned_field = Intern::<String>::from_ref(&field_name);
                for (name, ty) in fields {
                    if *name == interned_field {
                        return Some(format_ty_for_hover(ty));
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Get the definition span for a symbol at a position.
    /// Returns (start_byte, end_byte) of the definition.
    pub fn definition_span(
        &self,
        source: &str,
        line: u32,
        character: u32,
    ) -> Option<(usize, usize)> {
        let expr_id = self.expr_at_source_pos(source, line, character)?;
        let expr_ref = self.expr(expr_id)?;

        match expr_ref.kind {
            TypedExprKind::Bind { name, .. } => {
                for bind in self.defs.values() {
                    if bind.name == *name {
                        let span = self.span_table.get(bind.name_span);
                        return Some((span.start, span.end));
                    }
                }
                None
            }
            TypedExprKind::FnCall { target, .. } => {
                if let Some(bind) = self.defs.get(target) {
                    let span = self.span_table.get(bind.name_span);
                    return Some((span.start, span.end));
                }
                None
            }
            TypedExprKind::TagCall { .. } => None,
            _ => None,
        }
    }

    /// Collect all type flaws from the expression arena.
    pub fn all_flaws(&self) -> Vec<(ExprId, &diagnostic::TypeSymptom)> {
        let mut flaws = Vec::new();
        for i in 0..self.exprs.kind.len() {
            let expr_id = ExprId(i as u32);
            for flaw in &self.exprs.flaws[i] {
                flaws.push((expr_id, flaw));
            }
        }
        flaws
    }

    /// Collect declaration-level warnings (e.g., unrecognized markers).
    pub fn all_warnings(&self) -> &[diagnostic::TypeSymptom] {
        &self.warnings
    }
}

/// Format a `Ty` for hover display.
pub(crate) fn format_ty_for_hover(ty: &Ty) -> String {
    match ty {
        Ty::Int {
            width,
            signed,
            value,
        } => {
            let prefix = if *signed { "i" } else { "u" };
            if let Some(v) = value {
                format!("{}{} = {}", prefix, width, v)
            } else {
                format!("{}{}", prefix, width)
            }
        }
        Ty::Float { value } => {
            if let Some(v) = value {
                format!("f64 = {}", v)
            } else {
                "f64".to_string()
            }
        }
        Ty::Bool => "Bool".to_string(),
        Ty::Unit => "()".to_string(),
        Ty::Record { fields, .. } => {
            let parts: Vec<String> = fields
                .iter()
                .map(|(fname, fty)| format!("{}: {}", fname.as_str(), format_ty_for_hover(fty)))
                .collect();
            parts.join(", ")
        }
        Ty::Union { name, .. } => format!("Union({})", name.as_str()),
        Ty::Opaque(name) => name.as_str().to_string(),
        Ty::Array { elem, size } => format!("[{}; {}]", format_ty_for_hover(elem), size),
        Ty::Ptr { inner } => format!("*{}", format_ty_for_hover(inner)),
        Ty::Ref { inner, mutable } => {
            let prefix = if *mutable { "mut " } else { "ref " };
            format!("{}{}", prefix, format_ty_for_hover(inner))
        }
        Ty::Tuple(tys) => {
            let parts: Vec<String> = tys.iter().map(format_ty_for_hover).collect();
            format!("({})", parts.join(", "))
        }
        Ty::ConstUnion { name, .. } => format!("ConstUnion({})", name.as_str()),
    }
}
