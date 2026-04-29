//! Type representation, environment construction, and correctness checking.
//!
//! This module owns the "build" and "validate" phases of the type system:
//!
//! - **[`Ty`]** — the canonical type representation after resolving AST tag names.
//! - **[`TyEnv`]** — the owned type environment, built from parsed ASTs. Holds the
//!   resolved `tag_types` and `fn_return_types` maps, plus a `variant_map` for union
//!   lookups. Construction happens in [`TyEnv::from_multiple_file_asts`].
//! - **Check methods** — [`check_unknowns`], [`check_bind`], [`check_expr`], plus type-surface checking on [`Expr`]
//!   walk the AST and accumulate diagnostics for unknown names, type mismatches, etc.
//!
//! Pure type inference ("given an env, what type does this expression have?") lives in
//! [`infer.rs`] and is accessed through [`TyInferEnv`] / the [`TyInfer`](crate::TyInfer) trait.
//! This module provides [`TyEnv::infer_env`] to bridge between the two.

use crate::{TyInfer, TyInferEnv};
use ast::WhenArm;
use ast::{
    Bind, BindValue, DeclareValue, Expr, FileAst, FnCall, FormatPart, HasSpanId, IfCondition, Loop,
    ParameterKind, Spanned, type_surface_mangle_name,
};
use i256::I256;
use internment::Intern;
use std::collections::HashMap;

/// Symbol name for a callee: `foo` or `io.print` (matches codegen).
pub fn mangled_fn_call_name(call: &FnCall) -> Intern<String> {
    if call.path.segments.is_empty() {
        call.path.root
    } else {
        let segs: Vec<&str> = call.path.segments.iter().map(|s| s.as_str()).collect();
        Intern::<String>::new(format!("{}.{}", call.path.root.as_str(), segs.join(".")))
    }
}

pub(crate) fn is_type_surface(e: &Expr) -> bool {
    matches!(
        e,
        Expr::TypeNominal(..) | Expr::TypeQualified(_) | Expr::TypeGeneric { .. }
    )
}

pub(crate) fn resolve_type_expr_from_map(e: &Expr, tag_types: &HashMap<Intern<String>, Ty>) -> Ty {
    match e {
        Expr::TypeNominal(name, _) => tag_types.get(name).cloned().unwrap_or(Ty::Int {
            width: 64,
            signed: true,
            value: None,
        }),
        Expr::TypeGeneric { name, params, .. } => match name.as_str() {
            "Ptr" | "Ref" => {
                let inner = params
                    .iter()
                    .find_map(|(_, kind)| match kind {
                        ParameterKind::Tagged(sp) => {
                            Some(resolve_type_expr_from_map(&sp.0, tag_types))
                        }
                        _ => None,
                    })
                    .unwrap_or(Ty::Opaque(*name));
                if name.as_str() == "Ptr" {
                    Ty::Ptr {
                        inner: Box::new(inner),
                    }
                } else {
                    Ty::Ref {
                        inner: Box::new(inner),
                    }
                }
            }
            _ => tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name)),
        },
        Expr::TypeQualified(path) => tag_types
            .get(&path.root)
            .cloned()
            .unwrap_or(Ty::Opaque(path.root)),
        _ => Ty::Opaque(Intern::<String>::from_ref("?")),
    }
}

/// Resolved type — the canonical representation after resolving declared type names against declarations.
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Int {
        width: u8,
        signed: bool,
        /// Known compile-time value when constant-folded, otherwise `None`.
        value: Option<i128>,
    },
    Float {
        /// Known compile-time value when constant-folded, otherwise `None`.
        value: Option<f64>,
    },
    Bool,
    Unit,
    Record {
        name: Intern<String>,
        fields: Vec<(Intern<String>, Box<Ty>)>,
    },
    Union {
        name: Intern<String>,
        /// Each variant: (variant_name, [(field_name, field_type)]) in declaration order.
        #[allow(clippy::type_complexity)]
        variants: Vec<(Intern<String>, Vec<(Intern<String>, Box<Ty>)>)>,
    },
    /// Unresolved / generic type — falls back to `i64` in codegen.
    Opaque(Intern<String>),
    /// Fixed-size stack-allocated array (`(T.new; N)`). The value is a `!llvm.ptr`.
    Array {
        elem: Box<Ty>,
        size: usize,
    },
    /// Raw pointer — erases `T` from layout, kept only for type checking. Maps to `!llvm.ptr`.
    Ptr {
        inner: Box<Ty>,
    },
    /// Reference (borrow-checked pointer in future). Same layout as `Ptr` for now.
    Ref {
        inner: Box<Ty>,
    },
    /// Positional tuple VALUE — used for tuple literals `(e1, e2, …)`. Maps to an LLVM struct.
    Tuple(Vec<Ty>),
}

impl Ty {
    /// Return record fields in layout order.
    ///
    /// Fields are sorted by descending alignment (then descending size, then declaration order
    /// for ties) so the compiler packs them without padding. The programmer writes fields in
    /// any logical order; the physical layout is determined here.
    /// Empty for non-record types.
    pub fn record_fields_sorted(&self) -> Vec<(&Intern<String>, &Ty)> {
        if let Ty::Record { fields, .. } = self {
            let mut indexed: Vec<(usize, &Intern<String>, &Ty)> = fields
                .iter()
                .enumerate()
                .map(|(i, (k, v))| (i, k, v.as_ref()))
                .collect();
            indexed.sort_by(|(i, _, a), (j, _, b)| {
                let align_a = ty_alignment(a);
                let align_b = ty_alignment(b);
                align_b
                    .cmp(&align_a)
                    .then_with(|| ty_byte_size_static(b).cmp(&ty_byte_size_static(a)))
                    .then_with(|| i.cmp(j))
            });
            indexed.into_iter().map(|(_, k, v)| (k, v)).collect()
        } else {
            vec![]
        }
    }

