use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::ControlFlow;

use ast::{BinderId, ConstValue, GroupPath, NormalExpr};

use ast::HashFloat;
use ast::parameter::{ParamConvention, ParameterKind, Parameters};
use ast::prelude::*;
use ast::source::SourceExt;
use ast::span::{SpanId, SpanTable, SubSpan};
use ast::ty::ParamKind;
use ast::ty::PredicateExpr;
use ast_format::type_expr::ExprFormatExt;
use derive_more::From;
use diagnostic::Diagnostic;
use internment::Intern;

use crate::solver::{ConstraintEnv, Predicate, ProveResult};
use crate::ty::{Ty, TyArg};

mod semantic_index;
use semantic_index::*;
pub use semantic_index::{HoverResult, PackageSemanticIndex, format_ty_for_hover};
mod places;
pub use places::{
    PlaceId, PlaceProjection, PlaceVersionComponent, PlaceVersionId, PlaceVersionOrigin,
    TypedPlace, TypedPlaceVersion,
};
mod evidence;
pub use evidence::{
    AlternativeEvidence, EvidenceProposition, MathematicalComparison, ProjectedResultEvidence,
    ResultEvidence,
};
mod ownership;
pub use ownership::{
    AppliedEffectTarget, EffectTarget, GroupId, Overlap, ReferenceTargetGroup, ReferenceTargetSet,
    TargetGroupApplication, TargetIndex,
};
mod typed_expr;
pub use typed_expr::*;
pub use typed_expr::{
    typed_walk_expr_children as walk_expr_children,
    typed_walk_expr_children_of as walk_expr_children_of,
    typed_walk_expr_preorder as walk_expr_preorder,
};

/// Opaque file identifier assigned during compilation coordination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
pub struct FileId(pub u32);

/// A definition (bind) identifier — the fully-qualified name.
/// Interned string, e.g. Intern("main") or Intern("Range.new").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
pub struct DefId(pub Intern<String>);

/// A tag (type) identifier — the interned tag name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
pub struct TagId(pub Intern<String>);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VariantId {
    pub union: TagId,
    pub name: Intern<String>,
}

/// Index into the expression arena (soa_derive TypedExprVec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
pub struct ExprId(pub u32);

