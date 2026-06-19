use std::collections::{BTreeMap, HashMap, HashSet};

use ast::HashFloat;
use ast::parameter::{ParameterKind, Parameters};
use ast::prelude::*;
use ast::source::SourceExt;
use ast::span::{SpanId, SpanTable, SubSpan};
use diagnostic::Diagnostic;
use internment::Intern;

use crate::ty::Ty;

mod id;

pub use id::*;

/// Type alias for variant map entries: (union_name, discriminant, fields)
pub type VariantMapEntry = (Intern<String>, usize, Vec<(Intern<String>, Ty)>);

/// Type alias for the variant map: variant_name -> [(union_name, discriminant, fields)]
pub type VariantMap = HashMap<Intern<String>, Vec<VariantMapEntry>>;

/// Type alias for variant lookup result: (union_name, discriminant, field_slice)
pub type VariantLookupResult<'a> = (Intern<String>, usize, &'a [(Intern<String>, Ty)]);

/// Merge per-file variant maps once for package-scoped IDE / cross-file lowering.
///
/// Each file only stores variants for tags it declares.
pub fn collect_package_variant_map(asts: &[&TypedFileAst]) -> VariantMap {
    let mut seen: std::collections::HashSet<(Intern<String>, Intern<String>, usize)> =
        std::collections::HashSet::new();
    let mut variant_map: VariantMap = HashMap::new();
    for ast in asts {
        for (variant_name, entries) in &ast.variant_map {
            for entry in entries {
                let (union_name, disc, _) = entry;
                if !seen.insert((*variant_name, *union_name, *disc)) {
                    continue;
                }
                variant_map
                    .entry(*variant_name)
                    .or_default()
                    .push(entry.clone());
            }
        }
    }
    variant_map
}

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
    pub const_value: Option<ast::ConstValue>,
    /// Type/flow/flaw diagnostics attached to this expression.
    pub flaws: Vec<Diagnostic>,
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
        pattern: Box<ast::span::Spanned<TypeExpr>>,
        body: ExprId,
        /// Span of this is-arm (pattern and body).
        arm_span: SubSpan,
    },
    Else(ExprId, SubSpan),
}

/// Typed if-expression — like `IfExpr` but with `ExprId` children.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedIfExpr {
    pub subject: ExprId,
    /// Parsed `is …` pattern — structural [`TypeExpr`] (`Nominal` / `Qualified` / `Generic`).
    pub pattern: Box<ast::span::Spanned<TypeExpr>>,
    pub stmts: Vec<ExprId>,
    pub ret: Option<ExprId>,
    /// Covers from condition start to end (excludes the `if` keyword).
    /// The full expression span (including `if`) is on the `TypedExpr` arena entry.
    pub body_span: SubSpan,
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
        /// Original field names from the AST TagCall args (for named/positional
        /// args in shape literals). Empty for bare anonymous tags.
        field_names: Vec<Intern<String>>,
    },
    Bind {
        name: Intern<String>,
        stmts: Vec<ExprId>,
        body: ExprId,
        /// If true, this bind was declared with a type but no value (`name Type`).
        /// The variable starts in `Declared` state and must be assigned before use.
        unassigned: bool,
    },
    /// Reassign a value to a previously-declared variable.
    Reassign {
        name: Intern<String>,
        value: ExprId,
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
    /// Destructure bind: `Tag(field: bind, …) := expr`
    Destructure {
        value: ExprId,
        field_bindings: Vec<(Intern<String>, Intern<String>)>,
    },

    /// Record field write: `base.field: value`
    RecordSet {
        base: ExprId,
        field: Intern<String>,
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

    /// Argument passed with `eat` at call site: explicit consume.
    ConsumeArg(ExprId),
    /// Explicit consume: `eat expr`.
    Eat(ExprId),

    Asm(AsmExpr),
}

/// A fully-resolved tag declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedTag {
    pub name_span: SpanId,
    /// Full `Tag is …` / `Tag has …` site (for hover on the RHS, e.g. `in 0...255`).
    pub span: SpanId,
    /// The resolved type of this tag (e.g., `Ty::Union`, `Ty::Record`, `Ty::ConstUnion`, etc.).
    pub resolved_ty: Ty,
    /// Tag attributes (e.g., `#[test]`).
    pub attributes: DeclareAttributes,
    pub doc_comment: Option<DocComment>,
    /// Tag parameters (type variables, defaults), if any.
    pub params: Option<Parameters>,
    /// For record types (`has` bodies), field name → formatted type annotation surface
    /// (e.g. `"pointer"` → `"Pointer(x)"`). Populated during stage_declare.
    pub record_field_types: HashMap<Intern<String>, String>,
    /// For interface (`has`) bodies, method name → doc comment, if any.
    /// Only entries for members that have a doc are present.
    /// Populated during stage_declare alongside `record_field_types`.
    pub record_field_docs: HashMap<Intern<String>, String>,
    /// Formatted declaration text (e.g. "Bool is True or False"), for use in hover.
    pub declaration_text: String,
    /// Trait implementations provided via `and has TraitName(field: expr, ...)` clauses.
    /// Explicit (concrete) field definitions always win over these provided ones.
    pub provided_traits: Vec<ProvidedTrait>,
}

/// A fully-resolved bind (function or value definition).
#[derive(Debug, Clone, PartialEq)]
pub struct TypedBind {
    pub name: Intern<String>,
    /// Span of the name in source.
    pub name_span: SpanId,
    pub body: BindBody,
    /// The resolved return type (may include threaded params after desugaring).
    pub return_type: Ty,
    /// User-written return type before linear threading desugar.
    pub declared_return_type: Ty,
    /// Resolved parameter types.
    pub params: Vec<(Intern<String>, Ty)>,
    /// Receiver type for methods, if any.
    pub receiver_type: Option<Ty>,
    /// Bind attributes (e.g., `#[inline]`, visibility).
    pub attributes: BindAttributes,
    pub doc_comment: Option<DocComment>,
    /// Bind-level diagnostics (e.g., redundant self-param type).
    pub flaws: Vec<Diagnostic>,
    /// `name Type` at module or function scope with no `:` value yet.
    pub unassigned_decl: bool,
    /// Bound with `:=` (immutable).
    pub is_constant: bool,
    /// Comptime-classified; skipped by runtime flow when true.
    pub is_compile_time: bool,
    /// Source-level signature for hover (types as written, not resolved).
    pub signature_surface: String,
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
    /// Span table mapping SpanId → byte ranges (cloned from FileAst).
    pub span_table: SpanTable,
    /// The file identifier assigned during compilation coordination.
    pub file_id: FileId,

    /// Resolved tag declarations.
    pub tags: HashMap<TagId, TypedTag>,
    /// Resolved bind (function/value) declarations.
    pub defs: HashMap<DefId, TypedBind>,
    /// Private tag names.
    pub private_tags: HashSet<TagId>,
    /// Private def names.
    pub private_defs: HashSet<DefId>,

    /// The expression arena — all expressions in SoA layout.
    pub exprs: TypedExprVec,

    /// Top-level expression IDs (e.g., standalone expressions in the file).
    pub root_exprs: Vec<ExprId>,

    /// Resolved imports: symbol name → resolved target.
    pub resolved_imports: HashMap<Intern<String>, ResolvedImport>,

    /// span.start byte offset → ExprId for O(log n) position-based lookup.
    pub span_to_expr: BTreeMap<u32, ExprId>,

    /// Raw import ModPaths from the source file's import statements.
    /// Used for module path hover detection (non-final segment → module doc).
    pub import_mod_paths: Vec<ast::Spanned<ast::ModPath>>,