    pub fn is_int(&self) -> bool {
        matches!(self, Ty::Int { .. })
    }

    pub fn is_unsigned_int(&self) -> bool {
        matches!(self, Ty::Int { signed: false, .. })
    }

    pub fn is_signed_int(&self) -> bool {
        matches!(self, Ty::Int { signed: true, .. })
    }

    pub fn is_float(&self) -> bool {
        matches!(self, Ty::Float { .. })
    }

    pub fn is_ptr_or_ref(&self) -> bool {
        matches!(self, Ty::Ptr { .. } | Ty::Ref { .. })
    }

    pub fn is_union(&self) -> bool {
        matches!(self, Ty::Union { .. })
    }

    pub fn is_record(&self) -> bool {
        matches!(self, Ty::Record { .. })
    }

    pub fn is_unit(&self) -> bool {
        matches!(self, Ty::Unit)
    }
}

/// Type alias for union variant fields: (field_name, field_type)
#[allow(dead_code)]
type UnionFields = Vec<(Intern<String>, Box<Ty>)>;

/// Type alias for union variants: (variant_name, fields)
#[allow(dead_code)]
type UnionVariants = Vec<(Intern<String>, UnionFields)>;

/// Type alias for variant map entries: (union_name, discriminant, fields)
type VariantMapEntry = (Intern<String>, usize, Vec<(Intern<String>, Ty)>);

/// Type alias for the variant map: variant_name -> [(union_name, discriminant, fields)]
type VariantMap = HashMap<Intern<String>, Vec<VariantMapEntry>>;

/// Type alias for variant lookup result: (union_name, discriminant, field_slice)
type VariantLookupResult<'a> = (Intern<String>, usize, &'a [(Intern<String>, Ty)]);

/// Type environment built from a `FileAst`. Resolves tag names to `Ty` and infers
/// function parameter / return types.
#[derive(PartialEq)]
pub struct TyEnv {
    pub tag_types: HashMap<Intern<String>, Ty>,
    pub fn_return_types: HashMap<Intern<String>, Ty>,
    /// Reverse map: variant name → [(parent_union_name, discriminant_index, payload_fields)]
    /// A variant may appear in multiple unions if names collide; shape-based disambiguation is TODO.
    pub variant_map: VariantMap,
}

impl TyEnv {
    pub fn from_file_ast(ast: &FileAst) -> Self {
        Self::from_multiple_file_asts(std::slice::from_ref(ast))
    }

    pub fn from_multiple_file_asts(files: &[FileAst]) -> Self {
        let mut tag_types = HashMap::new();

        // Resolve all tag types from all files
        for ast in files {
            for name in ast.tags.keys() {
                let ty = resolve_name_from_files(*name, files, 0);
                tag_types.insert(*name, ty);
            }
        }

        // Register built-in types not declared in any gin source file.
        tag_types
            .entry(Intern::<String>::from_ref("Str"))
            .or_insert_with(str_record_ty);

        // Build variant reverse map from all union types.
        let mut variant_map: VariantMap = HashMap::new();
        for (union_name, ty) in &tag_types {
            if let Ty::Union { variants, .. } = ty {
                for (i, (variant_name, fields)) in variants.iter().enumerate() {
                    let field_tys: Vec<(Intern<String>, Ty)> =
                        fields.iter().map(|(n, t)| (*n, *t.clone())).collect();
                    variant_map
                        .entry(*variant_name)
                        .or_default()
                        .push((*union_name, i, field_tys));
                }
            }
        }

        // First pass: seed with explicitly-annotated return types from all files.
        let mut fn_return_types = HashMap::new();
        for ast in files {
            for (name, bind) in &ast.defs {
                if !bind.attributes().matches_current_platform() {
                    continue;
                }
                let env = TyInferEnv {
                    tag_types: &tag_types,
                    fn_return_types: &HashMap::new(),
                    locals: &HashMap::new(),
                };
                let ret = bind.infer_ty(&env);
                fn_return_types.insert(*name, ret);
            }
        }
        // Second pass: refine with cross-function call resolution now that
        // explicitly-typed functions are in the map.
        for ast in files {
            for (name, bind) in &ast.defs {
                if !bind.attributes().matches_current_platform() {
                    continue;
                }
                let env = TyInferEnv {
                    tag_types: &tag_types,
                    fn_return_types: &fn_return_types,
                    locals: &HashMap::new(),
                };
                let ret = bind.infer_ty(&env);
                fn_return_types.insert(*name, ret);
            }
        }

        TyEnv {
            tag_types,
            fn_return_types,
            variant_map,
        }
    }

    /// Resolve a type-surface [`Expr`] to a `Ty` using this environment's `tag_types`.
    pub fn resolve_type_expr(&self, e: &Expr) -> Ty {
        resolve_type_expr_from_map(e, &self.tag_types)
    }

    /// Resolve a type-surface [`Expr`] only when `e` is a nominal, qualified, or generic type form.
    pub fn resolve_type_surface(&self, e: &Expr) -> Option<Ty> {
        is_type_surface(e).then(|| resolve_type_expr_from_map(e, &self.tag_types))
    }

    /// Resolve a `ParameterKind` to a `Ty`.
    ///
    /// This consolidates the common pattern of converting parameters to their types.
    fn resolve_parameter_kind(&self, kind: &ParameterKind) -> Ty {
        match kind {
            ParameterKind::Tagged(sp) => {
                if is_type_surface(&sp.0) {
                    self.resolve_type_expr(&sp.0)
                } else {
                    Ty::Opaque(Intern::<String>::from_ref("?"))
                }
            }
            ParameterKind::Generic => Ty::Int {
                width: 64,
                signed: true,
                value: None,
            },
            ParameterKind::Default(expr) => {
                let empty: HashMap<Intern<String>, Ty> = HashMap::new();
                expr.infer_ty(&self.infer_env(&empty))
            }
        }
    }