impl ExprId {
    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Availability {
    Unknown,
    CompileTime,
    Runtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AvailabilityRequirement {
    Any,
    CompileTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallCapability {
    StagePolymorphic,
    RuntimeOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefinitionStage {
    CompileTimeOnly,
    StagePolymorphic,
    RuntimeOnly,
}

pub use crate::intrinsic::IntrinsicOp;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GroupProjection {
    Field { name: Intern<String>, index: usize },
    Items,
    Pointee,
    Variant { name: Intern<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedGroup {
    pub path: Option<GroupPath>,
    pub projections: Option<Vec<GroupProjection>>,
    pub referent_type: Ty,
}

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

#[derive(Debug, Clone, PartialEq)]
pub struct TypedTag {
    pub name_span: SpanId,
    /// Full `Tag is …` / `Tag has …` site (for hover on the RHS, e.g. `in 0 to 255`).
    pub span: SpanId,
    /// The resolved type of this tag (e.g., `Ty::Union`, `Ty::Record`, `Ty::ConstUnion`, etc.).
    pub resolved_ty: Ty,
    /// Tag attributes (e.g., `#[test]`).
    pub attributes: DeclareAttributes,
    pub doc_comment: Option<DocComment>,
    /// Tag parameters (type variables, defaults), if any.
    pub params: Option<Parameters>,
    /// For record types (`has` bodies), field name → formatted type annotation surface
    /// (e.g. `"pointer"` → `"@x"`). Populated during stage_declare.
    pub record_field_types: HashMap<Intern<String>, String>,
    /// For interface (`has`) bodies, method name → doc comment, if any.
    /// Only entries for members that have a doc are present.
    /// Populated during stage_declare alongside `record_field_types`.
    pub record_field_docs: HashMap<Intern<String>, String>,
    /// For record fields, field name → refinement predicate (e.g. `and < n`).
    /// Only entries for fields that have a refinement are present.
    pub record_field_refinements: HashMap<Intern<String>, PredicateExpr>,
    /// Formatted declaration text (e.g. "Bool is True or False"), for use in hover.
    pub declaration_text: String,
    /// Trait implementations provided via `and has TraitName(field: expr, ...)` clauses.
    /// Explicit (concrete) field definitions always win over these provided ones.
    pub provided_traits: Vec<ProvidedTrait>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FunctionEffects {
    pub reads: HashSet<EffectTarget>,
    pub writes: HashSet<EffectTarget>,
    pub invalidates: HashSet<EffectTarget>,
    pub consumes: HashSet<EffectTarget>,
    pub unknown_memory: bool,
    pub assembly_reads: Vec<(String, String)>,
    pub assembly_writes: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedCallableSignature {
    pub dependent_binder: BinderId,
    pub return_type: Ty,
    pub results: Vec<CallableResult>,
    pub params: Vec<(Intern<String>, Ty)>,
    pub param_kinds: Vec<ParamKind>,
    pub param_conventions: Vec<ParamConvention>,
    pub param_groups: Vec<Option<GroupId>>,
    pub param_refinements: Vec<Option<PredicateExpr>>,
    pub groups: Vec<TypedGroup>,
    /// Ownership target for each result, in result order.
    pub result_groups: Vec<Option<GroupId>>,
    pub effects: FunctionEffects,
    pub param_requirements: Vec<AvailabilityRequirement>,
    pub call_capability: CallCapability,
    pub definition_stage: DefinitionStage,
    pub infix_symbol: Option<Intern<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallableResult {
    pub name: Option<Intern<String>>,
    pub ty: Ty,
}

impl TypedCallableSignature {
    pub fn group_is_mutated(&self, group: GroupId) -> bool {
        self.param_groups
            .iter()
            .zip(&self.param_conventions)
            .any(|(param_group, convention)| {
                *param_group == Some(group) && *convention == ParamConvention::Mutate
            })
    }

    pub fn allows_parent_child_overlap(&self, left: GroupId, right: GroupId) -> bool {
        let Some(left_path) = self
            .groups
            .get(left.0 as usize)
            .and_then(|group| group.path.as_ref())
        else {
            return false;
        };
        let Some(right_path) = self
            .groups
            .get(right.0 as usize)
            .and_then(|group| group.path.as_ref())
        else {
            return false;
        };

        (left_path.is_ancestor_of(right_path) && !self.group_is_mutated(left))
            || (right_path.is_ancestor_of(left_path) && !self.group_is_mutated(right))
    }
}

impl From<&TypedBind> for TypedCallableSignature {
    fn from(bind: &TypedBind) -> Self {
        Self {
            dependent_binder: bind.dependent_binder,
            return_type: bind.return_type.clone(),
            results: callable_results(bind),
            params: bind.params.clone(),
            param_kinds: bind.param_kinds.clone(),
            param_conventions: bind.param_conventions.clone(),
            param_groups: bind.param_groups.clone(),
            param_refinements: bind.param_refinements.clone(),
            groups: bind.groups.clone(),
            result_groups: bind.result_groups.clone(),
            effects: bind.effects.clone(),
            param_requirements: bind.param_requirements.clone(),
            call_capability: bind.call_capability,
            definition_stage: bind.definition_stage,
            infix_symbol: bind.infix_symbol,
        }
    }
}

fn callable_results(bind: &TypedBind) -> Vec<CallableResult> {
    if let Ty::ResultRow(elements) = &bind.return_type {
        return elements
            .iter()
            .map(|element| CallableResult {
                name: Some(element.name),
                ty: (*element.ty).clone(),
            })
            .collect();
    }
    let tys = match &bind.return_type {
        Ty::Unit => Vec::new(),
        ty => vec![ty.clone()],
    };
    tys.into_iter()
        .enumerate()
        .map(|(index, ty)| CallableResult {
            name: bind.named_results.get(index).map(|result| result.name),
            ty,
        })
        .collect()
}

/// A fully-resolved bind (function or value definition).
#[derive(Debug, Clone, PartialEq)]
pub struct TypedBind {
    pub name: Intern<String>,
    pub dependent_binder: BinderId,
    /// Span of the name in source.
    pub name_span: SpanId,
    pub body: BindBody,
    /// The resolved return type.
    pub return_type: Ty,
    /// Named result declarations preserved for projection and tooling.
    pub named_results: Vec<ast::NamedResult>,
    /// User-written return type before later lowering.
    pub declared_return_type: Ty,
    pub return_evidence: Option<ResultEvidence>,
    /// Resolved parameter types (name, resolved Ty).
    pub params: Vec<(Intern<String>, Ty)>,
    /// Whether each parameter is a type or value parameter.
    pub param_kinds: Vec<ParamKind>,
    /// Ownership contract for each parameter, in declaration order.
    pub param_conventions: Vec<ParamConvention>,
    pub param_groups: Vec<Option<GroupId>>,
    pub param_refinements: Vec<Option<PredicateExpr>>,
    pub groups: Vec<TypedGroup>,
    /// Ownership target for each result, in result order.
    pub result_groups: Vec<Option<GroupId>>,
    pub effects: FunctionEffects,
    pub param_requirements: Vec<AvailabilityRequirement>,
    pub call_capability: CallCapability,
    pub definition_stage: DefinitionStage,
    /// Receiver type for methods, if any.
    pub receiver_type: Option<Ty>,
    /// Bind attributes (e.g., `#[inline]`, visibility).
    pub attributes: BindAttributes,
    pub infix_symbol: Option<Intern<String>>,
    pub doc_comment: Option<DocComment>,
    /// Bind-level diagnostics (e.g., redundant self-param type).
    pub flaws: Vec<Diagnostic>,
    /// `name Type` at module or function scope with no `:` value yet.
    pub unassigned_decl: bool,
    /// Bound with `:=` (immutable).
    pub is_constant: bool,
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
    Unlowered,
}

/// The typed AST for one `.gin` file — all types resolved, all flaws attached.
///
/// This is the source of truth for LSP queries and further analysis.
#[derive(Clone)]
pub struct TypedFileAst {
    /// Span table mapping SpanId → byte ranges (cloned from FileAst).
    pub span_table: SpanTable,
    /// The file identifier assigned during compilation coordination.
    pub file_id: FileId,
    pub type_registry: crate::TypeRegistry,
    pub target_layout: Option<crate::layout::TargetLayout>,
    pub constant_objects: crate::ConstObjectStore,

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
    pub places: Vec<TypedPlace>,
    pub place_versions: Vec<TypedPlaceVersion>,
    pub place_version_components: Vec<PlaceVersionComponent>,

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
    pub imported_trait_names: HashSet<Intern<String>>,
    pub(crate) self_contexts: Vec<(SpanId, Intern<String>, Intern<String>)>,
    pub eval_ast: std::sync::Arc<ast::FileAst>,
    pub semantic_origin: Option<FileSemanticOrigin>,
    pub module_doc: Option<ast::DocComment>,
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
            .field("type_registry", &self.type_registry)
            .field("tags", &self.tags)
            .field("defs", &self.defs)
            .field("private_tags", &self.private_tags)
            .field("private_defs", &self.private_defs)
            .field("exprs", &self.exprs)
            .field("root_exprs", &self.root_exprs)
            .field("span_to_expr", &self.span_to_expr)
            .field("tag_types", &self.tag_types)
            .field("fn_return_types", &self.fn_return_types)
            .field("variant_map", &self.variant_map)
            .field("semantic_origin", &self.semantic_origin)
            .finish()
    }
}

impl PartialEq for TypedFileAst {
    fn eq(&self, other: &Self) -> bool {
        self.file_id == other.file_id
            && self.target_layout == other.target_layout
            && self.tags == other.tags
            && self.defs == other.defs
            && self.private_tags == other.private_tags
            && self.private_defs == other.private_defs
            && self.exprs == other.exprs
            && self.root_exprs == other.root_exprs
            && self.span_to_expr == other.span_to_expr
            && self.tag_types == other.tag_types
            && self.fn_return_types == other.fn_return_types
            && self.variant_map == other.variant_map
            && self.variant_annotations == other.variant_annotations
            && self.semantic_origin == other.semantic_origin
            && self.module_doc == other.module_doc
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
    SelfRef {
        /// e.g. "self ref", "self mut", "self", "self own"
        modifier: Intern<String>,
        tag_name: Intern<String>,
    },
    TagDecl(Intern<String>),
    TagAtByte(Intern<String>),
    RecordField {
        name: Intern<String>,
        surface: String,
    },
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
    pub(crate) fn add_place_version(
        &mut self,
        place: PlaceId,
        origin: PlaceVersionOrigin,
        ty: Ty,
        integer_knowledge: Option<ast::integer::IntegerKnowledge>,
    ) -> PlaceVersionId {
        let id = PlaceVersionId(self.place_versions.len() as u32);
        self.place_versions.push(TypedPlaceVersion {
            place,
            origin,
            ty,
            integer_knowledge,
        });
        id
    }

    pub fn apply_target_groups(
        &self,
        args: &[ExprId],
        signature: &TypedCallableSignature,
    ) -> TargetGroupApplication {
        let mut application = TargetGroupApplication::default();
        for (index, group) in signature.param_groups.iter().enumerate() {
            let Some(group) = group else {
                continue;
            };
            let Some(actual) = args
                .get(index)
                .and_then(|arg| self.exprs.target_group_of(*arg))
                .and_then(Option::as_ref)
            else {
                continue;
            };
            let convention = signature
                .param_conventions
                .get(index)
                .copied()
                .unwrap_or(ParamConvention::Terminal);
            application
                .actuals
                .entry(*group)
                .or_default()
                .extend(actual);
            *application
                .grouped_argument_counts
                .entry(*group)
                .or_default() += 1;
            match convention {
                ParamConvention::Mutate => {
                    application.grouped_argument_mutate.insert(*group, true);
                }
                _ => {
                    application.grouped_argument_observe.insert(*group, true);
                }
            }
        }
        application
    }

    fn has_self_context_at_byte(
        &self,
        byte_offset: usize,
    ) -> Option<(Intern<String>, Intern<String>)> {
        if let Some((_, tag_name, modifier)) = self
            .self_contexts
            .iter()
            .find(|(span, _, _)| self.span_table.get(*span).contains(byte_offset))
        {
            return Some((*tag_name, *modifier));
        }

        let self_name = Intern::<String>::from_ref("self");
        for declare in self.eval_ast.tags.values() {
            let ast::DeclareValue::Has(members) = &declare.value else {
                continue;
            };
            for member in members {
                let ast::HasMember::Function(function) = member else {
                    continue;
                };
                if !function.params.contains_key(&self_name) {
                    continue;
                }
                let Some(body) = function.body.as_ref() else {
                    continue;
                };
                let value = match body {
                    ast::HasMemberBody::Overrideable(value) | ast::HasMemberBody::Final(value) => {
                        value
                    }
                };
                let mut self_spans = Vec::new();
                crate::transform::collect_bind_value_self_ref_spans(value, &mut self_spans);
                let ast_span_matches = self_spans
                    .iter()
                    .any(|span| self.span_table.get(*span).contains(byte_offset));
                if !ast_span_matches {
                    continue;
                }
                let modifier = match function.conventions.get(&self_name) {
                    Some(ast::ParamConvention::Observe) => "self ref",
                    Some(ast::ParamConvention::Mutate) => "self mut",
                    Some(ast::ParamConvention::Own) => "self own",
                    Some(ast::ParamConvention::Terminal) | None => "self",
                };
                return Some((declare.name, Intern::<String>::from_ref(modifier)));
            }
        }
        None
    }

    fn param_at_byte(&self, byte_offset: usize, word: &str) -> Option<(Intern<String>, String)> {
        let word = Intern::<String>::from_ref(word);
        let mut def_starts: Vec<usize> = self
            .defs
            .values()
            .map(|bind| self.span_table.get(bind.name_span).start())
            .collect();
        def_starts.sort_unstable();
        def_starts.dedup();
        for ast_bind in self.eval_ast.defs.values() {
            let Some(parameter) = ast_bind
                .params
                .as_ref()
                .and_then(|params| params.get(&word))
            else {
                continue;
            };
            let surface = self
                .defs
                .get(&DefId(ast_bind.name))
                .or_else(|| {
                    self.defs.values().find(|bind| {
                        bind.name.as_str() == ast_bind.name.as_str()
                            || bind.name.as_str().split('.').next_back()
                                == Some(ast_bind.name.as_str())
                    })
                })
                .and_then(|typed_bind| self.typed_param_surface(typed_bind, &word))
                .unwrap_or_else(|| param_kind_surface_for_hover(&parameter.kind));

            if self
                .span_table
                .get(parameter.name_span)
                .contains(byte_offset)
            {
                return Some((word, surface));
            }

            // Keep declaration-level parameter hovers in signatures; typed
            // body-scope hovers are handled by typed-bind fallback for the
            // current declaration set.
            continue;
        }
        for typed_bind in self.defs.values() {
            let has_local_def = self.eval_ast.defs.values().any(|ast_bind| {
                ast_bind.name == typed_bind.name
                    || ast_bind.name.split('.').next_back() == Some(typed_bind.name.as_str())
            });
            if has_local_def {
                continue;
            }
            let Some((index, parameter_name)) = typed_bind
                .params
                .iter()
                .enumerate()
                .find_map(|(index, (name, _))| (name == &word).then_some((index, name)))
            else {
                continue;
            };
            let surface = self
                .typed_param_surface_at(typed_bind, index)
                .unwrap_or_else(|| parameter_name.as_str().to_string());
            let body_contains = match &typed_bind.body {
                BindBody::Expr(expr) => self
                    .span_table
                    .get(self.exprs.span[expr.as_usize()])
                    .contains(byte_offset),
                BindBody::Body { exprs, ret } => {
                    exprs.iter().any(|expr| {
                        self.span_table
                            .get(self.exprs.span[expr.as_usize()])
                            .contains(byte_offset)
                    }) || ret.as_ref().is_some_and(|ret| {
                        self.span_table
                            .get(self.exprs.span[ret.as_usize()])
                            .contains(byte_offset)
                    })
                }
                BindBody::Unlowered => false,
            };
            let signature_contains = match &typed_bind.body {
                BindBody::Expr(expr) => {
                    byte_offset
                        < self
                            .span_table
                            .get(self.exprs.span[expr.as_usize()])
                            .start()
                }
                BindBody::Body { exprs, .. } => exprs.first().is_none_or(|expr| {
                    byte_offset
                        < self
                            .span_table
                            .get(self.exprs.span[expr.as_usize()])
                            .start()
                }),
                BindBody::Unlowered => false,
            };
            let this_def_start = self.span_table.get(typed_bind.name_span).start();
            if byte_offset < this_def_start {
                continue;
            }
            let next_def_start = def_starts
                .iter()
                .find(|start| **start > this_def_start)
                .copied()
                .unwrap_or(usize::MAX);
            if byte_offset >= next_def_start {
                continue;
            }
            if signature_contains || (body_contains && byte_offset < next_def_start) {
                return Some((word, surface));
            }
        }
        None
    }

    pub fn new(file_id: FileId, span_table: SpanTable) -> Self {
        Self {
            span_table,
            file_id,
            type_registry: crate::TypeRegistry::default(),
            target_layout: None,
            constant_objects: crate::ConstObjectStore::default(),
            tags: HashMap::new(),
            defs: HashMap::new(),
            private_tags: HashSet::new(),
            private_defs: HashSet::new(),
            exprs: TypedExprVec::new(),
            root_exprs: Vec::new(),
            places: Vec::new(),
            place_versions: Vec::new(),
            place_version_components: Vec::new(),

            span_to_expr: BTreeMap::new(),
            tag_types: HashMap::new(),
            fn_return_types: HashMap::new(),
            variant_map: HashMap::new(),
            variant_annotations: HashMap::new(),
            imported_trait_names: HashSet::new(),
            self_contexts: Vec::new(),
            eval_ast: std::sync::Arc::new(ast::FileAst::empty_for_tests()),
            semantic_origin: None,
            module_doc: None,
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
    /// Used for hovers on declare RHS (`in 0 to 255`, union variants, etc.).
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
    /// has `trait_name` and `fields`. Use `tag.declaration_text` for the type
    /// signature block, then insert a `HoverSection::Prose` (or a new section variant)
    /// that lists trait names and links to their declarations. Also consider:
    ///
    /// - Auto-synthesized traits (e.g. `Reflectable` via `synthesize_reflectable_trait`)
    ///   — include them but perhaps distinguish from user-written `and has` clauses.
    /// - Auto trait defaults that apply to this type.
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

    fn variant_pattern_label_local(
        &self,
        variant_map: &VariantMap,
        union_name: &str,
        variant_name: &str,
    ) -> String {
        let key = Intern::<String>::from_ref(variant_name);
        if let Some(entries) = variant_map.get(&key) {
            let union_key = Intern::<String>::from_ref(union_name);
            if let Some((_, _, fields)) = entries.iter().find(|(u, _, _)| *u == union_key) {
                return format_variant_pattern_label_local(
                    variant_name,
                    fields,
                    &self.tag_types,
                    &self.tags,
                );
            }
            if let Some((_, _, fields)) = entries.first() {
                return format_variant_pattern_label_local(
                    variant_name,
                    fields,
                    &self.tag_types,
                    &self.tags,
                );
            }
        }
        variant_name.to_string()
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
        let label = if let Some(p) = package {
            self.variant_pattern_label(
                variant_map,
                union_name.as_str(),
                word,
                &p.tag_types,
                Some(&p.tag_params),
            )
        } else {
            self.variant_pattern_label_local(variant_map, union_name.as_str(), word)
        };
        Some(
            HoverResult::single(self.hover_for_variant(union_name.as_str(), &label, &union_ty))
                .with_qualified_union(package, union_name),
        )
    }

    fn hover_pattern(&self, pattern: &Pattern, ctx: HoverPatternCtx<'_>) -> Option<HoverResult> {
        self.hover_pattern_node(pattern, ctx)
    }

    fn hover_pattern_node(
        &self,
        pattern: &Pattern,
        ctx: HoverPatternCtx<'_>,
    ) -> Option<HoverResult> {
        match pattern {
            Pattern::Generic {
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
                                type_annotation_surface_for_hover(&t, ctx.tag_types, ctx.tag_params)
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
            Pattern::ListCons { head, tail } => {
                let elem_ty = ctx.subject_ty.and_then(|ty| ty.list_elem_ty(ctx.tag_types));
                let head_ctx = HoverPatternCtx {
                    subject_ty: elem_ty.as_ref(),
                    ..ctx
                };
                if let Some(h) = self.hover_pattern_node(&head.value, head_ctx) {
                    return Some(h);
                }
                return self.hover_pattern_node(&tail.value, ctx);
            }
            Pattern::Nominal(name, span)
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
                    let list_type_param = ctx.subject_ty.and_then(|subject_ty| match subject_ty {
                        Ty::Record { name, .. } if name.as_str() == "List" => {
                            ctx.tag_params.and_then(|params| {
                                params
                                    .get(&Intern::from_ref("List"))
                                    .and_then(|params| params.first())
                                    .map(|(name, _)| name.as_str().to_string())
                            })
                        }
                        _ => None,
                    });

                    let ty_str = ctx
                        .subject_ty
                        .map(|t| {
                            type_annotation_surface_for_hover(t, ctx.tag_types, ctx.tag_params)
                        })
                        .map(|surface| {
                            if name_span.start() <= ctx.byte_offset
                                && ctx.byte_offset < name_span.end()
                                && ctx.word == name.as_str()
                                && let Some(list_param) = list_type_param.as_deref()
                                && surface.starts_with("List(")
                            {
                                format!("List({list_param})")
                            } else {
                                surface
                            }
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
        let owned_tag_params;
        let (tag_types, tag_params) = if let Some(p) = package {
            (&p.tag_types, Some(&p.tag_params))
        } else {
            owned_tag_types = self
                .tag_types
                .iter()
                .map(|(id, ty)| (id.0, ty.clone()))
                .collect();
            owned_tag_params = self
                .tags
                .iter()
                .filter_map(|(id, tag)| tag.params.as_ref().map(|p| (id.0, p.clone())))
                .collect();
            (&owned_tag_types, Some(&owned_tag_params))
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
                            && let Some(h) = self.hover_pattern(&pattern.value, pattern_ctx)
                        {
                            return Some(h);
                        }
                    }
                }
                TypedExprKind::If(if_expr) => {
                    let mut patterns = Vec::new();
                    if_expr.condition.pattern_subjects(&mut patterns);
                    for (subject, pattern) in patterns {
                        if !self.span_contains(pattern.span_id, byte_offset) {
                            continue;
                        }
                        let subject_ty = Some(self.resolve_pattern_subject_ty(
                            &self.exprs.ty[subject.as_usize()],
                            package,
                        ));
                        let subject_ty = subject_ty.as_ref();
                        let pattern_ctx = HoverPatternCtx {
                            subject_ty,
                            ..pattern_ctx
                        };
                        if let Some(h) = self.hover_pattern(&pattern.value, pattern_ctx) {
                            return Some(h);
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn param_surface_in_body(&self, byte_offset: usize, word: &str) -> Option<String> {
        let word = Intern::<String>::from_ref(word);
        self.eval_ast.defs.values().find_map(|bind| {
            if !self.bind_body_contains(bind, byte_offset) {
                return None;
            }
            let typed_bind = self.defs.get(&DefId(bind.name));
            typed_bind
                .and_then(|typed_bind| self.typed_param_surface(typed_bind, &word))
                .or_else(|| {
                    bind.params
                        .as_ref()?
                        .get(&word)
                        .map(|parameter| param_kind_surface_for_hover(&parameter.kind))
                })
        })
    }

    fn typed_param_surface(&self, bind: &TypedBind, name: &Intern<String>) -> Option<String> {
        let index = bind
            .params
            .iter()
            .position(|(param_name, _)| param_name == name)?;
        self.typed_param_surface_at(bind, index)
    }

    fn typed_param_surface_at(&self, bind: &TypedBind, index: usize) -> Option<String> {
        let (_, ty) = bind.params.get(index)?;
        let ty = type_annotation_surface_for_hover_local(ty, &self.tag_types, &self.tags);
        let prefix = match bind
            .param_conventions
            .get(index)
            .copied()
            .unwrap_or_default()
        {
            ParamConvention::Own => "own ",
            ParamConvention::Observe => "ref ",
            ParamConvention::Mutate => "mut ",
            ParamConvention::Terminal => "",
        };
        Some(format!("{prefix}{ty}"))
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
            ast::BindValue::Unassigned => false,
        }
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
        let expr_id = match &bind.body {
            BindBody::Expr(expr_id) => expr_id,
            _ => return sig,
        };
        let Some(expr) = self.expr(*expr_id) else {
            return sig;
        };
        if matches!(
            self.type_registry.resolved_definition_for_type(expr.ty),
            Ty::Record { .. }
        ) {
            return sig;
        }
        if expr.facts.const_facts.const_value.is_none()
            && let Some(TypedExprKind::TagCall { variant_id, .. }) = self.exprs.kind_of(*expr_id)
        {
            sig.push(' ');
            if let Some((union, _, _)) =
                self.variant_map.get(&variant_id.name).and_then(|entries| {
                    entries
                        .iter()
                        .find(|(union, _, _)| *union == variant_id.union.0)
                })
            {
                sig.push_str(union.as_str());
                sig.push('.');
            }
            sig.push_str(variant_id.name.as_str());
            return sig;
        }
        let Some(const_val) = expr.facts.const_facts.const_value else {
            return sig;
        };
        sig.push(' ');
        if let ConstValue::Tag {
            name: variant,
            qual_path: None,
            ..
        } = const_val
            && let Some(entries) = self.variant_map.get(variant)
            && let [(union, _, _)] = entries.as_slice()
        {
            sig.push_str(union.as_str());
            sig.push('.');
        }
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
        _source: &str,
        byte_offset: usize,
        word: &str,
        package: Option<&PackageSemanticIndex>,
    ) -> Option<HoverTarget> {
        let word_interned = Intern::<String>::from_ref(word);
        let tag_id = TagId(word_interned);

        // 0. Cursor is on `self` keyword — show the actual type (`self Type`,
        //    `self ref Type`, `self mut Type`) using the tag's resolved type.
        //    Must run before param_at_byte since `self` may also appear as a parameter.
        if word == "self"
            && let Some((tag_name, modifier)) = self.has_self_context_at_byte(byte_offset)
        {
            return Some(HoverTarget::SelfRef { modifier, tag_name });
        }

        // 1. Cursor is exactly on a definition name.
        if self.defs.values().any(|b| {
            b.name.as_str() == word && self.span_table.get(b.name_span).contains(byte_offset)
        }) {
            return Some(HoverTarget::Definition(TagId(word_interned).0));
        }

        // 1a. Cursor is on a function parameter declaration or a use in that function body.
        if let Some((name, surface)) = self.param_at_byte(byte_offset, word) {
            return Some(HoverTarget::Param { name, surface });
        }

        // 1b. Cursor is on a type parameter (e.g. `x` in `Range(x) has start x`.
        if let Some((_, tag)) = self.tag_at_byte(byte_offset, word)
            && let Some(params) = &tag.params
            && params.contains_key(&Intern::<String>::from_ref(word))
        {
            return Some(HoverTarget::Param {
                name: Intern::<String>::from_ref(word),
                surface: "type parameter".to_string(),
            });
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
        } else if self.imported_trait_names.contains(&word_interned)
            && let Some(pkg) = package
            && pkg.tag_decl_for_word(&word_interned).is_some()
        {
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
        //    body (e.g. `Bool` in `has b Bool` or `@x` in `pointer @x`).
        //    Route to tag declaration directly — checked before `tag_at_byte` so
        //    it wins over the containing tag's body hover.
        if self.tags.contains_key(&tag_id) {
            return Some(HoverTarget::TagDecl(tag_id.0));
        }
        if self.imported_trait_names.contains(&word_interned)
            && let Some(pkg) = package
            && pkg.tag_decl_for_word(&word_interned).is_some()
        {
            return Some(HoverTarget::TagDecl(tag_id.0));
        }

        if word.starts_with(char::is_uppercase)
            && !self.tags.contains_key(&tag_id)
            && !self.imported_trait_names.contains(&word_interned)
        {
            return None;
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

        if let Some(surface) = self.record_field_surface_at_byte(byte_offset, word) {
            return Some(HoverTarget::RecordField {
                name: word_interned,
                surface,
            });
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
        let word = source
            .symbol_at_byte_offset(byte_offset)
            .or_else(|| source.word_at_byte_offset(byte_offset))?;

        let target = self.classify_hover(source, byte_offset, &word, package)?;

        self.render_hover(source, target, &word, byte_offset, package)
    }

    /// Render markdown for a classified [`HoverTarget`].
    fn render_hover(
        &self,
        _source: &str,
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
            HoverTarget::SelfRef { modifier, tag_name } => {
                // Resolve the receiver type from the tag declaration.
                let ty = self.tags.get(&TagId(tag_name)).map(|tag| &tag.resolved_ty);
                if let Some(ty) = ty {
                    let ty_str = if let Some(p) = package {
                        type_annotation_surface_for_hover(ty, &p.tag_types, Some(&p.tag_params))
                    } else {
                        type_annotation_surface_for_hover_local(ty, &self.tag_types, &self.tags)
                    };
                    let rendered = if modifier.as_str() == "self" {
                        format!("self {ty_str}")
                    } else {
                        format!("{} {ty_str}", modifier.as_str())
                    };
                    Some(HoverResult::single(
                        ast::hover_format::HoverDoc::new().gin(rendered).render(),
                    ))
                } else {
                    // Tag not found locally — check package index for cross-file tags.
                    let ty_str = package.and_then(|p| {
                        p.tag_types.get(&tag_name).map(|ty| {
                            type_annotation_surface_for_hover(ty, &p.tag_types, Some(&p.tag_params))
                        })
                    });
                    if let Some(ty_str) = ty_str {
                        let rendered = if modifier.as_str() == "self" {
                            format!("self {ty_str}")
                        } else {
                            format!("{} {ty_str}", modifier.as_str())
                        };
                        Some(HoverResult::single(
                            ast::hover_format::HoverDoc::new().gin(rendered).render(),
                        ))
                    } else {
                        // Last resort: just show the tag name.
                        let rendered = if modifier.as_str() == "self" {
                            format!("self {tag_name}")
                        } else {
                            format!("{} {tag_name}", modifier.as_str())
                        };
                        Some(HoverResult::single(
                            ast::hover_format::HoverDoc::new().gin(rendered).render(),
                        ))
                    }
                }
            }
            HoverTarget::Param { name, surface } => Some(HoverResult::single(
                ast::hover_format::HoverDoc::new()
                    .gin(format!("{} {}", name.as_str(), surface))
                    .render(),
            )),
            HoverTarget::RecordField { name, surface } => Some(HoverResult::single(
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

                if let Some(decl) = self.eval_ast.tags.get(&name)
                    && let ast::DeclareValue::Has(members) = &decl.value
                {
                    for member in members {
                        match member {
                            ast::HasMember::Property(p)
                                if p.name.as_str() == word
                                    && self.span_table.get(p.name_span).contains(byte_offset) =>
                            {
                                let member_key = Intern::<String>::from_ref(word);
                                let ty_surface = tag
                                    .record_field_types
                                    .get(&member_key)
                                    .cloned()
                                    .unwrap_or_else(|| {
                                        p.ty.as_ref()
                                            .map(|ty| ty.value.format_surface())
                                            .unwrap_or_default()
                                    });
                                let ty_surface = ty_surface.trim_start();
                                let shape = if ty_surface.starts_with('(') {
                                    format!("{word}{ty_surface}")
                                } else {
                                    format!("{word} {ty_surface}")
                                };
                                let mut hover = ast::hover_format::HoverDoc::new().gin(shape);
                                if let Some(doc) = tag.record_field_docs.get(&member_key) {
                                    hover = hover.prose(doc.clone());
                                }
                                let result = HoverResult::single(hover.render());
                                return Some(result.with_qualified_union(package, &tag_id.0));
                            }
                            ast::HasMember::Function(f)
                                if f.name.as_str() == word
                                    && self.span_table.get(f.name_span).contains(byte_offset) =>
                            {
                                let mut signature = String::new();
                                if !f.params.is_empty() {
                                    signature.push('(');
                                    let mut first = true;
                                    for (k, v) in f.params.iter() {
                                        if !first {
                                            signature.push_str(", ");
                                        }
                                        first = false;
                                        signature.push_str(k.as_str());
                                        match &v.kind {
                                            ast::ParameterKind::Tagged(sp) => {
                                                signature.push(' ');
                                                signature.push_str(&sp.value.format_surface());
                                            }
                                            ast::ParameterKind::ValueParam { ty } => {
                                                signature.push(' ');
                                                signature.push_str(&ty.value.format_surface());
                                            }
                                            ast::ParameterKind::Inferred { ty } => {
                                                signature.push(' ');
                                                signature.push_str(&ty.value.format_surface());
                                                signature.push_str(": ?");
                                            }
                                            ast::ParameterKind::Default(expr) => {
                                                use std::fmt::Write as _;
                                                let _ = write!(&mut signature, ": {:?}", expr);
                                            }
                                            ast::ParameterKind::Generic => {}
                                        }
                                        if let Some(conv) = f.conventions.get(k) {
                                            match conv {
                                                ast::ParamConvention::Observe => {
                                                    signature.push_str(" ref")
                                                }
                                                ast::ParamConvention::Mutate => {
                                                    signature.push_str(" mut")
                                                }
                                                ast::ParamConvention::Own => {
                                                    signature.push_str(" own")
                                                }
                                                ast::ParamConvention::Terminal => {}
                                            }
                                        }
                                    }
                                    signature.push(')');
                                }
                                if let Some(rt) = &f.return_ty {
                                    signature.push(' ');
                                    signature.push_str(&rt.value.format_surface());
                                }
                                if let Some(et) = &f.error_ty {
                                    signature.push_str(" or ");
                                    signature.push_str(&et.value.format_surface());
                                }
                                let rendered =
                                    format!("```gin\n{}{}\n```", f.name.as_str(), signature)
                                        .replacen(" (", "(", 1);
                                let rendered = rendered
                                    .strip_prefix("```gin\n")
                                    .and_then(|it| it.strip_suffix("\n```"))
                                    .unwrap_or(&rendered);
                                let mut hover = ast::hover_format::HoverDoc::new().gin(rendered);
                                if let Some(doc) = tag.record_field_docs.get(&f.name) {
                                    hover = hover.prose(doc.clone());
                                }
                                let result = HoverResult::single(hover.render());
                                return Some(result.with_qualified_union(package, &tag_id.0));
                            }
                            _ => {}
                        }
                    }
                }

                let member_key = Intern::<String>::from_ref(word);
                if let Some(ty_surface) = tag.record_field_types.get(&member_key) {
                    let shape = if ty_surface.starts_with('(') {
                        format!("{}{}", word, ty_surface)
                    } else {
                        format!("{} {}", word, ty_surface)
                    };
                    let mut hover = ast::hover_format::HoverDoc::new().gin(shape);
                    if let Some(doc) = tag.record_field_docs.get(&member_key) {
                        hover = hover.prose(doc.clone());
                    }
                    let result = HoverResult::single(hover.render());
                    return Some(result.with_qualified_union(package, &tag_id.0));
                }

                if let Ty::Record { fields, .. } = self
                    .type_registry
                    .resolved_definition_for_type(&tag.resolved_ty)
                    && let Some((_, fty)) = fields.iter().find(|(n, _)| *n == member_key)
                {
                    let ty_str = {
                        let display_ty: &Ty = match &**fty {
                            Ty::Ptr { inner } => inner.as_ref(),
                            Ty::Address { pointee, .. } => pointee.as_ref(),
                            other => other,
                        };
                        type_annotation_surface_for_hover_local(
                            display_ty,
                            &self.tag_types,
                            &self.tags,
                        )
                    };
                    let shape = if ty_str.starts_with('(') {
                        format!("{}{}", word, ty_str)
                    } else {
                        format!("{} {}", word, ty_str)
                    };
                    let member_doc = tag.record_field_docs.get(&member_key).map(|s| s.as_str());
                    let mut hover = ast::hover_format::HoverDoc::new().gin(shape);
                    if let Some(doc) = member_doc {
                        hover = hover.prose(doc.to_string());
                    }
                    let result = HoverResult::single(hover.render());
                    return Some(result.with_qualified_union(package, &tag_id.0));
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
                let resolved_definition =
                    self.type_registry.resolved_definition_for_type(resolved_ty);
                // For literal-constant unions (e.g. `Architecture is 'x86_64' or 'arm64'`),
                // show the parent tag declaration instead of just the literal label,
                // so the user sees the full union shape.
                if resolved_definition.union_literal_values().is_some() {
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
                let variant_label = const_union_variant_label(&resolved_definition, discriminant)
                    .unwrap_or_else(|| {
                        // Format variant with its fields (e.g. `Some(x)` not just `Some`)
                        let variant_map =
                            package.map(|p| &p.variant_map).unwrap_or(&self.variant_map);
                        let key = Intern::<String>::from_ref(word);
                        if let Some(entries) = variant_map.get(&key)
                            && let Some((_, _, fields)) = entries.first()
                        {
                            return if let Some(p) = package {
                                format_variant_pattern_label(word, fields, &p.tag_types, None)
                            } else {
                                format_variant_pattern_label_local(
                                    word,
                                    fields,
                                    &self.tag_types,
                                    &self.tags,
                                )
                            };
                        }
                        word.to_string()
                    });
                let result = HoverResult::single(self.hover_for_variant(
                    union_name.as_str(),
                    &variant_label,
                    &resolved_definition,
                ));
                Some(result.with_qualified_union(package, &union_name))
            }
            HoverTarget::Expr(expr_id) => {
                if let Some(surface) = self.param_surface_in_body(byte_offset, word) {
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
                    && self
                        .type_registry
                        .resolved_definition_for_type(expr_ref.ty)
                        .union_literal_values()
                        .is_some()
                {
                    return Some(HoverResult::single(format!(
                        "{} union\n---\n\n",
                        name.as_str()
                    )));
                }
                let ty_str = if let Some(p) = package {
                    type_annotation_surface_for_hover(
                        expr_ref.ty,
                        &p.tag_types,
                        Some(&p.tag_params),
                    )
                } else {
                    type_annotation_surface_for_hover_local(
                        expr_ref.ty,
                        &self.tag_types,
                        &self.tags,
                    )
                };
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
                        let cv = expr_ref.facts.const_facts.const_value.as_ref().or_else(|| {
                            let body_idx = body.as_usize();
                            if body_idx < self.exprs.facts.const_facts.const_value.len()
                                && self.exprs.facts.const_facts.const_value[body_idx].is_some()
                            {
                                return self.exprs.facts.const_facts.const_value[body_idx].as_ref();
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
                        if let Some(cv) = &expr_ref.facts.const_facts.const_value {
                            cv.to_hover_string()
                        } else {
                            format!(
                                "`{}` (variant of `{}`)",
                                variant_id.name.as_str(),
                                variant_id.union.0.as_str(),
                            )
                        }
                    }
                    TypedExprKind::Lit(_) => match &expr_ref.facts.const_facts.const_value {
                        Some(cv) => cv.to_hover_string(),
                        None => ty_str,
                    },
                    _ => ty_str,
                };
                let summary = self.integer_hover_summary(expr_id, summary);
                Some(HoverResult::single(
                    ast::hover_format::HoverDoc::new().inline(summary).render(),
                ))
            }
        }
    }

    fn integer_hover_summary(&self, expr_id: ExprId, summary: String) -> String {
        let ty = &self.exprs.ty[expr_id.as_usize()];
        let Some(validity) = self.type_registry.integer_validity_for_type(ty) else {
            return summary;
        };
        let identity = if ty.named_instance_stripping_reference_wrappers().is_some() {
            ty.format_for_hover()
        } else {
            "anonymous integer".to_string()
        };
        let knowledge = match self
            .exprs
            .integer_knowledge_of(expr_id)
            .and_then(Option::as_ref)
        {
            Some(ast::integer::IntegerKnowledge::Exact(
                ast::integer::CanonicalIntegerExpr::Value(value),
            )) => format!("exactly {value}"),
            Some(ast::integer::IntegerKnowledge::Exact(
                ast::integer::CanonicalIntegerExpr::Symbolic(value),
            )) => format!("exactly {value}"),
            Some(ast::integer::IntegerKnowledge::Domain(domain)) => domain
                .storage_hull()
                .map(|hull| format!("{} to {}", hull.min(), hull.max()))
                .unwrap_or_else(|| "symbolic domain".to_string()),
            Some(ast::integer::IntegerKnowledge::Unknown) | None => "unknown".to_string(),
            Some(ast::integer::IntegerKnowledge::Poison) => "poison".to_string(),
        };
        let validity = validity
            .domain()
            .storage_hull()
            .map(|hull| format!("{} to {}", hull.min(), hull.max()))
            .unwrap_or_else(|| "no finite hull".to_string());
        let representation = self
            .type_registry
            .integer_width_for_type(ty)
            .map(|width| format!("i{width}"))
            .unwrap_or_else(|| "unresolved".to_string());
        let interpretation = self
            .type_registry
            .nominal_integer_interpretation_for_type(ty)
            .map(|interpretation| format!("\ninterpretation: {interpretation:?}"))
            .unwrap_or_default();
        format!(
            "{summary}\n{identity}\ncurrent knowledge: {knowledge}\nvalidity: {validity}\nruntime representation: {representation}{interpretation}"
        )
    }

    fn record_field_surface_at_byte(&self, byte_offset: usize, word: &str) -> Option<String> {
        let field = Intern::<String>::from_ref(word);
        for (expr_index, kind) in self.exprs.kind.iter().enumerate() {
            let TypedExprKind::AggregateGet { base, index } = kind else {
                continue;
            };
            if !self
                .span_table
                .get(self.exprs.span[expr_index])
                .contains(byte_offset)
            {
                continue;
            }
            let base_definition = self
                .exprs
                .ty
                .get(base.as_usize())
                .map(|ty| self.type_registry.resolved_definition_for_type(ty));
            if let Some(Ty::Record {
                name: tag_name,
                fields,
                ..
            }) = base_definition
                && let Some((name, ty)) = fields.get(*index)
                && *name == field
            {
                if let Some(tag) = self.tags.get(&TagId(tag_name))
                    && let Some(surface) = tag.record_field_types.get(&field)
                {
                    return Some(surface.clone());
                }
                return Some(format_ty_for_hover(ty));
            }
            let TypedExprKind::FnCall { target, .. } = self.exprs.kind.get(base.as_usize())? else {
                continue;
            };
            let bind = self.defs.get(target)?;
            let body = match &bind.body {
                BindBody::Expr(expr) => Some(*expr),
                BindBody::Body { exprs, ret } => (*ret).or_else(|| exprs.last().copied()),
                BindBody::Unlowered => None,
            }?;
            let TypedExprKind::TagCall { variant_id, .. } = self.exprs.kind.get(body.as_usize())?
            else {
                continue;
            };
            let tag = self.tags.get(&variant_id.union)?;
            let Ty::Record { fields, .. } = self
                .type_registry
                .resolved_definition_for_type(&tag.resolved_ty)
            else {
                continue;
            };
            let (name, _) = fields.get(*index)?;
            if *name == field {
                return tag.record_field_types.get(&field).cloned();
            }
            continue;
        }
        None
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
        match self.type_registry.resolved_definition_for_type(expr_ref.ty) {
            Ty::Record { fields, .. } => {
                let interned_field = Intern::<String>::from_ref(&field_name);
                for (name, ty) in fields {
                    if name == interned_field {
                        return Some(format_ty_for_hover(&ty));
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
            for flaw in &self.exprs.facts.semantic_facts.flaws[i] {
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