    // (cache, deterministically reconstructible from declarations)
    /// Tag name → resolved type.
    pub tag_types: HashMap<TagId, Ty>,
    /// Function name → return type.
    pub fn_return_types: HashMap<DefId, Ty>,
    /// Variant name → [(union_name, discriminant, fields)].
    pub variant_map: VariantMap,
    /// Display text and doc comment for each variant, keyed by `"{union_tag}.{variant_name}"`.
    /// Populated during stage_declare from the AST `Variant` shapes.
    pub variant_annotations: HashMap<String, (String, Option<String>)>,
    /// Blanket compile-time trait impls from `x has Trait(...)`.
    pub blanket_impls: Vec<ast::BlanketImpl>,
    pub imported_trait_names: HashSet<Intern<String>>,
    pub eval_ast: std::sync::Arc<ast::FileAst>,
    /// Declaration-level warnings.
    pub warnings: Vec<Diagnostic>,
    /// Type-name flaws on declarations (`has` fields, return types, etc.).
    pub declaration_flaws: Vec<(SpanId, Diagnostic)>,
    /// Parse-level diagnostics from the parser (cursor errors, lex errors).
    pub parse_warnings: Vec<diagnostic::Diagnostic>,
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
            && self.variant_annotations == other.variant_annotations
    }
}

#[derive(Copy, Clone)]
struct HoverPatternCtx<'a> {
    byte_offset: usize,
    word: &'a str,
    subject_ty: Option<&'a Ty>,
    tag_types: &'a HashMap<Intern<String>, Ty>,
    tag_params: Option<&'a HashMap<Intern<String>, Parameters>>,
    variant_map: &'a VariantMap,
    package: Option<&'a PackageSemanticIndex>,
}

/// What kind of thing is at a cursor position — used by [`TypedFileAst::classify_hover`].
#[derive(Debug, Clone)]
#[doc(hidden)]
pub enum HoverTarget {
    Definition(Intern<String>),
    Param {
        name: Intern<String>,
        surface: String,
    },
    TagDecl(Intern<String>),
    TagAtByte(Intern<String>),
    TypePattern(HoverResult),
    Variant {
        union_name: Intern<String>,
        discriminant: usize,
        union_ty: Ty,
    },
    Expr(ExprId),
    /// Cursor is on a non-final segment of a qualified module path.
    /// The string is the qualified module path (e.g. `"core.maybe"`).
    ModulePath(String),
}

impl TypedFileAst {
    fn param_at_byte(
        &self,
        source: &str,
        byte_offset: usize,
        word: &str,
    ) -> Option<(Intern<String>, String)> {
        let word = Intern::<String>::from_ref(word);
        for ast_bind in self.eval_ast.defs.values() {
            let params = ast_bind.params.as_ref();
            let source_surface =
                source_param_surface(source, &self.span_table, ast_bind, word.as_str());
            if source_surface.is_none() && !params.is_some_and(|params| params.contains_key(&word))
            {
                continue;
            }

            let surface = source_surface.or_else(|| {
                self.defs
                    .get(&DefId(ast_bind.name))
                    .or_else(|| {
                        self.defs.values().find(|bind| {
                            bind.name.as_str() == ast_bind.name.as_str()
                                || bind.name.as_str().split('.').next_back()
                                    == Some(ast_bind.name.as_str())
                        })
                    })
                    .and_then(|typed_bind| {
                        source_param_surface(source, &self.span_table, ast_bind, word.as_str())
                            .or_else(|| {
                                signature_param_surface(
                                    &typed_bind.signature_surface,
                                    word.as_str(),
                                )
                            })
                            .or_else(|| {
                                typed_bind.params.iter().find_map(|(name, ty)| {
                                    (*name == word).then(|| {
                                        let tag_types = self.tag_types_by_name();
                                        let tag_params = self.tag_params_by_name();
                                        type_annotation_surface_for_hover(
                                            ty,
                                            &tag_types,
                                            Some(&tag_params),
                                        )
                                    })
                                })
                            })
                    })
                    .or_else(|| {
                        params
                            .and_then(|params| params.get(&word).map(param_kind_surface_for_hover))
                    })
            })?;

            if let Some(span) = self.inferred_param_name_span(source, ast_bind, word.as_str())
                && span.contains(&byte_offset)
            {
                return Some((word, surface));
            }

            if self.bind_body_contains(ast_bind, byte_offset) {
                return Some((word, surface));
            }
        }
        None
    }

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
            variant_annotations: HashMap::new(),
            blanket_impls: Vec::new(),
            imported_trait_names: HashSet::new(),
            eval_ast: std::sync::Arc::new(ast::FileAst::empty_for_tests()),
            warnings: Vec::new(),
            declaration_flaws: Vec::new(),
            import_mod_paths: Vec::new(),
            parse_warnings: Vec::new(),
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

    pub fn lookup_variant(&self, name: &Intern<String>) -> Option<&Vec<VariantMapEntry>> {
        self.variant_map.get(name)
    }

    pub fn expr_at_byte(&self, byte_offset: u32) -> Option<ExprId> {
        let pos = byte_offset as usize;
        let mut best: Option<(usize, ExprId)> = None;
        for (i, span_id) in self.exprs.span.iter().enumerate() {
            if !span_id.is_valid() {
                continue;
            }
            let span = self.span_table.get(*span_id);
            if span.contains(pos) && best.is_none_or(|(best_len, _)| span.len() < best_len) {
                best = Some((span.len(), ExprId(i as u32)));
            }
        }
        best.map(|(_, id)| id)
    }

    /// Tag whose declare site contains `byte_offset` (smallest span wins).
    ///
    /// Skips when the cursor is on a definition name or on another tag's name.
    /// Used for hovers on declare RHS (`in 0...255`, union variants, etc.).
    fn tag_at_byte(&self, byte_offset: usize, word: &str) -> Option<(&TagId, &TypedTag)> {
        if self.defs.values().any(|b| {
            b.name.as_str() == word && self.span_table.get(b.name_span).contains(byte_offset)
        }) {
            return None;
        }

        let mut best: Option<(&TagId, &TypedTag, usize)> = None;
        for (id, tag) in &self.tags {
            if !tag.span.is_valid() {
                continue;
            }
            let full = self.span_table.get(tag.span);
            if !full.contains(byte_offset) {
                continue;
            }
            let name = self.span_table.get(tag.name_span);
            if name.contains(byte_offset) && id.0.as_str() != word {
                continue;
            }
            if self.tags.iter().any(|(other, t)| {
                other != id
                    && other.0.as_str() == word
                    && self.span_table.get(t.name_span).contains(byte_offset)
            }) {
                continue;
            }
            let len = full.len();
            if best.is_none_or(|(_, _, bl)| len < bl) {
                best = Some((id, tag, len));
            }
        }
        best.map(|(id, tag, _)| (id, tag))
    }

    pub fn expr_at_source_pos(&self, source: &str, line: u32, character: u32) -> Option<ExprId> {
        let byte_offset = source.position_to_byte_offset(line, character)?;
        self.expr_at_byte(byte_offset as u32)
    }

    /// Check if the cursor is on a non-final segment of a qualified module path
    /// in any import statement. Returns the qualified module path (e.g. `"core.maybe"`).
    fn module_path_at_byte(&self, byte_offset: usize) -> Option<String> {
        for path in &self.import_mod_paths {
            let mp = &path.value;
            // Check root span (when there are segments)
            if !mp.segments.is_empty() && self.span_table.get(mp.root_span).contains(byte_offset) {
                return Some(mp.root.to_string());
            }
            // Check each segment span (non-final segments only)
            for (i, seg_span) in mp.segment_spans.iter().enumerate() {
                if i < mp.segments.len() - 1 && self.span_table.get(*seg_span).contains(byte_offset)
                {
                    let mut module = mp.root.to_string();
                    for j in 0..=i {
                        module.push('.');
                        module.push_str(mp.segments[j].as_str());
                    }
                    return Some(module);
                }
            }
        }
        None
    }