    /// Return the typed parameter list for a function binding.
    /// Preserves insertion order of the `Parameters` map.
    pub fn param_types<'a>(&self, bind: &'a Bind) -> Vec<(&'a Intern<String>, Ty)> {
        match bind.params().as_ref() {
            None => vec![],
            Some(params) => params
                .iter()
                .map(|(name, kind)| (name, self.resolve_parameter_kind(kind)))
                .collect(),
        }
    }

    /// Look up the pre-computed return type of a top-level function by name.
    pub fn fn_return_ty(&self, name: &Intern<String>) -> Option<&Ty> {
        self.fn_return_types.get(name)
    }

    /// Look up a declared type by its tag name.
    pub fn lookup_tag(&self, name: Intern<String>) -> Option<&Ty> {
        self.tag_types.get(&name)
    }

    /// Look up which union a variant belongs to, its discriminant index, and payload fields.
    ///
    /// Returns `(union_name, discriminant, [(field_name, field_type)])` in declaration order.
    /// If multiple unions declare a variant with the same name, the first match is returned.
    /// TODO: shape-based disambiguation per spec (answer #6).
    pub fn lookup_variant(&self, name: Intern<String>) -> Option<VariantLookupResult<'_>> {
        let candidates = self.variant_map.get(&name)?;
        candidates
            .first()
            .map(|(union, idx, fields)| (*union, *idx, fields.as_slice()))
    }

    /// Return all variant names belonging to `union_name`.
    pub fn all_variants_of(&self, union_name: Intern<String>) -> Vec<Intern<String>> {
        self.variant_map
            .iter()
            .filter_map(|(variant_name, entries)| {
                if entries.iter().any(|(u, _, _)| *u == union_name) {
                    Some(*variant_name)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Build the union→variants reverse map for use in flow analysis display.
    pub fn build_union_to_variants(&self) -> HashMap<Intern<String>, Vec<Intern<String>>> {
        let mut map: HashMap<Intern<String>, Vec<Intern<String>>> = HashMap::new();
        for (variant_name, entries) in &self.variant_map {
            for (union_name, _, _) in entries {
                map.entry(*union_name).or_default().push(*variant_name);
            }
        }
        map
    }

    /// Resolve the union type reachable via a dot expression from `name`.
    ///
    /// Returns `Some(Ty::Union { .. })` when `name` is either:
    /// - A union type directly (e.g., `Bool`, `Maybe`)
    /// - A variable binding whose type annotation names a union type
    ///
    /// Returns `None` for non-union types and unresolvable names.
    pub fn resolve_dot_type(&self, ast: &FileAst, name: Intern<String>) -> Option<Ty> {
        if let Some(ty) = self.lookup_tag(name)
            && ty.is_union()
        {
            return Some(ty.clone());
        }
        let type_name = binding_type_annotation(ast, name)?;
        self.lookup_tag(type_name).cloned()
    }

    /// Build a `TyInferEnv` from this `TyEnv` and a local variable set.
    ///
    /// Callers then use `expr.infer_ty(&ty_env.infer_env(&locals))` directly,
    /// keeping inference logic in [`infer.rs`](crate::infer).
    pub fn infer_env<'a>(&'a self, locals: &'a dyn crate::LocalTypes) -> crate::TyInferEnv<'a> {
        crate::TyInferEnv {
            tag_types: &self.tag_types,
            fn_return_types: &self.fn_return_types,
            locals,
        }
    }
}

// ─── Internals ───────────────────────────────────────────────────────────────

/// Find the type annotation name for a binding named `name` in the AST.
/// Checks top-level defs first, then local bindings inside function bodies.
fn binding_type_annotation(ast: &FileAst, name: Intern<String>) -> Option<Intern<String>> {
    if let Some(bind) = ast.defs().values().find(|b| b.name() == name) {
        return bind.type_annotation.as_ref().map(|(tn, _)| *tn);
    }
    ast.defs().values().find_map(|bind| {
        let BindValue::Body { exprs, .. } = bind.value() else {
            return None;
        };
        exprs.iter().find_map(|expr| {
            let Expr::Bind(b) = &**expr else { return None };
            if b.name() == name {
                b.type_annotation.as_ref().map(|(tn, _)| *tn)
            } else {
                None
            }
        })
    })
}

/// Returns the alignment (in bytes) of a type.
/// Calculate the size needed for a union discriminant based on number of variants.
/// For 2 variants (like Bool), only 1 byte is needed.
/// For 3-256 variants, 1 byte is needed.
/// For 257-65536 variants, 2 bytes are needed.
/// Otherwise, 8 bytes.
fn ty_union_discriminant_size(num_variants: usize) -> usize {
    if num_variants <= 256 {
        1
    } else if num_variants <= 65536 {
        2
    } else {
        8
    }
}

/// Type alias for union variant fields: (variant_name, [(field_name, field_type)])
type UnionVariant<'a> = (Intern<String>, Vec<(Intern<String>, Box<Ty>)>);

/// Calculate the maximum field size across all union variants.
fn ty_union_max_field_size(variants: &[UnionVariant<'_>]) -> usize {
    variants
        .iter()
        .flat_map(|(_, fields)| fields.iter().map(|(_, ft)| ty_byte_size_static(ft)))
        .max()
        .unwrap_or(0)
}

pub fn ty_alignment(ty: &Ty) -> usize {
    match ty {
        Ty::Int { width: 8, .. } | Ty::Bool => 1,
        Ty::Int { width: 16, .. } => 2,
        Ty::Int { width: 32, .. } => 4,
        Ty::Int { width: 128, .. } => 16,
        Ty::Int { .. } | Ty::Float { .. } | Ty::Array { .. } | Ty::Ptr { .. } | Ty::Ref { .. } => 8,
        Ty::Unit | Ty::Opaque(_) => 8,
        Ty::Record { fields, .. } => fields
            .iter()
            .map(|(_, ft)| ty_alignment(ft))
            .max()
            .unwrap_or(1),
        Ty::Union { variants, .. } => {
            // Check if all variants have no fields
            let all_empty = variants.iter().all(|(_, fields)| fields.is_empty());
            if all_empty {
                // Only discriminant needed, align to 1 byte
                1
            } else {
                // Align to the maximum alignment of all field types
                variants
                    .iter()
                    .flat_map(|(_, fields)| fields.iter().map(|(_, ft)| ty_alignment(ft)))
                    .max()
                    .unwrap_or(8)
            }
        }
        Ty::Tuple(fields) => fields.iter().map(ty_alignment).max().unwrap_or(1),
    }
}

/// Returns the in-memory size (bytes) of a type without recursing into the typeck context.
pub fn ty_byte_size_static(ty: &Ty) -> usize {
    match ty {
        Ty::Int { width: 8, .. } | Ty::Bool => 1,
        Ty::Int { width: 16, .. } => 2,
        Ty::Int { width: 32, .. } => 4,
        Ty::Int { width: 128, .. } => 16,
        Ty::Int { .. } | Ty::Float { .. } => 8,
        Ty::Array { .. } | Ty::Ptr { .. } | Ty::Ref { .. } => 8,
        Ty::Unit | Ty::Opaque(_) => 8,
        Ty::Record { fields, .. } => fields.iter().map(|(_, ft)| ty_byte_size_static(ft)).sum(),
        Ty::Union { variants, .. } => {
            // For unions with 2 variants and no fields (like Bool), use 1 byte
            let all_empty = variants.iter().all(|(_, fields)| fields.is_empty());
            if all_empty && variants.len() <= 256 {
                // Only discriminant needed, 1 byte is sufficient for 2-256 variants
                1
            } else if all_empty {
                // Many variants but no fields
                ty_union_discriminant_size(variants.len())
            } else {
                // Calculate: discriminant + max field size
                let discriminant_size = ty_union_discriminant_size(variants.len());
                let max_field_size = ty_union_max_field_size(variants);
                discriminant_size + max_field_size
            }
        }
        Ty::Tuple(fields) => fields.iter().map(ty_byte_size_static).sum(),
    }
}

/// Canonical `Str` record type: `{ pointer: Ptr(Byte), len: Int }`.
pub fn str_record_ty() -> Ty {
    Ty::Record {
        name: Intern::<String>::from_ref("Str"),
        fields: vec![
            (
                Intern::<String>::from_ref("pointer"),
                Box::new(Ty::Ptr {
                    inner: Box::new(Ty::Int {
                        width: 8,
                        signed: false,
                        value: None,
                    }),
                }),
            ),
            (
                Intern::<String>::from_ref("len"),
                Box::new(Ty::Int {
                    width: 64,
                    signed: false,
                    value: None,
                }),
            ),
        ],
    }
}

fn range_bit_width(min: I256, max: I256) -> u8 {
    let range = max - min;
    if range <= I256::from_i128(u8::MAX as i128 + 1) {
        8
    } else if range <= I256::from_i128(u16::MAX as i128 + 1) {
        16
    } else if range <= I256::from_i128(u32::MAX as i128 + 1) {
        32
    } else if range <= I256::from_i128(u64::MAX as i128 + 1) {
        64
    } else {
        128
    }
}

fn resolve_type_expr_ref(
    e: &Expr,
    raw: &HashMap<Intern<String>, &DeclareValue>,
    recursion_depth: usize,
) -> Ty {
    match e {
        Expr::TypeNominal(name, _) => resolve_name(*name, raw, recursion_depth),
        Expr::TypeGeneric { name, params, .. } => match name.as_str() {
            "Ptr" | "Ref" => {
                let inner = params
                    .iter()
                    .find_map(|(_, kind)| match kind {
                        ParameterKind::Tagged(sp) => {
                            Some(resolve_type_expr_ref(&sp.0, raw, recursion_depth + 1))
                        }
                        _ => None,
                    })
                    .unwrap_or(Ty::Opaque(*name));
                if name.as_str() == "Ptr" {
                    Ty::Ptr {
                        inner: Box::new(inner),
                    }
                } else {
                    Ty::Ref {
                        inner: Box::new(inner),
                    }
                }
            }
            _ => resolve_name(*name, raw, recursion_depth + 1),
        },
        Expr::TypeQualified(path) => {
            // For qualified types like Bool.True, resolve the union type
            resolve_name(path.root, raw, recursion_depth)
        }
        _ => Ty::Opaque(Intern::<String>::from_ref("?")),
    }
}

fn resolve_name_from_files(name: Intern<String>, files: &[FileAst], recursion_depth: usize) -> Ty {
    // Build a temporary lookup map for this file set
    let mut raw: HashMap<Intern<String>, &DeclareValue> = HashMap::new();
    for ast in files {
        for (k, v) in ast.tags.iter() {
            raw.insert(*k, v.value());
        }
    }

    resolve_name(name, &raw, recursion_depth)
}

fn resolve_name(
    name: Intern<String>,
    raw: &HashMap<Intern<String>, &DeclareValue>,
    recursion_depth: usize,
) -> Ty {
    if recursion_depth > 16 {
        return Ty::Opaque(name);
    }
    match raw.get(&name) {
        Some(DeclareValue::Alias(sp)) => {
            if is_type_surface(&sp.0) {
                resolve_type_expr_ref(&sp.0, raw, recursion_depth + 1)
            } else {
                Ty::Opaque(name)
            }
        }
        Some(DeclareValue::Range(start, end)) => Ty::Int {
            width: range_bit_width(*start, *end),
            signed: start.is_negative(),
            value: None,
        },
        Some(DeclareValue::InRange(start, end)) => Ty::Int {
            width: range_bit_width(*start, *end),
            signed: start.is_negative(),
            value: None,
        },
        Some(DeclareValue::Record(params)) => {
            let fields = params
                .iter()
                .map(|(field_name, kind)| {
                    let field_ty = match kind {
                        ParameterKind::Tagged(sp) => {
                            if is_type_surface(&sp.0) {
                                resolve_type_expr_ref(&sp.0, raw, recursion_depth + 1)
                            } else {
                                Ty::Opaque(*field_name)
                            }
                        }
                        ParameterKind::Generic => Ty::Opaque(*field_name),
                        ParameterKind::Default(_) => Ty::Int {
                            width: 64,
                            signed: true,
                            value: None,
                        },
                    };
                    (*field_name, Box::new(field_ty))
                })
                .collect();
            Ty::Record { name, fields }
        }
        Some(DeclareValue::Union { variants }) => {
            let resolved = variants
                .iter()
                .filter_map(|v| {
                    let shape = &v.shape().0;
                    if !is_type_surface(shape) {
                        return None;
                    }
                    let variant_name = Intern::<String>::from_ref(type_surface_mangle_name(shape));
                    let fields = match shape {
                        Expr::TypeGeneric { params, .. } if !params.is_empty() => params
                            .iter()
                            .filter_map(|(field_name, kind)| match kind {
                                ParameterKind::Tagged(sp) => is_type_surface(&sp.0).then_some((
                                    *field_name,
                                    Box::new(resolve_type_expr_ref(
                                        &sp.0,
                                        raw,
                                        recursion_depth + 1,
                                    )),
                                )),
                                ParameterKind::Generic => {
                                    Some((*field_name, Box::new(Ty::Opaque(*field_name))))
                                }
                                _ => None,
                            })
                            .collect(),
                        _ => vec![],
                    };
                    Some((variant_name, fields))
                })
                .collect();
            Ty::Union {
                name,
                variants: resolved,
            }
        }
        _ => Ty::Opaque(name),
    }
}

// ─── Unknown reference checking ──────────────────────────────────────────────

impl TyEnv {
    pub fn check_unknowns(&self, ast: &FileAst, symptoms: &mut Vec<diagnostic::Diagnostic>) {
        for bind in ast.defs.values() {
            if !bind.attributes().matches_current_platform() {
                continue;
            }
            let mut locals = HashMap::new();
            if let Some(params) = bind.params() {
                for (name, kind) in params.iter() {
                    locals.insert(*name, self.resolve_parameter_kind(kind));
                }
            }
            self.check_bind(bind, symptoms, &locals);
        }
    }

    fn check_bind(
        &self,
        bind: &Bind,
        symptoms: &mut Vec<diagnostic::Diagnostic>,
        locals: &HashMap<Intern<String>, Ty>,
    ) {
        if let Some(sp) = &bind.return_tag
            && is_type_surface(&sp.0)
        {
            self.check_type_expr(&sp.0, symptoms);
            if let Some(Ty::Union {
                name: union_name,
                variants,
            }) = self.lookup_tag(Intern::<String>::from_ref(type_surface_mangle_name(&sp.0)))
            {
                let valid_variants: Vec<Intern<String>> =
                    variants.iter().map(|(vname, _)| *vname).collect();
                check_return_variants(bind, &valid_variants, *union_name, symptoms);
            }
        }
        match bind.value() {
            BindValue::Expr(expr) => self.check_expr(expr, symptoms, locals),
            BindValue::Body { exprs, ret } => {
                use diagnostic::DiagnosticLike;
                use diagnostic::type_::TypeSymptom;

                let mut body_locals = locals.clone();
                for (i, expr) in exprs.iter().enumerate() {
                    if let Expr::Bind(inner) = &**expr {
                        self.check_bind(inner, symptoms, &body_locals);
                        let name = inner.name();
                        let used = exprs[i + 1..].iter().any(|e| expr_references_name(e, name))
                            || ret
                                .0
                                .as_ref()
                                .is_some_and(|e| expr_references_name(e, name));
                        if !used && !name.starts_with('_') {
                            symptoms.push(
                                TypeSymptom::UnusedBinding {
                                    name: name.to_string(),
                                }
                                .into_diagnostic(inner.name_span),
                            );
                        }
                        body_locals.insert(name, {
                            let env = TyInferEnv {
                                tag_types: &self.tag_types,
                                fn_return_types: &self.fn_return_types,
                                locals: &HashMap::new(),
                            };
                            inner.infer_ty(&env)
                        });
                    } else {
                        self.check_expr(expr, symptoms, &body_locals);
                    }
                }
                if let Some(ret_expr) = &ret.0 {
                    self.check_expr(ret_expr, symptoms, &body_locals);
                }
            }
            BindValue::Extern => {}
        }
    }

    fn check_expr(
        &self,
        expr: &Expr,
        symptoms: &mut Vec<diagnostic::Diagnostic>,
        locals: &HashMap<Intern<String>, Ty>,
    ) {
        use diagnostic::DiagnosticLike;
        use diagnostic::type_::TypeSymptom;

        match expr {
            Expr::FnCall(call) => {
                let name = call.path.root;
                let mangled = mangled_fn_call_name(call);
                if let Some(args) = &call.args {
                    if self.fn_return_ty(&mangled).is_none() {
                        symptoms.push(
                            TypeSymptom::UnknownBinding {
                                name: mangled.to_string(),
                            }
                            .into_diagnostic(call.path.span_id()),
                        );
                    }
                    for arg in args {
                        self.check_expr(arg, symptoms, locals);
                    }
                } else if call.path.segments.is_empty()
                    && !locals.contains_key(&name)
                    && self.fn_return_ty(&mangled).is_none()
                {
                    symptoms.push(
                        TypeSymptom::UnknownBinding {
                            name: mangled.to_string(),
                        }
                        .into_diagnostic(call.path.span_id()),
                    );
                }
            }
            Expr::Bind(bind) => self.check_bind(bind, symptoms, locals),
            Expr::Binary(bin) => {
                self.check_expr(&bin.lhs, symptoms, locals);
                self.check_expr(&bin.rhs, symptoms, locals);
            }
            Expr::When(w) => {
                // Infer subject type once for pattern-variant checking.
                let subject_ty = w
                    .subject
                    .as_ref()
                    .map(|s| s.infer_ty(&self.infer_env(locals)));
                if let Some(subject) = &w.subject {
                    self.check_expr(subject, symptoms, locals);
                }
                for arm in &w.arms {
                    match arm {
                        WhenArm::Cond { condition, body } => {
                            self.check_expr(condition, symptoms, locals);
                            self.check_expr(body, symptoms, locals);
                        }
                        WhenArm::Is { pattern, body } => {
                            if is_type_surface(&pattern.0) {
                                let surface_name = type_surface_mangle_name(&pattern.0);
                                let variant_name = Intern::<String>::from_ref(surface_name);
                                match &subject_ty {
                                    Some(Ty::Union {
                                        name: union_name,
                                        variants,
                                    }) => {
                                        if !variants.iter().any(|(vname, _)| vname == &variant_name)
                                        {
                                            symptoms.push(
                                                TypeSymptom::NotAVariant {
                                                    name: surface_name.to_string(),
                                                    union_name: union_name.to_string(),
                                                }
                                                .into_diagnostic(pattern.1),
                                            );
                                        }
                                    }
                                    _ => {
                                        if self.lookup_variant(variant_name).is_none() {
                                            symptoms.push(
                                                TypeSymptom::UnknownTag {
                                                    name: surface_name.to_string(),
                                                }
                                                .into_diagnostic(pattern.1),
                                            );
                                        }
                                    }
                                }
                                check_type_pattern_default_exprs(&pattern.0, &mut |e| {
                                    self.check_expr(e, symptoms, locals);
                                });
                            } else {
                                symptoms.push(
                                    TypeSymptom::UnknownTag {
                                        name: "invalid is-pattern".to_string(),
                                    }
                                    .into_diagnostic(pattern.1),
                                );
                            }
                            self.check_expr(body, symptoms, locals);
                        }
                        WhenArm::Else(body) => {
                            self.check_expr(body, symptoms, locals);
                        }
                    }
                }
            }
            Expr::If(if_expr) => match &if_expr.condition {
                IfCondition::Bool(cond) => {
                    self.check_expr(cond, symptoms, locals);
                    for e in &if_expr.body {
                        self.check_expr(e, symptoms, locals);
                    }
                }
                IfCondition::Pattern { subject, pattern } => {
                    self.check_expr(subject, symptoms, locals);
                    let mut if_locals = locals.clone();
                    if is_type_surface(&pattern.0) {
                        if let Expr::TypeGeneric { params, .. } = &pattern.0 {
                            if_locals.extend(params.iter().filter_map(|(k, _)| {
                                if k.as_str() != "_" {
                                    Some((*k, Ty::Opaque(*k)))
                                } else {
                                    None
                                }
                            }));
                        }
                        check_type_pattern_default_exprs(&pattern.0, &mut |e| {
                            self.check_expr(e, symptoms, locals);
                        });
                    } else {
                        symptoms.push(
                            TypeSymptom::UnknownTag {
                                name: "invalid is-pattern".to_string(),
                            }
                            .into_diagnostic(pattern.1),
                        );
                    }
                    for e in &if_expr.body {
                        self.check_expr(e, symptoms, &if_locals);
                    }
                }
            },
            Expr::Loop(loop_expr) => match loop_expr {
                Loop::While(w) => {
                    self.check_expr(&w.cond, symptoms, locals);
                    for e in &w.exprs {
                        self.check_expr(e, symptoms, locals);
                    }
                }
                Loop::ForIn(f) => {
                    self.check_expr(&f.iter, symptoms, locals);
                    for e in &f.exprs {
                        self.check_expr(e, symptoms, locals);
                    }
                }
            },
            Expr::TupleLit(elems) => {
                for e in elems {
                    self.check_expr(e, symptoms, locals);
                }
            }
            Expr::TupleAlloc { init, .. } => self.check_expr(init, symptoms, locals),
            Expr::TupleGet { base, .. } => self.check_expr(base, symptoms, locals),
            Expr::TupleSet { base, value, .. } => {
                self.check_expr(base, symptoms, locals);
                self.check_expr(value, symptoms, locals);
            }
            Expr::BufGet { buf, index, .. } => {
                self.check_expr(buf, symptoms, locals);
                self.check_expr(index, symptoms, locals);
            }
            Expr::BufSet {
                buf, index, value, ..
            } => {
                self.check_expr(buf, symptoms, locals);
                self.check_expr(index, symptoms, locals);
                self.check_expr(value, symptoms, locals);
            }
            Expr::Cast { expr, .. } => self.check_expr(expr, symptoms, locals),
            Expr::TakePtr(e) | Expr::TakeRef(e) | Expr::Deref(e) | Expr::Negate(e) => {
                self.check_expr(e, symptoms, locals);
            }
            Expr::AnonymousTag(name, span) => {
                if self.lookup_variant(*name).is_none() {
                    symptoms.push(
                        TypeSymptom::UnknownTag {
                            name: name.to_string(),
                        }
                        .into_diagnostic(*span),
                    );
                }
            }
            Expr::TagCall(tc) => {
                if let Some(path) = &tc.qual_path {
                    if self.lookup_tag(path.root).is_none() {
                        symptoms.push(
                            TypeSymptom::UnknownTag {
                                name: path.root.to_string(),
                            }
                            .into_diagnostic(path.span_id()),
                        );
                    }
                } else if self.lookup_variant(tc.name).is_none() {
                    symptoms.push(
                        TypeSymptom::UnknownTag {
                            name: tc.name.to_string(),
                        }
                        .into_diagnostic(tc.span_id()),
                    );
                }
                for arg in &tc.args {
                    self.check_expr(arg, symptoms, locals);
                }
            }
            Expr::TypeNominal(..) | Expr::TypeQualified(_) | Expr::TypeGeneric { .. } => {
                self.check_type_expr(expr, symptoms);
            }
            Expr::Lit(_)
            | Expr::SelfRef(_)
            | Expr::Range(_)
            | Expr::FormatString(_)
            | Expr::Asm(_) => {}
        }
    }

    fn check_type_expr(&self, e: &Expr, symptoms: &mut Vec<diagnostic::Diagnostic>) {
        use diagnostic::DiagnosticLike;
        use diagnostic::type_::TypeSymptom;

        match e {
            Expr::TypeNominal(name, span) if self.lookup_tag(*name).is_none() => {
                symptoms.push(
                    TypeSymptom::UnknownTag {
                        name: name.to_string(),
                    }
                    .into_diagnostic(*span),
                );
            }
            Expr::TypeGeneric { name, params, span } => {
                if self.lookup_tag(*name).is_none() {
                    symptoms.push(
                        TypeSymptom::UnknownTag {
                            name: name.to_string(),
                        }
                        .into_diagnostic(*span),
                    );
                }
                for (_, kind) in params {
                    if let ParameterKind::Tagged(sp) = kind
                        && is_type_surface(&sp.0)
                    {
                        self.check_type_expr(&sp.0, symptoms);
                    }
                }
            }
            Expr::TypeQualified(path) if self.lookup_tag(path.root).is_none() => {
                symptoms.push(
                    TypeSymptom::UnknownTag {
                        name: path.root.to_string(),
                    }
                    .into_diagnostic(path.span_id()),
                );
            }
            _ => {}
        }
    }
}

fn check_type_pattern_default_exprs(surface: &Expr, check: &mut impl FnMut(&Spanned<Expr>)) {
    let Expr::TypeGeneric { params, .. } = surface else {
        return;
    };
    for (_, pk) in params {
        match pk {
            ParameterKind::Default(e) => check(e),
            ParameterKind::Tagged(sp) => check_type_surface_defaults(&sp.0, check),
            ParameterKind::Generic => {}
        }
    }
}

fn check_type_surface_defaults(e: &Expr, check: &mut impl FnMut(&Spanned<Expr>)) {
    if let Expr::TypeGeneric { params, .. } = e {
        for (_, pk) in params {
            match pk {
                ParameterKind::Default(e) => check(e),
                ParameterKind::Tagged(sp) => check_type_surface_defaults(&sp.0, check),
                ParameterKind::Generic => {}
            }
        }
    }
}

fn type_pattern_references_name(surface: &Expr, name: Intern<String>) -> bool {
    let Expr::TypeGeneric { params, .. } = surface else {
        return false;
    };
    params.iter().any(|(_, pk)| match pk {
        ParameterKind::Default(e) => expr_references_name(&e.0, name),
        ParameterKind::Tagged(sp) => type_surface_references_name(&sp.0, name),
        ParameterKind::Generic => false,
    })
}

fn type_surface_references_name(e: &Expr, name: Intern<String>) -> bool {
    match e {
        Expr::TypeGeneric { params, .. } => params.iter().any(|(_, pk)| match pk {
            ParameterKind::Default(e) => expr_references_name(&e.0, name),
            ParameterKind::Tagged(sp) => type_surface_references_name(&sp.0, name),
            ParameterKind::Generic => false,
        }),
        _ => false,
    }
}

fn expr_references_name(expr: &Expr, name: Intern<String>) -> bool {
    match expr {
        Expr::FnCall(call) => {
            (call.path.root == name && call.path.segments.is_empty())
                || call
                    .args
                    .as_ref()
                    .is_some_and(|args| args.iter().any(|a| expr_references_name(a, name)))
        }
        Expr::Bind(bind) => match bind.value() {
            BindValue::Expr(e) => expr_references_name(e, name),
            BindValue::Body { exprs, ret } => {
                exprs.iter().any(|e| expr_references_name(e, name))
                    || ret
                        .0
                        .as_ref()
                        .is_some_and(|e| expr_references_name(e, name))
            }
            BindValue::Extern => false,
        },
        Expr::Binary(bin) => {
            expr_references_name(&bin.lhs, name) || expr_references_name(&bin.rhs, name)
        }
        Expr::When(w) => {
            w.subject
                .as_ref()
                .is_some_and(|s| expr_references_name(s, name))
                || w.arms.iter().any(|arm| match arm {
                    WhenArm::Cond { condition, body } => {
                        expr_references_name(condition, name) || expr_references_name(body, name)
                    }
                    WhenArm::Is { pattern, body } => {
                        type_pattern_references_name(&pattern.0, name)
                            || expr_references_name(&body.0, name)
                    }
                    WhenArm::Else(body) => expr_references_name(body, name),
                })
        }
        Expr::If(if_expr) => {
            let cond_ref = match &if_expr.condition {
                IfCondition::Bool(c) => expr_references_name(c, name),
                IfCondition::Pattern { subject, pattern } => {
                    expr_references_name(subject, name)
                        || type_pattern_references_name(&pattern.0, name)
                }
            };
            cond_ref || if_expr.body.iter().any(|e| expr_references_name(e, name))
        }
        Expr::Loop(loop_expr) => match loop_expr {
            Loop::While(w) => {
                expr_references_name(&w.cond, name)
                    || w.exprs.iter().any(|e| expr_references_name(e, name))
            }
            Loop::ForIn(f) => {
                expr_references_name(&f.iter, name)
                    || f.exprs.iter().any(|e| expr_references_name(e, name))
            }
        },
        Expr::TupleLit(elems) => elems.iter().any(|e| expr_references_name(e, name)),
        Expr::TupleAlloc { init, .. } => expr_references_name(init, name),
        Expr::TupleGet { base, .. } => expr_references_name(base, name),
        Expr::TupleSet { base, value, .. } => {
            expr_references_name(base, name) || expr_references_name(value, name)
        }
        Expr::BufGet { buf, index, .. } => {
            expr_references_name(buf, name) || expr_references_name(index, name)
        }
        Expr::BufSet {
            buf, index, value, ..
        } => {
            expr_references_name(buf, name)
                || expr_references_name(index, name)
                || expr_references_name(value, name)
        }
        Expr::Cast { expr, .. } => expr_references_name(expr, name),
        Expr::TakePtr(e) | Expr::TakeRef(e) | Expr::Deref(e) | Expr::Negate(e) => {
            expr_references_name(e, name)
        }
        Expr::FormatString(fs) => fs.parts.iter().any(|p| {
            if let FormatPart::Expr(e) = p {
                expr_references_name(e, name)
            } else {
                false
            }
        }),
        Expr::Range(range) => {
            expr_references_name(&range.start, name) || expr_references_name(&range.end, name)
        }
        Expr::TypeGeneric { .. } => type_surface_references_name(expr, name),
        Expr::TypeNominal(..) | Expr::TypeQualified(_) => false,
        Expr::Lit(_)
        | Expr::SelfRef(_)
        | Expr::AnonymousTag(..)
        | Expr::TagCall(_)
        | Expr::Asm(_) => false,
    }
}

fn check_return_variants(
    bind: &Bind,
    valid_variants: &[Intern<String>],
    union_name: Intern<String>,
    symptoms: &mut Vec<diagnostic::Diagnostic>,
) {
    use diagnostic::DiagnosticLike;
    use diagnostic::type_::TypeSymptom;

    fn check_expr(
        expr: &Spanned<Expr>,
        valid_variants: &[Intern<String>],
        union_name: Intern<String>,
        symptoms: &mut Vec<diagnostic::Diagnostic>,
    ) {
        match &expr.0 {
            Expr::AnonymousTag(name, span)
                if !valid_variants.iter().any(|v| v.as_str() == name.as_str()) =>
            {
                symptoms.push(
                    TypeSymptom::NotAVariant {
                        name: name.to_string(),
                        union_name: union_name.to_string(),
                    }
                    .into_diagnostic(*span),
                );
            }
            Expr::TagCall(tc)
                if !valid_variants
                    .iter()
                    .any(|v| v.as_str() == tc.name.as_str()) =>
            {
                symptoms.push(
                    TypeSymptom::NotAVariant {
                        name: tc.name.to_string(),
                        union_name: union_name.to_string(),
                    }
                    .into_diagnostic(tc.span_id()),
                );
            }
            Expr::If(if_expr) => {
                for e in &if_expr.body {
                    check_expr(e, valid_variants, union_name, symptoms);
                }
                if let Some(ret_expr) = &if_expr.ret.0 {
                    check_expr(ret_expr, valid_variants, union_name, symptoms);
                } else {
                    symptoms.push(
                        TypeSymptom::EmptyReturn {
                            expected_type: union_name.to_string(),
                        }
                        .into_diagnostic(expr.1),
                    );
                }
            }
            Expr::When(w) => {
                for arm in &w.arms {
                    match arm {
                        WhenArm::Cond { body, .. } => {
                            check_expr(body, valid_variants, union_name, symptoms)
                        }
                        WhenArm::Is { body, .. } => {
                            check_expr(body, valid_variants, union_name, symptoms)
                        }
                        WhenArm::Else(body) => {
                            check_expr(body, valid_variants, union_name, symptoms)
                        }
                    }
                }
            }
            Expr::Bind(inner) => match inner.value() {
                BindValue::Expr(e) => check_expr(e, valid_variants, union_name, symptoms),
                BindValue::Body { exprs, ret } => {
                    for e in exprs {
                        check_expr(e, valid_variants, union_name, symptoms);
                    }
                    if let Some(r) = &ret.0 {
                        check_expr(r, valid_variants, union_name, symptoms);
                    } else {
                        symptoms.push(
                            TypeSymptom::EmptyReturn {
                                expected_type: union_name.to_string(),
                            }
                            .into_diagnostic(inner.name_span),
                        );
                    }
                }
                BindValue::Extern => {}
            },
            _ => {}
        }
    }

    match bind.value() {
        BindValue::Expr(expr) => check_expr(expr, valid_variants, union_name, symptoms),
        BindValue::Body { exprs, ret } => {
            for expr in exprs {
                check_expr(expr, valid_variants, union_name, symptoms);
            }
            if let Some(ret_expr) = &ret.0 {
                check_expr(ret_expr, valid_variants, union_name, symptoms);
            } else {
                symptoms.push(
                    TypeSymptom::EmptyReturn {
                        expected_type: union_name.to_string(),
                    }
                    .into_diagnostic(bind.name_span),
                );
            }
        }
        BindValue::Extern => {}
    }
}