    /// Hover markdown for a resolved tag declaration.
    ///
    /// TODO: Add a "traits" section after the declaration text to show which
    /// interfaces/traits this type implements, e.g.:
    ///
    ///   ```gin
    ///   Bool is True or False
    ///   ```
    ///   ---
    ///   has [Happy](...), [ToString](...)
    ///   ---
    ///   Bool represents a value...
    ///
    /// The data is already available on `tag.provided_traits` (each `ProvidedTrait`
    /// has `trait_name` and `fields`). Use `tag.declaration_text` for the type
    /// signature block, then insert a `HoverSection::Prose` (or a new section variant)
    /// that lists trait names and links to their declarations. Also consider:
    ///
    /// - Auto-synthesized traits (e.g. `Reflectable` via `synthesize_reflectable_trait`)
    ///   — include them but perhaps distinguish from user-written `and has` clauses.
    /// - Blanket implementations from the package index (`PackageSemanticIndex`)
    ///   that apply to this type.
    fn hover_for_tag(&self, tag: &TypedTag) -> String {
        ast::hover_format::HoverDoc::new()
            .gin(&tag.declaration_text)
            .doc_opt(tag.doc_comment.as_ref())
            .render()
    }

    /// Hover for a union / const-union variant name.
    fn union_name_for_variant(
        &self,
        variant_map: &VariantMap,
        variant_name: &str,
    ) -> Option<Intern<String>> {
        variant_map
            .get(&Intern::<String>::from_ref(variant_name))
            .and_then(|entries| entries.first().map(|(union, _, _)| *union))
    }

    fn variant_pattern_label(
        &self,
        variant_map: &VariantMap,
        union_name: &str,
        variant_name: &str,
        tag_types: &HashMap<Intern<String>, Ty>,
        tag_params: Option<&HashMap<Intern<String>, Parameters>>,
    ) -> String {
        let key = Intern::<String>::from_ref(variant_name);
        if let Some(entries) = variant_map.get(&key) {
            let union_key = Intern::<String>::from_ref(union_name);
            if let Some((_, _, fields)) = entries.iter().find(|(u, _, _)| *u == union_key) {
                return format_variant_pattern_label(variant_name, fields, tag_types, tag_params);
            }
            if let Some((_, _, fields)) = entries.first() {
                return format_variant_pattern_label(variant_name, fields, tag_types, tag_params);
            }
        }
        variant_name.to_string()
    }

    fn tag_params_by_name(&self) -> HashMap<Intern<String>, Parameters> {
        self.tags
            .iter()
            .filter_map(|(id, tag)| tag.params.clone().map(|p| (id.0, p)))
            .collect()
    }

    fn hover_for_variant(&self, union_name: &str, variant_label: &str, _union_ty: &Ty) -> String {
        // Use just the base variant name (before any `(`) for annotation lookup.
        let base_name = variant_label.split('(').next().unwrap_or(variant_label);
        let annotation_key = format!("{union_name}.{base_name}");
        let doc_text = self
            .variant_annotations
            .get(&annotation_key)
            .and_then(|(_, doc)| doc.clone());
        let mut h = ast::hover_format::HoverDoc::new().gin(variant_label);
        if let Some(ref text) = doc_text {
            h = h.prose(text);
        }
        h.render()
    }

    fn tag_types_by_name(&self) -> HashMap<Intern<String>, Ty> {
        self.tag_types
            .iter()
            .map(|(id, ty)| (id.0, ty.clone()))
            .collect()
    }

    fn span_contains(&self, span_id: SpanId, byte_offset: usize) -> bool {
        self.span_table.contains(span_id, byte_offset)
    }

    /// When the subject is an opaque tag name (e.g. `Type` before cross-file union resolve), use `tag_types`.
    fn resolve_pattern_subject_ty(&self, ty: &Ty, package: Option<&PackageSemanticIndex>) -> Ty {
        if let Ty::Opaque(name) = ty
            && let Some(resolved) = package
                .and_then(|p| p.tag_types.get(name))
                .or_else(|| self.tag_types.get(&TagId(*name)))
            && !matches!(resolved, Ty::Opaque(_))
        {
            return resolved.clone();
        }
        ty.clone()
    }

    fn hover_for_union_variant_word(
        &self,
        word: &str,
        variant_map: &VariantMap,
        package: Option<&PackageSemanticIndex>,
    ) -> Option<HoverResult> {
        let key = Intern::<String>::from_ref(word);
        let entries = variant_map.get(&key)?;
        let (union_name, _, _) = entries.first()?;
        let union_ty = package
            .and_then(|p| p.tag_types.get(union_name))
            .cloned()
            .or_else(|| self.tag_types.get(&TagId(*union_name)).cloned())
            .unwrap_or(Ty::Opaque(*union_name));
        let owned_tag_types;
        let tag_types = match package {
            Some(p) => &p.tag_types,
            None => {
                owned_tag_types = self.tag_types_by_name();
                &owned_tag_types
            }
        };
        let owned_tag_params;
        let tag_params = match package {
            Some(p) => Some(&p.tag_params),
            None => {
                owned_tag_params = self.tag_params_by_name();
                Some(&owned_tag_params)
            }
        };
        let label = self.variant_pattern_label(
            variant_map,
            union_name.as_str(),
            word,
            tag_types,
            tag_params,
        );
        Some(
            HoverResult::single(self.hover_for_variant(union_name.as_str(), &label, &union_ty))
                .with_qualified_union(package, union_name),
        )
    }

    fn hover_type_expr_pattern(
        &self,
        pattern: &TypeExpr,
        ctx: HoverPatternCtx<'_>,
    ) -> Option<HoverResult> {
        match pattern {
            TypeExpr::Generic {
                name,
                params,
                param_spans,
                span,
                ..
            } => {
                let head_span = self.span_table.get(*span);
                if head_span.start() <= ctx.byte_offset
                    && ctx.byte_offset < head_span.end()
                    && ctx.word == name.as_str()
                {
                    let union_key = match ctx.subject_ty {
                        Some(Ty::Union { name, .. }) => Some(*name),
                        _ => self.union_name_for_variant(ctx.variant_map, name.as_str()),
                    };
                    if let Some(union_key) = union_key {
                        let union_name = union_key.as_str();
                        let label = self.variant_pattern_label(
                            ctx.variant_map,
                            union_name,
                            name.as_str(),
                            ctx.tag_types,
                            ctx.tag_params,
                        );
                        let union_ty = ctx
                            .subject_ty
                            .cloned()
                            .or_else(|| {
                                ctx.package
                                    .and_then(|p| p.tag_types.get(&union_key))
                                    .cloned()
                                    .or_else(|| self.tag_types.get(&TagId(union_key)).cloned())
                            })
                            .unwrap_or(Ty::Opaque(union_key));
                        return Some(
                            HoverResult::single(
                                self.hover_for_variant(union_name, &label, &union_ty),
                            )
                            .with_qualified_union(ctx.package, &union_key),
                        );
                    }
                    return Some(HoverResult::single(format!("```gin\n{name}\n```")));
                }
                for (slot, (pname, pspan)) in param_spans.iter().enumerate() {
                    let ps = self.span_table.get(*pspan);
                    if ps.start() <= ctx.byte_offset
                        && ctx.byte_offset < ps.end()
                        && (ctx.word == pname.as_str()
                            || (pname.is_pattern_wildcard() && ctx.word == "_"))
                    {
                        if let Some(kind) = params.get(slot).map(|(_, k)| k)
                            && let ParameterKind::Tagged(sp) = kind
                            && sp.value.denotes_variant_name(ctx.word)
                            && let Some(h) = self.hover_for_union_variant_word(
                                ctx.word,
                                ctx.variant_map,
                                ctx.package,
                            )
                        {
                            return Some(h);
                        }
                        let ty_str = pattern
                            .pattern_param_type_at_slot(
                                slot,
                                ctx.subject_ty,
                                ctx.variant_map,
                                ctx.tag_types,
                            )
                            .map(|t| {
                                pattern_binding_type_surface_for_hover(
                                    &t,
                                    ctx.tag_types,
                                    ctx.tag_params,
                                )
                            })
                            .unwrap_or_else(|| "infer".to_string());
                        let summary = if pname.is_pattern_wildcard() {
                            format!("_: `{ty_str}`")
                        } else {
                            format!("{} {}", ctx.word, ty_str)
                        };
                        return Some(HoverResult::single(format!("```gin\n{summary}\n```")));
                    }
                }
            }
            TypeExpr::ListCons { head, tail } => {
                let elem_ty = ctx.subject_ty.and_then(|ty| ty.list_elem_ty(ctx.tag_types));
                let head_ctx = HoverPatternCtx {
                    subject_ty: elem_ty.as_ref(),
                    ..ctx
                };
                if let Some(h) = self.hover_type_expr_pattern(&head.value, head_ctx) {
                    return Some(h);
                }
                return self.hover_type_expr_pattern(&tail.value, ctx);
            }
            TypeExpr::Tuple(elems) => {
                for e in elems {
                    if let Some(h) = self.hover_type_expr_pattern(&e.value, ctx) {
                        return Some(h);
                    }
                }
            }
            TypeExpr::Nominal(name, span)
                if !name.is_pattern_wildcard()
                    && name
                        .as_str()
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_lowercase()) =>
            {
                let name_span = self.span_table.get(*span);
                if name_span.start() <= ctx.byte_offset
                    && ctx.byte_offset < name_span.end()
                    && ctx.word == name.as_str()
                {
                    let ty_str = ctx
                        .subject_ty
                        .map(|t| {
                            pattern_binding_type_surface_for_hover(t, ctx.tag_types, ctx.tag_params)
                        })
                        .unwrap_or_else(|| "infer".to_string());
                    let summary = format!("{} {}", ctx.word, ty_str);
                    return Some(HoverResult::single(format!("```gin\n{summary}\n```")));
                }
            }
            _ => {}
        }
        None
    }

    fn hover_type_pattern_at(
        &self,
        byte_offset: usize,
        word: &str,
        package: Option<&PackageSemanticIndex>,
    ) -> Option<HoverResult> {
        let owned_tag_types;
        let tag_types = match package {
            Some(p) => &p.tag_types,
            None => {
                owned_tag_types = self.tag_types_by_name();
                &owned_tag_types
            }
        };
        let owned_tag_params;
        let tag_params = match package {
            Some(p) => Some(&p.tag_params),
            None => {
                owned_tag_params = self.tag_params_by_name();
                Some(&owned_tag_params)
            }
        };
        let variant_map = package.map(|p| &p.variant_map).unwrap_or(&self.variant_map);
        let pattern_ctx = HoverPatternCtx {
            byte_offset,
            word,
            subject_ty: None,
            tag_types,
            tag_params,
            variant_map,
            package,
        };
        for i in 0..self.exprs.kind.len() {
            match &self.exprs.kind[i] {
                TypedExprKind::When(w) => {
                    let subject_ty = w.subject.as_ref().map(|id| {
                        self.resolve_pattern_subject_ty(&self.exprs.ty[id.as_usize()], package)
                    });
                    let subject_ty = subject_ty.as_ref();
                    let pattern_ctx = HoverPatternCtx {
                        subject_ty,
                        ..pattern_ctx
                    };
                    for arm in &w.arms {
                        if let TypedWhenArm::Is { pattern, .. } = arm
                            && self.span_contains(pattern.span_id, byte_offset)
                            && let Some(h) =
                                self.hover_type_expr_pattern(&pattern.value, pattern_ctx)
                        {
                            return Some(h);
                        }
                    }
                }
                TypedExprKind::If(if_expr)
                    if self.span_contains(if_expr.pattern.span_id, byte_offset) =>
                {
                    let subject_ty = Some(self.resolve_pattern_subject_ty(
                        &self.exprs.ty[if_expr.subject.as_usize()],
                        package,
                    ));
                    let subject_ty = subject_ty.as_ref();
                    let pattern_ctx = HoverPatternCtx {
                        subject_ty,
                        ..pattern_ctx
                    };
                    if let Some(h) =
                        self.hover_type_expr_pattern(&if_expr.pattern.value, pattern_ctx)
                    {
                        return Some(h);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn param_surface_in_body(
        &self,
        source: &str,
        byte_offset: usize,
        word: &str,
    ) -> Option<String> {
        self.eval_ast.defs.values().find_map(|bind| {
            if !self.bind_body_contains(bind, byte_offset) {
                return None;
            }
            source_param_surface(source, &self.span_table, bind, word)
        })
    }

    fn bind_body_contains(&self, bind: &ast::Bind, byte_offset: usize) -> bool {
        match &bind.value {
            ast::BindValue::Expr(expr) => self.span_table.get(expr.span_id).contains(byte_offset),
            ast::BindValue::Body { exprs, ret } => {
                exprs
                    .iter()
                    .any(|expr| self.span_table.get(expr.span_id).contains(byte_offset))
                    || self.span_table.get(ret.span_id).contains(byte_offset)
            }
            ast::BindValue::Extern | ast::BindValue::Unassigned => false,
        }
    }

    fn inferred_param_name_span(
        &self,
        source: &str,
        bind: &ast::Bind,
        param_name: &str,
    ) -> Option<std::ops::Range<usize>> {
        let start = self.span_table.get(bind.name_span).end();
        let end = match &bind.value {
            ast::BindValue::Expr(expr) => self.span_table.get(expr.span_id).start(),
            ast::BindValue::Body { exprs, ret } => exprs
                .first()
                .map(|expr| self.span_table.get(expr.span_id).start())
                .unwrap_or_else(|| self.span_table.get(ret.span_id).start()),
            ast::BindValue::Extern | ast::BindValue::Unassigned => start,
        };
        if start >= end {
            return None;
        }
        for (name, _) in bind.params.as_ref()? {
            let name = name.as_str();
            let offset = find_param_name_between(source, start, end, name)?;
            if name == param_name {
                return Some(offset..offset + name.len());
            }
        }
        None
    }

    /// Hover for a definition name (`arch` in `arch Architecture`, or a function def).
    fn parent_bind_name_for_body(&self, body: ExprId) -> Option<Intern<String>> {
        self.exprs.kind.iter().find_map(|kind| match kind {
            TypedExprKind::Bind {
                name,
                body: bind_body,
                unassigned: false,
                ..
            } if *bind_body == body => Some(*name),
            _ => None,
        })
    }

    fn hover_for_def(&self, bind: &TypedBind) -> String {
        ast::hover_format::HoverDoc::new()
            .gin(self.hover_signature_with_const(bind))
            .doc_opt(bind.doc_comment.as_ref())
            .render()
    }

    /// Signature surface plus the const RHS (e.g. `false Bool.False`) when the
    /// bind has no written type and the body folded to a constant.
    fn hover_signature_with_const(&self, bind: &TypedBind) -> String {
        let mut sig = bind.signature_surface.clone();
        if sig != bind.name.as_str() {
            return sig;
        }
        if bind.unassigned_decl {
            return sig;
        }
        let BindBody::Expr(expr_id) = &bind.body else {
            return sig;
        };
        let Some(expr) = self.expr(*expr_id) else {
            return sig;
        };
        let Some(const_val) = expr.const_value else {
            return sig;
        };
        sig.push(' ');
        sig.push_str(&const_val.to_hover_string());
        sig
    }

    /// Get hover type information at a source position.
    /// Returns a string describing the type at that position, if available.
    pub fn hover_at(&self, source: &str, line: u32, character: u32) -> Option<String> {
        self.hover_at_with_package(source, line, character, None)
            .map(|r| r.markdown)
    }

    /// Like [`hover_at`](Self::hover_at) but uses a pre-built [`PackageSemanticIndex`] (no AST clone).
    /// Classify the thing at the cursor position into a [`HoverTarget`].
    #[doc(hidden)]
    pub fn classify_hover(
        &self,
        source: &str,
        byte_offset: usize,
        word: &str,
        package: Option<&PackageSemanticIndex>,
    ) -> Option<HoverTarget> {
        let word_interned = Intern::<String>::from_ref(word);
        let tag_id = TagId(word_interned);

        // 1. Cursor is exactly on a definition name.
        if self.defs.values().any(|b| {
            b.name.as_str() == word && self.span_table.get(b.name_span).contains(byte_offset)
        }) {
            return Some(HoverTarget::Definition(TagId(word_interned).0));
        }

        // 1a. Cursor is on a function parameter declaration or a use in that function body.
        if let Some((name, surface)) = self.param_at_byte(source, byte_offset, word) {
            return Some(HoverTarget::Param { name, surface });
        }

        // 2. Cursor is exactly on a tag declaration name.
        if let Some(tag) = self.tags.get(&tag_id) {
            if self.span_table.get(tag.name_span).contains(byte_offset) {
                return Some(HoverTarget::TagDecl(tag_id.0));
            }
            // 2a. Tag name referenced in an expression (e.g. `Bool` in `Bool.False`
            //     or a type annotation). Route to tag declaration hover.
            if let Some(expr_id) = self.expr_at_byte(byte_offset as u32)
                && let Some(expr_ref) = self.expr(expr_id)
            {
                let is_tag_ref = match &expr_ref.kind {
                    TypedExprKind::TagCall { variant_id, .. } => {
                        variant_id.union.0 == word_interned
                    }
                    _ => false,
                };
                if is_tag_ref {
                    return Some(HoverTarget::TagDecl(tag_id.0));
                }
            }
        } else if let Some(pkg) = package
            && pkg.tag_decl_for_word(&word_interned).is_some()
        {
            // Tag declared in another file.
            return Some(HoverTarget::TagDecl(tag_id.0));
        }

        // Literal expressions (like `'x86_64'`) can share their spelling with
        // literal-union variant names. Check if this word matches a variant before
        // falling through to the generic Expr classification.
        // Skip if the word is also a top-level definition — def hover wins or
        // the expression classification (step 6) handles it more informatively.
        let variant_map = package.map(|p| &p.variant_map).unwrap_or(&self.variant_map);
        if !self.defs.contains_key(&DefId(word_interned))
            && let Some(candidates) = variant_map.get(&word_interned)
            && let Some((union_name, discriminant, _)) = candidates.first()
        {
            return Some(HoverTarget::Variant {
                union_name: *union_name,
                discriminant: *discriminant,
                union_ty: Ty::Opaque(*union_name),
            });
        }

        // 3. Cursor is on a union variant name (checked before tag body
        //    so variants in union declarations get the variant-specific hover).
        //    Skip if the word is also a top-level definition — def hover wins.
        let variant_map = package.map(|p| &p.variant_map).unwrap_or(&self.variant_map);
        if !self.defs.contains_key(&DefId(word_interned))
            && let Some(candidates) = variant_map.get(&word_interned)
            && let Some((union_name, discriminant, _)) = candidates.first()
        {
            return Some(HoverTarget::Variant {
                union_name: *union_name,
                discriminant: *discriminant,
                union_ty: Ty::Opaque(*union_name),
            });
        }

        // 4. Word is a known tag name referenced inside a type annotation or tag
        //    body (e.g. `Bool` in `has (b Bool)` or `Pointer` in `pointer Pointer(x)`).
        //    Route to tag declaration directly — checked before `tag_at_byte` so
        //    it wins over the containing tag's body hover.
        if self.tags.contains_key(&tag_id) {
            return Some(HoverTarget::TagDecl(tag_id.0));
        }
        if let Some(pkg) = package
            && pkg.tag_decl_for_word(&word_interned).is_some()
        {
            // Cross-file tag reference in a type position.
            return Some(HoverTarget::TagDecl(tag_id.0));
        }

        // 5. Cursor is inside a tag declaration body (not the name or a variant).
        if let Some((tag_id, _)) = self.tag_at_byte(byte_offset, word) {
            return Some(HoverTarget::TagAtByte(tag_id.0));
        }

        // 6. Cursor is on a type pattern in a when/is arm.
        if let Some(h) = self.hover_type_pattern_at(byte_offset, word, package) {
            return Some(HoverTarget::TypePattern(h));
        }

        // 7. Cursor is on a non-final segment of a qualified module path (import).
        if let Some(module) = self.module_path_at_byte(byte_offset) {
            return Some(HoverTarget::ModulePath(module));
        }

        // 8. Cursor is on an expression.
        if let Some(expr_id) = self.expr_at_byte(byte_offset as u32) {
            return Some(HoverTarget::Expr(expr_id));
        }

        None
    }

    pub fn hover_at_with_package(
        &self,
        source: &str,
        line: u32,
        character: u32,
        package: Option<&PackageSemanticIndex>,
    ) -> Option<HoverResult> {
        let byte_offset = source.position_to_byte_offset(line, character)?;
        if let Some((name, surface)) = source_param_at_byte(source, byte_offset) {
            return Some(HoverResult::single(
                ast::hover_format::HoverDoc::new()
                    .gin(format!("{name} {surface}"))
                    .render(),
            ));
        }
        let word = source
            .symbol_at_byte_offset(byte_offset)
            .or_else(|| source.word_at_byte_offset(byte_offset))?;

        let target = self.classify_hover(source, byte_offset, &word, package)?;

        self.render_hover(source, target, &word, byte_offset, package)
    }

    /// Render markdown for a classified [`HoverTarget`].
    fn render_hover(
        &self,
        source: &str,
        target: HoverTarget,
        word: &str,
        byte_offset: usize,
        package: Option<&PackageSemanticIndex>,
    ) -> Option<HoverResult> {
        match target {
            HoverTarget::Definition(name) => {
                let bind = self.defs.get(&DefId(name))?;
                let result = HoverResult::single(self.hover_for_def(bind));
                Some(result.with_def_module(package, &name))
            }
            HoverTarget::Param { name, surface } => Some(HoverResult::single(
                ast::hover_format::HoverDoc::new()
                    .gin(format!("{} {}", name.as_str(), surface))
                    .render(),
            )),
            HoverTarget::TagDecl(name) => {
                let tag_id = TagId(name);
                // Try local tags first, then fall back to cross-file package index.
                if let Some(tag) = self.tags.get(&tag_id) {
                    let result = HoverResult::single(self.hover_for_tag(tag));
                    return Some(result.with_tag_module(package, &tag_id.0));
                }
                if let Some(pkg) = package
                    && let Some((_module, tag)) = pkg.tag_decl_for_word(&name)
                {
                    let result = HoverResult::single(self.hover_for_tag(tag));
                    return Some(result.with_tag_module(package, &name));
                }
                None
            }
            HoverTarget::TagAtByte(name) => {
                let tag_id = TagId(name);
                let tag = self.tags.get(&tag_id)?;
                // Check if the word is a field name in the tag's record type.
                if let Ty::Record { fields, .. } = &tag.resolved_ty {
                    let field_key = Intern::<String>::from_ref(word);
                    if let Some((_, fty)) = fields.iter().find(|(n, _)| *n == field_key) {
                        // Use the original type annotation surface from the tag's
                        // record field definitions, preserved during stage_declare.
                        let field_surface = tag.record_field_types.get(&field_key);
                        let ty_str = field_surface.cloned().unwrap_or_else(|| {
                            // Fallback: format the inner type directly.
                            let tag_types = self.tag_types_by_name();
                            let display_ty: &Ty = match &**fty {
                                Ty::Ptr { inner } => inner.as_ref(),
                                other => other,
                            };
                            type_annotation_surface_for_hover(display_ty, &tag_types, None)
                        });
                        // No space between field name and surface when surface starts with `(`
                        // (e.g. `allocate(ref self, ...)` not `allocate (ref self, ...)`).
                        let shape = if ty_str.starts_with('(') {
                            format!("{}{}", word, ty_str)
                        } else {
                            format!("{} {}", word, ty_str)
                        };
                        // Include member doc comment if available
                        let member_doc = tag.record_field_docs.get(&field_key).map(|s| s.as_str());
                        let mut hover = ast::hover_format::HoverDoc::new().gin(shape);
                        if let Some(doc) = member_doc {
                            hover = hover.prose(doc.to_string());
                        }
                        let result = HoverResult::single(hover.render());
                        return Some(result.with_qualified_union(package, &tag_id.0));
                    }
                }
                let result = HoverResult::single(self.hover_for_tag(tag));
                Some(result.with_tag_module(package, &name))
            }
            HoverTarget::ModulePath(module) => {
                let doc = package
                    .and_then(|p| p.module_docs.get(&module))
                    .map(|d| ast::hover_format::HoverDoc::new().prose(d.clone()).render())
                    .unwrap_or_else(|| format!("`{module}`"));
                Some(HoverResult::single(doc))
            }
            HoverTarget::TypePattern(hover) => Some(hover),
            HoverTarget::Variant {
                union_name,
                discriminant,
                union_ty,
            } => {
                let resolved_ty = package
                    .and_then(|p| p.tag_types.get(&union_name))
                    .or_else(|| self.tag_types.get(&TagId(union_name)))
                    .unwrap_or(&union_ty);
                // For literal-constant unions (e.g. `Architecture is 'x86_64' or 'arm64'`),
                // show the parent tag declaration instead of just the literal label,
                // so the user sees the full union shape.
                if resolved_ty.union_literal_values().is_some() {
                    if let Some(tag) = self.tags.get(&TagId(union_name)) {
                        let result = HoverResult::single(self.hover_for_tag(tag));
                        return Some(result.with_tag_module(package, &union_name));
                    }
                    if let Some(pkg) = package
                        && let Some((_module, tag)) = pkg.tag_decl_for_word(&union_name)
                    {
                        let result = HoverResult::single(self.hover_for_tag(tag));
                        return Some(result.with_tag_module(package, &union_name));
                    }
                }
                let variant_label = const_union_variant_label(resolved_ty, discriminant)
                    .unwrap_or_else(|| {
                        // Format variant with its fields (e.g. `Some(x)` not just `Some`)
                        let variant_map =
                            package.map(|p| &p.variant_map).unwrap_or(&self.variant_map);
                        let key = Intern::<String>::from_ref(word);
                        if let Some(entries) = variant_map.get(&key)
                            && let Some((_, _, fields)) = entries.first()
                        {
                            let local_tag_types = self.tag_types_by_name();
                            let tag_types =
                                package.map(|p| &p.tag_types).unwrap_or(&local_tag_types);
                            return format_variant_pattern_label(word, fields, tag_types, None);
                        }
                        word.to_string()
                    });
                let result = HoverResult::single(self.hover_for_variant(
                    union_name.as_str(),
                    &variant_label,
                    resolved_ty,
                ));
                Some(result.with_qualified_union(package, &union_name))
            }
            HoverTarget::Expr(expr_id) => {
                if let Some(surface) = self
                    .param_surface_in_body(source, byte_offset, word)
                    .or_else(|| source_param_surface_before_byte(source, byte_offset, word))
                {
                    return Some(HoverResult::single(
                        ast::hover_format::HoverDoc::new()
                            .gin(format!("{word} {surface}"))
                            .render(),
                    ));
                }
                let expr_ref = self.expr(expr_id)?;
                if let TypedExprKind::FnCall { target, .. } = &expr_ref.kind
                    && word == target.0.as_str()
                    && let Some(bind) = self.defs.get(target)
                {
                    let result = HoverResult::single(self.hover_for_def(bind));
                    return Some(result.with_def_module(package, &target.0));
                }
                if matches!(expr_ref.kind, TypedExprKind::Lit(_))
                    && let Some(name) = self.parent_bind_name_for_body(expr_id)
                    && expr_ref.ty.union_literal_values().is_some()
                {
                    return Some(HoverResult::single(format!(
                        "{} union\n---\n\n",
                        name.as_str()
                    )));
                }
                let owned_tag_types;
                let owned_tag_params;
                let (tag_types, tag_params) = match package {
                    Some(p) => (&p.tag_types, Some(&p.tag_params)),
                    None => {
                        owned_tag_types = self.tag_types_by_name();
                        owned_tag_params = self.tag_params_by_name();
                        (&owned_tag_types, Some(&owned_tag_params))
                    }
                };
                let ty_str = type_annotation_surface_for_hover(expr_ref.ty, tag_types, tag_params);
                let is_copy = false;
                let summary = match &expr_ref.kind {
                    TypedExprKind::Bind {
                        name,
                        unassigned,
                        body,
                        ..
                    } => {
                        let no_explicit_type = !unassigned;
                        // Try bind's own const_value first; if None, try to find it
                        // from the body expression (local const binds store the literal
                        // value on the body expression, not on the bind itself).
                        let cv = expr_ref.const_value.as_ref().or_else(|| {
                            let body_idx = body.as_usize();
                            if body_idx < self.exprs.const_value.len()
                                && self.exprs.const_value[body_idx].is_some()
                            {
                                return self.exprs.const_value[body_idx].as_ref();
                            }
                            None
                        });
                        if no_explicit_type && let Some(cv) = cv {
                            // `:=` bind with const value, no explicit type: show `x 42`
                            format!("{} {}", name.as_str(), cv.to_hover_string())
                        } else {
                            // Explicit type annotation or no const: show `x Int`
                            format!("{} {}", name.as_str(), ty_str)
                        }
                    }
                    TypedExprKind::FnCall { target, .. } => {
                        if word == target.0.as_str() && !self.defs.contains_key(target) {
                            format!("{} {}", target.0.as_str(), ty_str)
                        } else {
                            format!("`{}`: `{ty_str}`", target.0.as_str())
                        }
                    }
                    TypedExprKind::TagCall { variant_id, .. } => {
                        if let Some(cv) = &expr_ref.const_value {
                            cv.to_hover_string()
                        } else {
                            format!(
                                "`{}` (variant of `{}`)",
                                variant_id.name.as_str(),
                                variant_id.union.0.as_str(),
                            )
                        }
                    }
                    TypedExprKind::Lit(_) => match &expr_ref.const_value {
                        Some(cv) => cv.to_hover_string(),
                        None => ty_str,
                    },
                    _ => ty_str,
                };
                Some(HoverResult::single(
                    ast::hover_format::HoverDoc::new()
                        .inline(summary)
                        .copy_is(is_copy)
                        .render(),
                ))
            }
        }
    }

    /// Resolve the type of a field access expression at a source position.
    /// For a position at `expr.field`, finds the expression before the dot,
    /// looks up its type, and if it's a Record, returns the field's type
    /// formatted via [`format_ty_for_hover`].
    pub fn dot_type(&self, source: &str, line: u32, character: u32) -> Option<String> {
        let byte_offset = source.position_to_byte_offset(line, character)?;

        // Check that there's a dot before the cursor position.
        let dot_pos = byte_offset.checked_sub(1)?;
        if source.as_bytes().get(dot_pos) != Some(&b'.') {
            return None;
        }

        // Extract the field name (the word at the cursor position).
        let field_name = source.word_at_byte_offset(byte_offset)?;

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
                        return Some((span.start(), span.end()));
                    }
                }
                None
            }
            TypedExprKind::FnCall { target, .. } => {
                if let Some(bind) = self.defs.get(target) {
                    let span = self.span_table.get(bind.name_span);
                    return Some((span.start(), span.end()));
                }
                None
            }
            TypedExprKind::TagCall { .. } => None,
            _ => None,
        }
    }

    /// Collect all type flaws from the expression arena and bind-level diagnostics.
    pub fn all_flaws(&self) -> Vec<(SpanId, &Diagnostic)> {
        let mut flaws = Vec::new();
        for i in 0..self.exprs.kind.len() {
            let span_id = self.exprs.span[i];
            for flaw in &self.exprs.flaws[i] {
                let flaw_span = if flaw.code.slug() == "type-unreachable-else-arm" {
                    if let TypedExprKind::When(when_expr) = &self.exprs.kind[i] {
                        when_expr
                            .arms
                            .iter()
                            .find_map(|arm| match arm {
                                TypedWhenArm::Else(_, sub) => Some(sub.into_inner()),
                                _ => None,
                            })
                            .unwrap_or(span_id)
                    } else {
                        span_id
                    }
                } else {
                    span_id
                };
                flaws.push((flaw_span, flaw));
            }
        }
        for bind in self.defs.values() {
            for flaw in &bind.flaws {
                flaws.push((bind.name_span, flaw));
            }
        }
        for (span_id, flaw) in &self.declaration_flaws {
            flaws.push((*span_id, flaw));
        }
        flaws
    }

    /// Collect declaration-level warnings.
    pub fn all_warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }
}

fn source_param_at_byte(source: &str, byte_offset: usize) -> Option<(String, String)> {
    let bytes = source.as_bytes();
    let mut start = byte_offset.min(bytes.len());
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = byte_offset.min(bytes.len());
    while end < bytes.len() && is_ident_byte(bytes[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    let name = source.get(start..end)?;
    let open = source.get(..start)?.rfind('(')?;
    let close = source.get(open..)?.find(')').map(|i| open + i)?;
    if end > close {
        return None;
    }
    let colon_eq = source.get(close..)?.find(":=").map(|i| close + i)?;
    let newline = source.get(close..colon_eq)?.find('\n');
    if newline.is_some() {
        return None;
    }
    let surface = extract_param_surface_after_name(source, end)?;
    Some((name.to_string(), surface))
}

fn source_param_surface_before_byte(
    source: &str,
    byte_offset: usize,
    name: &str,
) -> Option<String> {
    let prefix = source.get(..byte_offset)?;
    let needle = format!("{name} ");
    let mut search_end = prefix.len();
    while let Some(relative) = prefix.get(..search_end)?.rfind(&needle) {
        let open = prefix.get(..relative)?.rfind('(')?;
        let colon_eq = prefix.get(open..)?.find(":=").map(|i| open + i);
        if colon_eq.is_some_and(|idx| idx > relative) {
            return extract_param_surface_after_name(source, relative + name.len());
        }
        search_end = relative;
    }
    None
}

fn source_param_surface(
    source: &str,
    span_table: &SpanTable,
    bind: &ast::Bind,
    name: &str,
) -> Option<String> {
    let span = inferred_param_name_span_from(span_table, source, bind, name)?;
    extract_param_surface_after_name(source, span.end)
}

fn extract_param_surface_after_name(source: &str, name_end: usize) -> Option<String> {
    let bytes = source.as_bytes();
    let mut i = name_end;
    while bytes.get(i).is_some_and(|b| b.is_ascii_whitespace()) {
        i += 1;
    }
    let start = i;
    let mut depth = 0i32;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'(' => depth += 1,
            b')' if depth > 0 => depth -= 1,
            b',' | b')' if depth == 0 => break,
            _ => {}
        }
        i += 1;
    }
    let surface = source.get(start..i)?.trim();
    (!surface.is_empty()).then(|| surface.to_string())
}

fn signature_param_surface(signature: &str, name: &str) -> Option<String> {
    let open = signature.find('(')?;
    let close = signature.rfind(')')?;
    let params = signature.get(open + 1..close)?;
    for part in params.split(',') {
        let part = part.trim();
        let rest = part.strip_prefix(name)?.trim_start();
        if rest.is_empty()
            || rest.starts_with(':')
            || rest.starts_with(|c: char| c.is_ascii_uppercase())
        {
            return Some(rest.to_string());
        }
    }
    None
}

fn param_kind_surface_for_hover(kind: &ParameterKind) -> String {
    match kind {
        ParameterKind::Tagged(sp) => type_expr_surface_for_hover(&sp.value),
        ParameterKind::Generic => String::new(),
        ParameterKind::Default(expr) => format!(": {:?}", expr.value),
    }
}

fn type_expr_surface_for_hover(expr: &TypeExpr) -> String {
    match expr {
        TypeExpr::Nominal(name, _) => name.as_str().to_string(),
        TypeExpr::Generic { name, params, .. } => {
            let inner = params
                .iter()
                .map(|(key, kind)| match kind {
                    ParameterKind::Tagged(sp) => type_expr_surface_for_hover(&sp.value),
                    ParameterKind::Generic => key.as_str().to_string(),
                    ParameterKind::Default(expr) => format!("{}: {:?}", key.as_str(), expr.value),
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({})", name.as_str(), inner)
        }
        TypeExpr::Qualified(path) => {
            let mut out = path.root.as_str().to_string();
            for segment in &path.segments {
                out.push('.');
                out.push_str(segment.as_str());
            }
            out
        }
        other => format!("{other:?}"),
    }
}

/// IDE hover payload; [`module_prefix`] holds a qualified type path when known.
fn inferred_param_name_span_from(
    span_table: &SpanTable,
    source: &str,
    bind: &ast::Bind,
    param_name: &str,
) -> Option<std::ops::Range<usize>> {
    let start = span_table.get(bind.name_span).end();
    let end = match &bind.value {
        ast::BindValue::Expr(expr) => span_table.get(expr.span_id).start(),
        ast::BindValue::Body { exprs, ret } => exprs
            .first()
            .map(|expr| span_table.get(expr.span_id).start())
            .unwrap_or_else(|| span_table.get(ret.span_id).start()),
        ast::BindValue::Extern | ast::BindValue::Unassigned => start,
    };
    let offset = find_param_name_between(source, start, end, param_name)?;
    Some(offset..offset + param_name.len())
}

fn find_param_name_between(source: &str, start: usize, end: usize, name: &str) -> Option<usize> {
    let haystack = source.get(start..end)?;
    let mut search_from = 0;
    while let Some(relative) = haystack.get(search_from..)?.find(name) {
        let offset = start + search_from + relative;
        let before = source.as_bytes().get(offset.wrapping_sub(1)).copied();
        let after = source.as_bytes().get(offset + name.len()).copied();
        let ident_before = before.is_some_and(is_ident_byte);
        let ident_after = after.is_some_and(is_ident_byte);
        if !ident_before && !ident_after {
            return Some(offset);
        }
        search_from += relative + name.len();
    }
    None
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverResult {
    pub markdown: String,
    /// Qualified path to the base type (e.g. `core.primitive.Bool` for variant `False`).
    pub module_prefix: Option<String>,
}

impl HoverResult {
    pub fn single(markdown: String) -> Self {
        Self {
            markdown,
            module_prefix: None,
        }
    }

    /// Prefix hover with `{module}` when the tag's declaring module is known.
    pub fn with_tag_module(
        mut self,
        package: Option<&PackageSemanticIndex>,
        tag_name: &Intern<String>,
    ) -> Self {
        if self.module_prefix.is_none()
            && let Some(pkg) = package
            && let Some(module) = pkg.tag_module.get(tag_name)
        {
            self.module_prefix = Some(module.clone());
        }
        self
    }

    /// Prefix hover with `{module}.{def}` when the definition's declaring module is known.
    pub fn with_def_module(
        mut self,
        package: Option<&PackageSemanticIndex>,
        def_name: &Intern<String>,
    ) -> Self {
        if self.module_prefix.is_none()
            && let Some(pkg) = package
            && let Some(path) = pkg.qualified_def_path(def_name)
        {
            self.module_prefix = Some(path);
        }
        self
    }

    /// Prefix hover with `{module}.{union}` when the union's declaring module is known.
    pub fn with_qualified_union(
        mut self,
        package: Option<&PackageSemanticIndex>,
        union_name: &Intern<String>,
    ) -> Self {
        if self.module_prefix.is_none()
            && let Some(pkg) = package
            && let Some(path) = pkg.qualified_union_path(union_name)
        {
            self.module_prefix = Some(path);
        }
        self
    }
}

/// Cross-file tag and variant index for package-scoped IDE hover.
#[derive(Clone, PartialEq)]
pub struct PackageSemanticIndex {
    pub tag_types: HashMap<Intern<String>, Ty>,
    pub tag_params: HashMap<Intern<String>, Parameters>,
    pub variant_map: VariantMap,
    /// Declaring module per tag name (`Bool` → `core.primitive`).
    /// Qualified path to the base type (e.g. `core.primitive.Bool` for variant `False`).
    pub tag_module: HashMap<Intern<String>, String>,
    /// Tag declarations from across the package for cross-file name hovers.
    pub tag_decls: HashMap<Intern<String>, TypedTag>,
    /// Declaring module per definition name (`Default` → `core.default`).
    pub def_module: HashMap<Intern<String>, String>,
    /// Merged module-level doc comments (`--|`), keyed by qualified module path.
    /// Populated by [`stage_build_index`] — concatenates docs from all files in
    /// each module directory in alphabetical filename order.
    pub module_docs: HashMap<String, String>,
}

impl PackageSemanticIndex {
    pub fn tag_decl_for_word(&self, name: &Intern<String>) -> Option<(&String, &TypedTag)> {
        self.tag_module.get(name).zip(self.tag_decls.get(name))
    }
}

impl PackageSemanticIndex {
    pub fn from_typed_asts(asts: &[&TypedFileAst]) -> Self {
        let mut tag_types: HashMap<Intern<String>, Ty> = HashMap::new();
        let mut tag_params: HashMap<Intern<String>, Parameters> = HashMap::new();

        for ast in asts {
            for (tag_id, ty) in &ast.tag_types {
                tag_types.insert(tag_id.0, ty.clone());
            }
            for (tag_id, tag) in &ast.tags {
                if let Some(params) = &tag.params {
                    tag_params.insert(tag_id.0, params.clone());
                }
            }
        }

        let variant_map = collect_package_variant_map(asts);

        Self {
            tag_types,
            tag_params,
            variant_map,
            tag_module: HashMap::new(),
            tag_decls: HashMap::new(),
            def_module: HashMap::new(),
            module_docs: HashMap::new(),
        }
    }

    pub fn module_for_variant_word(&self, variant: &Intern<String>) -> Option<&String> {
        let (union, _, _) = self.variant_map.get(variant)?.first()?;
        self.tag_module.get(union)
    }

    /// `core.primitive.Bool` for union `Bool` declared in module `core.primitive`.
    pub fn qualified_union_path(&self, union: &Intern<String>) -> Option<String> {
        let module = self.tag_module.get(union)?;
        Some(format!("{module}.{}", union.as_str()))
    }

    /// `core.default` for definition `Default` declared in module `core.default`.
    pub fn qualified_def_path(&self, def: &Intern<String>) -> Option<String> {
        self.def_module.get(def).cloned()
    }
}

fn const_union_variant_label(ty: &Ty, discriminant: usize) -> Option<String> {
    let values = ty.union_literal_values()?;
    let cv = values.get(discriminant)?;
    match cv {
        ast::ConstValue::String(s) => Some(format!("'{s}'")),
        _ => Some(cv.to_hover_string()),
    }
}

fn format_variant_pattern_label(
    name: &str,
    fields: &[(Intern<String>, Ty)],
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> String {
    if fields.is_empty() {
        return name.to_string();
    }
    let parts: Vec<String> = fields
        .iter()
        .map(|(n, t)| {
            let type_str = variant_pattern_field_type_surface(t, tag_types, tag_params);
            if type_str == n.as_str() {
                // Opaque generic matching the field name — show just the name.
                n.as_str().to_string()
            } else {
                format!("{} {}", n.as_str(), type_str)
            }
        })
        .collect();
    format!("{}({})", name, parts.join(", "))
}

fn variant_pattern_field_type_surface(
    ty: &Ty,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> String {
    if let Ty::Record { name, .. } = ty
        && name.as_str() != "List"
    {
        return format_ty_for_hover(ty).replace("Union(Type)", "Union(union)");
    }
    type_annotation_surface_for_hover(ty, tag_types, tag_params)
}

/// Type name as written in a `name Type` declare (e.g. `List(NamedTy)` not `pointer: …`).
pub(crate) fn type_annotation_surface_for_hover(
    ty: &Ty,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> String {
    match ty {
        Ty::Union { name, .. } => {
            if name.as_str() == "union"
                && let Some((tag_name, _)) = tag_types.iter().find(|(_, tag_ty)| *tag_ty == ty)
            {
                return tag_name.as_str().to_string();
            }
            name.as_str().to_string()
        }
        Ty::Opaque(name) => name.as_str().to_string(),
        Ty::Record { name, fields } => {
            if let Some(surface) = generic_record_surface(name, fields, tag_types, tag_params) {
                return surface;
            }
            if tag_types.contains_key(name) && !tag_has_generic_params(name, tag_params) {
                return name.as_str().to_string();
            }
            format_ty_for_hover(ty)
        }
        other => {
            // For unbounded concrete ints matching a tag declaration, show the source-level
            // type name. Bounded `in lo...hi` types should keep their range surface; otherwise
            // unrelated aliases like `Byte`/`TinyInt` can win HashMap iteration order and obscure
            // reflective payload slots such as `Primitive(width BigInt, ...)`.
            if matches!(
                other,
                Ty::Int {
                    min: None,
                    max: None,
                    ..
                }
            ) && let Some((name, _)) = tag_types.iter().find(|(_, t)| {
                matches!(
                    (other, t),
                    (Ty::Int { .. }, Ty::Int { .. }) | (Ty::Int { .. }, Ty::Union { .. })
                )
            }) {
                return name.as_str().to_string();
            }
            format_ty_for_hover(other)
        }
    }
}

fn pattern_binding_type_surface_for_hover(
    ty: &Ty,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> String {
    type_annotation_surface_for_hover(ty, tag_types, tag_params)
}

fn tag_has_generic_params(
    name: &Intern<String>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> bool {
    tag_params
        .and_then(|tp| tp.get(name))
        .is_some_and(|params| {
            params
                .iter()
                .any(|(_, k)| matches!(k, ParameterKind::Generic))
        })
}

fn generic_record_surface(
    name: &Intern<String>,
    fields: &[(Intern<String>, Box<Ty>)],
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> Option<String> {
    if !tag_has_generic_params(name, tag_params) {
        return None;
    }
    match name.as_str() {
        "List" => {
            let elem = list_element_ty_from_record_fields(fields)?;
            Some(format!(
                "List({})",
                type_annotation_surface_for_hover(&elem, tag_types, tag_params)
            ))
        }
        _ => None,
    }
}

fn is_capitalized_type_name(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

fn list_element_ty_from_record_fields(fields: &[(Intern<String>, Box<Ty>)]) -> Option<Ty> {
    let (_, pointer) = fields.iter().find(|(n, _)| n.as_str() == "pointer")?;
    match pointer.as_ref() {
        Ty::Ptr { inner } => Some(inner.as_ref().clone()),
        Ty::Opaque(name) if is_capitalized_type_name(name.as_str()) => {
            Some(pointer.as_ref().clone())
        }
        Ty::Record { name, .. }
            if is_capitalized_type_name(name.as_str()) && name.as_str() != "List" =>
        {
            Some(pointer.as_ref().clone())
        }
        Ty::Record {
            name,
            fields: pfields,
        } if name.as_str() == "Pointer" => pfields.iter().find_map(|(n, t)| {
            if n.as_str() == "addr" {
                return None;
            }
            match t.as_ref() {
                Ty::Opaque(inner) if is_capitalized_type_name(inner.as_str()) => {
                    Some(t.as_ref().clone())
                }
                Ty::Record { name: inner, .. } if is_capitalized_type_name(inner.as_str()) => {
                    Some(t.as_ref().clone())
                }
                _ => None,
            }
        }),
        _ => None,
    }
}

/// Format a `Ty` for hover display.
pub fn format_ty_for_hover(ty: &Ty) -> String {
    match ty {
        Ty::Int {
            width,
            signed,
            value,
            min,
            max,
        } => {
            if let (Some(lo), Some(hi)) = (min, max) {
                if let Some(v) = value {
                    format!("in {lo}...{hi} (= {v})")
                } else {
                    format!("in {lo}...{hi}")
                }
            } else {
                let prefix = if *signed { "i" } else { "u" };
                if let Some(v) = value {
                    format!("{}{} = {}", prefix, width, v)
                } else {
                    format!("{}{}", prefix, width)
                }
            }
        }
        Ty::Float { value } => {
            if let Some(HashFloat(v)) = value {
                format!("f64 = {}", v)
            } else {
                "f64".to_string()
            }
        }
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
        Ty::Literal(cv) => match cv {
            ast::ConstValue::String(s) => format!("'{s}'"),
            other => other.to_hover_string(),
        },
    }
}
